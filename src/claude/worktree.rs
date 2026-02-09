//! Git worktree detection and creation.
//!
//! Supports both bare repo worktrees (`repo.git/branch`) and
//! non-bare repo worktrees (sibling directories with `.git` file).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

/// Information about a git repository at a given path.
#[derive(Debug, Clone)]
pub enum RepoInfo {
    /// A bare repository root (e.g., `repo.git/`)
    BareRepo { repo_path: PathBuf },
    /// A worktree inside a bare repository (e.g., `repo.git/feature`)
    BareRepoWorktree { repo_path: PathBuf, branch: String },
    /// A normal (non-bare) repository root
    NormalRepo { repo_path: PathBuf },
    /// A worktree created from a normal repository
    NormalRepoWorktree { repo_path: PathBuf, branch: String },
}

impl RepoInfo {
    /// The root repo path (bare dir or normal repo root), used as the source for `git worktree add`.
    pub fn repo_path(&self) -> &Path {
        match self {
            RepoInfo::BareRepo { repo_path }
            | RepoInfo::BareRepoWorktree { repo_path, .. }
            | RepoInfo::NormalRepo { repo_path }
            | RepoInfo::NormalRepoWorktree { repo_path, .. } => repo_path,
        }
    }

    /// A display-friendly name for the repository.
    pub fn display_name(&self) -> String {
        let name = self
            .repo_path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".to_string());
        name.trim_end_matches(".git").to_string()
    }

    /// Whether this is a bare repo (or worktree of one).
    pub fn is_bare(&self) -> bool {
        matches!(
            self,
            RepoInfo::BareRepo { .. } | RepoInfo::BareRepoWorktree { .. }
        )
    }
}

/// Detect the type of git repository at `project_path`.
///
/// Returns `None` if the path is not inside a git repository.
pub fn detect_repo_info(project_path: &Path) -> Option<RepoInfo> {
    let dot_git = project_path.join(".git");

    if dot_git.is_dir() {
        // .git is a directory — either a bare repo or a normal repo root.
        // Check if this is a bare repo by looking for HEAD directly in the project dir
        // (bare repos have HEAD at repo root, not inside .git/).
        let head_at_root = project_path.join("HEAD");
        if head_at_root.exists() && project_path.join("refs").is_dir() {
            return Some(RepoInfo::BareRepo {
                repo_path: project_path.to_path_buf(),
            });
        }
        // Normal repo with .git directory
        return Some(RepoInfo::NormalRepo {
            repo_path: project_path.to_path_buf(),
        });
    }

    if dot_git.is_file() {
        // .git is a file — this is a worktree. Parse the gitdir pointer to find the repo.
        // File contents: "gitdir: /path/to/repo/.git/worktrees/<name>" (normal)
        //             or "gitdir: /path/to/repo.git/worktrees/<name>" (bare)
        if let Ok(contents) = std::fs::read_to_string(&dot_git) {
            if let Some(gitdir) = contents.trim().strip_prefix("gitdir: ") {
                let gitdir_path = PathBuf::from(gitdir);
                // Navigate up 2 levels to the "git container":
                //   normal → /repo/.git
                //   bare   → /repo.git
                let git_container = gitdir_path.parent().and_then(|p| p.parent());

                if let Some(container) = git_container {
                    let branch = gitdir_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();

                    if container.file_name().is_some_and(|n| n == ".git") {
                        // Normal repo: container is .git dir, repo root is its parent
                        if let Some(repo_root) = container.parent() {
                            return Some(RepoInfo::NormalRepoWorktree {
                                repo_path: repo_root.to_path_buf(),
                                branch,
                            });
                        }
                    } else if container.join("HEAD").exists() && container.join("refs").is_dir() {
                        // Bare repo: container IS the repo
                        return Some(RepoInfo::BareRepoWorktree {
                            repo_path: container.to_path_buf(),
                            branch,
                        });
                    }
                }
            }
        }
    }

    // Check for bare repo worktree pattern: /path/to/repo.git/<branch>
    // The parent should be a bare repo (has HEAD + refs at its root).
    if let Some(parent) = project_path.parent() {
        let parent_str = parent.to_string_lossy();
        if parent_str.ends_with(".git")
            && parent.join("HEAD").exists()
            && parent.join("refs").is_dir()
        {
            let branch = project_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            return Some(RepoInfo::BareRepoWorktree {
                repo_path: parent.to_path_buf(),
                branch,
            });
        }
    }

    None
}

/// Create a new git worktree with a new branch.
///
/// For bare repos, the worktree is created inside the bare repo directory.
/// For normal repos, the worktree is created as a sibling directory.
///
/// Returns the path to the new worktree directory.
pub fn create_worktree(repo_info: &RepoInfo, branch_name: &str) -> Result<PathBuf> {
    let repo_path = repo_info.repo_path();

    // Determine where to place the new worktree
    let worktree_path = if repo_info.is_bare() {
        // Bare repo: place inside the bare repo dir
        repo_path.join(branch_name)
    } else {
        // Normal repo: place as a sibling directory, prefixed with repo name
        // e.g., /work/myrepo + branch "feat" → /work/myrepo-feat
        let repo_name = repo_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".to_string());
        repo_path
            .parent()
            .map(|p| p.join(format!("{}-{}", repo_name, branch_name)))
            .context("Cannot determine parent directory for worktree")?
    };

    if worktree_path.exists() {
        anyhow::bail!("Directory already exists: {}", worktree_path.display());
    }

    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            branch_name,
            &worktree_path.to_string_lossy(),
        ])
        .current_dir(repo_path)
        .output()
        .context("Failed to run git worktree add")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git worktree add failed: {}", stderr.trim());
    }

    Ok(worktree_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_bare_repo_worktree_from_dot_git_file() {
        let dir = tempfile::tempdir().unwrap();

        // Create a bare repo structure: repo.git/ with HEAD + refs/
        let bare_repo = dir.path().join("myrepo.git");
        std::fs::create_dir_all(bare_repo.join("refs")).unwrap();
        std::fs::write(bare_repo.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        // Create a worktrees entry inside the bare repo
        let worktree_gitdir = bare_repo.join("worktrees").join("feature");
        std::fs::create_dir_all(&worktree_gitdir).unwrap();

        // Create the worktree directory with a .git file pointing to the bare repo
        let worktree_dir = dir.path().join("feature");
        std::fs::create_dir_all(&worktree_dir).unwrap();
        std::fs::write(
            worktree_dir.join(".git"),
            format!("gitdir: {}", worktree_gitdir.display()),
        )
        .unwrap();

        let info = detect_repo_info(&worktree_dir).expect("should detect repo info");
        match info {
            RepoInfo::BareRepoWorktree { repo_path, branch } => {
                assert_eq!(repo_path, bare_repo);
                assert_eq!(branch, "feature");
            }
            other => panic!("Expected BareRepoWorktree, got {:?}", other),
        }
    }

    #[test]
    fn detect_normal_repo_worktree_from_dot_git_file() {
        let dir = tempfile::tempdir().unwrap();

        // Create a normal repo structure: repo/.git/ with worktrees/
        let repo = dir.path().join("myrepo");
        let git_dir = repo.join(".git");
        let worktree_gitdir = git_dir.join("worktrees").join("feature");
        std::fs::create_dir_all(&worktree_gitdir).unwrap();

        // Create the worktree directory with a .git file
        let worktree_dir = dir.path().join("feature");
        std::fs::create_dir_all(&worktree_dir).unwrap();
        std::fs::write(
            worktree_dir.join(".git"),
            format!("gitdir: {}", worktree_gitdir.display()),
        )
        .unwrap();

        let info = detect_repo_info(&worktree_dir).expect("should detect repo info");
        match info {
            RepoInfo::NormalRepoWorktree { repo_path, branch } => {
                assert_eq!(repo_path, repo);
                assert_eq!(branch, "feature");
            }
            other => panic!("Expected NormalRepoWorktree, got {:?}", other),
        }
    }

    #[test]
    fn detect_bare_repo_root() {
        let dir = tempfile::tempdir().unwrap();

        // Create a bare repo that also has a .git dir (some bare repos do)
        let bare_repo = dir.path().join("myrepo.git");
        std::fs::create_dir_all(bare_repo.join(".git")).unwrap();
        std::fs::create_dir_all(bare_repo.join("refs")).unwrap();
        std::fs::write(bare_repo.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let info = detect_repo_info(&bare_repo).expect("should detect repo info");
        match info {
            RepoInfo::BareRepo { repo_path } => {
                assert_eq!(repo_path, bare_repo);
            }
            other => panic!("Expected BareRepo, got {:?}", other),
        }
    }

    #[test]
    fn detect_normal_repo_root() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("myrepo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();

        let info = detect_repo_info(&repo).expect("should detect repo info");
        match info {
            RepoInfo::NormalRepo { repo_path } => {
                assert_eq!(repo_path, repo);
            }
            other => panic!("Expected NormalRepo, got {:?}", other),
        }
    }

    #[test]
    fn detect_returns_none_for_non_repo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-repo");
        std::fs::create_dir_all(&path).unwrap();

        assert!(detect_repo_info(&path).is_none());
    }
}
