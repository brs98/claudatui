//! claudatui library crate.
//!
//! This library provides the core functionality for claudatui, including:
//! - PTY session management
//! - Claude conversation parsing and grouping
//! - Terminal UI components

pub mod app;
pub mod claude;
pub mod event;
#[cfg(feature = "git")]
pub mod git;
pub mod pty;
pub mod session;
pub mod ui;
