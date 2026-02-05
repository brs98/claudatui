//! Git worktree operations.

use std::path::{Path, PathBuf};

use git2::{BranchType, Repository};

/// Information about a git repository.
#[derive(Debug, Clone)]
pub struct RepoInfo {
    /// The root path of the bare repo (e.g., /path/to/repo.git)
    pub repo_root: PathBuf,
    /// The name of the current branch
    pub current_branch: String,
}

/// Errors that can occur during worktree operations.
#[derive(Debug)]
pub enum WorktreeError {
    /// The path is not inside a git repository
    NotARepo,
    /// A branch with this name already exists
    BranchExists(String),
    /// A path for this worktree already exists
    PathExists(PathBuf),
    /// The branch name is invalid
    InvalidBranchName(String),
    /// An error from the git2 library
    GitError(git2::Error),
}

impl std::fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorktreeError::NotARepo => write!(f, "Not a git repository"),
            WorktreeError::BranchExists(name) => write!(f, "Branch '{}' already exists", name),
            WorktreeError::PathExists(path) => {
                write!(f, "Path already exists: {}", path.display())
            }
            WorktreeError::InvalidBranchName(reason) => {
                write!(f, "Invalid branch name: {}", reason)
            }
            WorktreeError::GitError(e) => write!(f, "Git error: {}", e),
        }
    }
}

impl std::error::Error for WorktreeError {}

impl From<git2::Error> for WorktreeError {
    fn from(err: git2::Error) -> Self {
        WorktreeError::GitError(err)
    }
}

/// Validate a branch name according to git rules.
///
/// Returns Ok(()) if valid, or Err with a description of the problem.
pub fn validate_branch_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Branch name cannot be empty".to_string());
    }

    // Check for forbidden characters and patterns
    if name.starts_with('-') {
        return Err("Cannot start with '-'".to_string());
    }
    if name.starts_with('.') {
        return Err("Cannot start with '.'".to_string());
    }
    if name.ends_with('.') {
        return Err("Cannot end with '.'".to_string());
    }
    if name.ends_with('/') {
        return Err("Cannot end with '/'".to_string());
    }
    if name.contains("..") {
        return Err("Cannot contain '..'".to_string());
    }
    if name.contains("//") {
        return Err("Cannot contain '//'".to_string());
    }
    if name.contains(' ') {
        return Err("Cannot contain spaces".to_string());
    }
    if name.contains('~') || name.contains('^') || name.contains(':') || name.contains('\\') {
        return Err("Cannot contain ~, ^, :, or \\".to_string());
    }
    if name.contains('\x7f') || name.chars().any(|c| c.is_control()) {
        return Err("Cannot contain control characters".to_string());
    }
    if name.contains("@{") {
        return Err("Cannot contain '@{'".to_string());
    }
    if name == "@" {
        return Err("Cannot be '@'".to_string());
    }
    if name.ends_with(".lock") {
        return Err("Cannot end with '.lock'".to_string());
    }

    Ok(())
}

/// Open a git repository from any path inside a worktree.
///
/// This handles both regular repositories and bare repositories with worktrees.
/// Returns the repository handle and information about the repo.
pub fn get_repo_from_worktree_path(path: &Path) -> Result<(Repository, RepoInfo), WorktreeError> {
    let repo = Repository::discover(path)?;

    // Get the repository root (the .git directory or bare repo path)
    // For worktrees, we need to find the main repository
    let repo_root = if repo.is_bare() {
        repo.path().to_path_buf()
    } else if repo.is_worktree() {
        // This is a worktree - the .git file points to the main repo's worktrees directory
        // We need to find the parent bare repo
        // The path() returns something like /path/to/repo.git/worktrees/branch-name/
        // We want /path/to/repo.git/
        let git_path = repo.path();
        if let Some(worktrees_dir) = git_path.parent() {
            if worktrees_dir.file_name().map(|n| n == "worktrees").unwrap_or(false) {
                if let Some(main_repo) = worktrees_dir.parent() {
                    main_repo.to_path_buf()
                } else {
                    git_path.to_path_buf()
                }
            } else {
                git_path.to_path_buf()
            }
        } else {
            git_path.to_path_buf()
        }
    } else {
        // Regular repo - check if workdir parent has a .git directory (bare repo worktree setup)
        if let Some(workdir) = repo.workdir() {
            if let Some(parent) = workdir.parent() {
                // Check if parent is a .git directory (bare repo worktree setup)
                if parent.extension().map(|e| e == "git").unwrap_or(false) {
                    parent.to_path_buf()
                } else {
                    repo.path().to_path_buf()
                }
            } else {
                repo.path().to_path_buf()
            }
        } else {
            repo.path().to_path_buf()
        }
    };

    // Get the current branch name
    let current_branch = {
        let head = repo.head()?;
        head.shorthand().unwrap_or("HEAD").to_string()
    };

    Ok((
        repo,
        RepoInfo {
            repo_root,
            current_branch,
        },
    ))
}

/// Create a new git worktree with a new branch.
///
/// The worktree will be created as a sibling directory to the base_path.
/// For example, if base_path is `/repo.git/main`, the new worktree will be
/// created at `/repo.git/feature-branch`.
///
/// # Arguments
/// * `repo` - The repository to create the worktree in
/// * `branch_name` - The name for the new branch
/// * `base_path` - The path of the worktree we're branching from
///
/// # Returns
/// The path to the newly created worktree.
pub fn create_worktree(
    repo: &Repository,
    branch_name: &str,
    base_path: &Path,
) -> Result<PathBuf, WorktreeError> {
    // Validate the branch name
    if let Err(reason) = validate_branch_name(branch_name) {
        return Err(WorktreeError::InvalidBranchName(reason));
    }

    // Check if branch already exists
    if repo.find_branch(branch_name, BranchType::Local).is_ok() {
        return Err(WorktreeError::BranchExists(branch_name.to_string()));
    }

    // Calculate the worktree path (sibling to base_path)
    let worktree_path = base_path
        .parent()
        .ok_or_else(|| WorktreeError::InvalidBranchName("Cannot determine parent directory".to_string()))?
        .join(branch_name);

    // Check if path already exists
    if worktree_path.exists() {
        return Err(WorktreeError::PathExists(worktree_path));
    }

    // Get the current HEAD commit to base the new branch on
    let head = repo.head()?;
    let head_commit = head.peel_to_commit()?;

    // Create the new branch pointing to the current HEAD
    let branch = repo.branch(branch_name, &head_commit, false)?;
    let branch_ref = branch.into_reference();
    let branch_ref_name = branch_ref.name()
        .ok_or_else(|| WorktreeError::InvalidBranchName("Invalid branch reference".to_string()))?;

    // Create the worktree
    repo.worktree(
        branch_name,
        &worktree_path,
        Some(
            git2::WorktreeAddOptions::new()
                .reference(Some(&branch_ref))
        ),
    )?;

    // The worktree is created but we need to check it out to the branch
    // Open the worktree's repository and checkout the branch
    let worktree_repo = Repository::open(&worktree_path)?;
    let obj = worktree_repo.revparse_single(branch_ref_name)?;
    worktree_repo.checkout_tree(&obj, None)?;
    worktree_repo.set_head(branch_ref_name)?;

    Ok(worktree_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_branch_name_valid() {
        assert!(validate_branch_name("feature-branch").is_ok());
        assert!(validate_branch_name("feature/add-login").is_ok());
        assert!(validate_branch_name("fix-123").is_ok());
        assert!(validate_branch_name("my_branch").is_ok());
    }

    #[test]
    fn test_validate_branch_name_invalid() {
        assert!(validate_branch_name("").is_err());
        assert!(validate_branch_name("-starts-with-dash").is_err());
        assert!(validate_branch_name(".starts-with-dot").is_err());
        assert!(validate_branch_name("ends-with-dot.").is_err());
        assert!(validate_branch_name("ends-with-slash/").is_err());
        assert!(validate_branch_name("has..double-dots").is_err());
        assert!(validate_branch_name("has//double-slash").is_err());
        assert!(validate_branch_name("has space").is_err());
        assert!(validate_branch_name("has~tilde").is_err());
        assert!(validate_branch_name("has^caret").is_err());
        assert!(validate_branch_name("has:colon").is_err());
        assert!(validate_branch_name("has\\backslash").is_err());
        assert!(validate_branch_name("has@{at-brace").is_err());
        assert!(validate_branch_name("@").is_err());
        assert!(validate_branch_name("branch.lock").is_err());
    }
}
