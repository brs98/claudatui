//! claudatui library crate.
//!
//! This library provides the core functionality for claudatui, including:
//! - PTY session management
//! - Claude conversation parsing and grouping
//! - Terminal UI components
//! - Bookmark management

pub mod app;
pub mod bookmarks;
pub mod claude;
pub mod config;
pub mod input;
pub mod pty;
pub mod search;
pub mod session;
pub mod ui;
