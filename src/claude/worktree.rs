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
            .map(|n| n.to_string_lossy().to_string())
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
        // .git is a file — this is a worktree of a non-bare repo.
        // File contents: "gitdir: /path/to/repo/.git/worktrees/<branch-name>"
        if let Ok(contents) = std::fs::read_to_string(&dot_git) {
            if let Some(gitdir) = contents.trim().strip_prefix("gitdir: ") {
                let gitdir_path = PathBuf::from(gitdir);
                // Navigate from .git/worktrees/<name> back to the repo root
                // gitdir_path = /repo/.git/worktrees/<name>
                // parent three times: worktrees -> .git -> repo root
                if let Some(repo_root) = gitdir_path.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
                    let branch = gitdir_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    return Some(RepoInfo::NormalRepoWorktree {
                        repo_path: repo_root.to_path_buf(),
                        branch,
                    });
                }
            }
        }
    }

    // Check for bare repo worktree pattern: /path/to/repo.git/<branch>
    // The parent should be a bare repo (has HEAD + refs at its root).
    if let Some(parent) = project_path.parent() {
        let parent_str = parent.to_string_lossy();
        if parent_str.ends_with(".git") && parent.join("HEAD").exists() && parent.join("refs").is_dir() {
            let branch = project_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
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
        // Normal repo: place as a sibling directory
        repo_path
            .parent()
            .map(|p| p.join(branch_name))
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
