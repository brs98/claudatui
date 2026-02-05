//! Modal dialog components for the TUI.

pub mod new_project;
pub mod search;

pub use new_project::{NewProjectModal, NewProjectModalState, NewProjectTab};
pub use search::{SearchKeyResult, SearchModal, SearchModalState};
