//! Daemon module for claudatui.
//!
//! The daemon owns PTY processes and terminal state, allowing the TUI
//! to restart (for hot reload) without losing sessions.

pub mod protocol;
pub mod session_manager;

// Re-exports for daemon binary
#[allow(unused_imports)]
pub use protocol::{Request, Response, SessionId, SessionInfo, SessionState};
#[allow(unused_imports)]
pub use session_manager::{ManagedSession, SessionManager};
