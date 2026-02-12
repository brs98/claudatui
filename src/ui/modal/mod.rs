//! Modal dialog components for the TUI.

use std::path::PathBuf;

use crossterm::event::KeyEvent;

pub mod new_project;
pub mod profile;
pub mod search;
pub mod workspace;
pub mod worktree;
pub mod worktree_search;

pub use new_project::{NewProjectModal, NewProjectModalState, NewProjectTab};
pub use profile::{ProfileModal, ProfileModalState};
pub use search::{SearchKeyResult, SearchModal, SearchModalState};
pub use workspace::{WorkspaceModal, WorkspaceModalState};
pub use worktree::{WorktreeModal, WorktreeModalState};
pub use worktree_search::{
    WorktreeProject, WorktreeSearchKeyResult, WorktreeSearchModal, WorktreeSearchModalState,
};

/// Unified result type for modal key handling dispatch.
pub enum ModalKeyResult {
    /// Nothing happened, continue.
    Continue,
    /// Modal wants to close.
    Close,
    /// A path was selected (NewProject modal).
    PathSelected(PathBuf),
    /// A search result was selected (Search modal).
    SearchSelected(String),
    /// Search query changed (Search modal).
    SearchQueryChanged,
    /// A branch name was entered (Worktree modal).
    BranchSelected(String),
    /// Worktree search confirmed (WorktreeSearch modal).
    WorktreeSearchConfirmed {
        project_path: PathBuf,
        branch_name: String,
    },
    /// Query changed in worktree search.
    WorktreeSearchQueryChanged,
    /// A workspace directory was added.
    WorkspaceAdded(String),
    /// A workspace directory was removed (by index).
    WorkspaceRemoved(usize),
    /// A new profile was created.
    ProfileCreated(String),
    /// A profile was renamed.
    ProfileRenamed { index: usize, new_name: String },
    /// A profile was deleted (by index).
    ProfileDeleted(usize),
    /// A profile was activated (by index).
    ProfileActivated(usize),
}

/// Trait for unified modal key dispatch.
pub trait Modal {
    fn handle_key_modal(&mut self, key: KeyEvent) -> ModalKeyResult;
}
