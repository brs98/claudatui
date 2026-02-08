//! claudatui library crate.
//!
//! This library provides the core functionality for claudatui, including:
//! - PTY session management
//! - Claude conversation parsing and grouping
//! - Terminal UI components
//! - Workspace management

pub mod app;
pub mod claude;
pub mod config;
pub mod event_loop;
pub mod handlers;
pub mod input;
pub mod search;
pub mod session;
pub mod ui;
