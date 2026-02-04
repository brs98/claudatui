//! claudatui library crate.
//!
//! This library provides the core functionality for claudatui, including:
//! - Daemon for PTY management (survives TUI restarts)
//! - Claude conversation parsing and grouping
//! - Terminal UI components

pub mod claude;
pub mod daemon;
pub mod daemon_client;
pub mod event;
pub mod pty;
pub mod session;
pub mod ui;
