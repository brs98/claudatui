//! Claude conversation data: parsing, grouping, archiving, and file watching.

pub mod archive;
pub mod conversation;
pub mod grouping;
pub mod sessions;
pub mod watcher;
pub mod worktree;

pub use archive::ArchiveManager;
pub use watcher::SessionsWatcher;
