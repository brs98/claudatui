//! Modal dialog components for the TUI.

#[cfg(feature = "git")]
pub mod new_branch;
pub mod new_project;

#[cfg(feature = "git")]
pub use new_branch::{NewBranchModal, NewBranchModalState};
pub use new_project::{NewProjectModal, NewProjectModalState, NewProjectTab};
