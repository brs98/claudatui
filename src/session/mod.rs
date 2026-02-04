//! Session management for Claude Code PTY sessions.
//!
//! This module provides:
//! - `SessionManager` - Manages multiple PTY sessions
//! - `ManagedSession` - A single PTY session with terminal emulation
//! - Terminal state types for rendering

pub mod manager;
pub mod types;

pub use manager::SessionManager;
pub use types::{CellAttrs, ScreenState, SessionState, TermColor};
