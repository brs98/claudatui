//! Git integration for claudatui.
//!
//! This module provides git worktree operations for creating feature branches.

mod worktree;

pub use worktree::{
    create_worktree, get_repo_from_worktree_path, validate_branch_name, RepoInfo, WorktreeError,
};
