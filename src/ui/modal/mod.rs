//! Modal dialog components for the TUI.

pub mod new_project;
pub mod search;
pub mod worktree;
pub mod worktree_search;

pub use new_project::{NewProjectModal, NewProjectModalState, NewProjectTab};
pub use search::{SearchKeyResult, SearchModal, SearchModalState};
pub use worktree::{WorktreeModal, WorktreeModalState};
pub use worktree_search::{
    WorktreeProject, WorktreeSearchKeyResult, WorktreeSearchModal, WorktreeSearchModalState,
};
