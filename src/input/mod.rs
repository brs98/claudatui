//! Input mode handling for vim-like modal interface.
//!
//! This module provides a modal input system with three modes:
//! - **Normal**: Navigation, commands, leader key access
//! - **Insert**: Text input mode (modal dialogs, PTY passthrough)
//! - **Leader**: Command discovery via which-key style popup

pub mod which_key;

use std::time::Instant;

/// Vim-like input modes for the TUI
#[derive(Debug, Clone, Default, PartialEq)]
pub enum InputMode {
    /// Normal mode - navigation, commands, leader key access
    /// This is the default mode for command entry and navigation.
    #[default]
    Normal,
    /// Insert mode - text input active
    /// Active when modals with text fields are open or when typing to PTY in terminal.
    Insert,
    /// Leader mode - shows which-key popup for command discovery
    /// Entered by pressing Space in Normal mode.
    Leader(LeaderState),
}

impl InputMode {
    /// Returns the display name for the status line
    pub fn display_name(&self) -> &'static str {
        match self {
            InputMode::Normal => "NORMAL",
            InputMode::Insert => "INSERT",
            InputMode::Leader(_) => "LEADER",
        }
    }

    /// Returns true if this mode accepts text input
    pub fn is_text_input(&self) -> bool {
        matches!(self, InputMode::Insert)
    }

    /// Returns true if in leader mode
    pub fn is_leader(&self) -> bool {
        matches!(self, InputMode::Leader(_))
    }
}

/// State for leader key mode, tracking the current path through the command tree
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderState {
    /// Keys pressed so far in the leader sequence (e.g., ['x'] for archive submenu)
    pub path: Vec<char>,
    /// When leader mode was started (for timeout)
    pub started_at: Instant,
    /// First key of a potential jk/kj escape sequence (char, when_pressed)
    pub pending_escape: Option<(char, Instant)>,
}

impl LeaderState {
    /// Create a new leader state
    pub fn new() -> Self {
        Self {
            path: Vec::new(),
            started_at: Instant::now(),
            pending_escape: None,
        }
    }

    /// Returns the path as a display string (e.g., "SPC b m")
    pub fn display_path(&self) -> String {
        if self.path.is_empty() {
            "SPC".to_string()
        } else {
            format!(
                "SPC {}",
                self.path
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
    }

    /// Add a key to the path
    pub fn push(&mut self, key: char) {
        self.path.push(key);
    }

    /// Check if leader mode has timed out
    pub fn is_expired(&self, timeout_ms: u64) -> bool {
        self.started_at.elapsed().as_millis() as u64 > timeout_ms
    }
}

impl Default for LeaderState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_returns_correct_string_for_each_mode() {
        assert_eq!(InputMode::Normal.display_name(), "NORMAL");
        assert_eq!(InputMode::Insert.display_name(), "INSERT");
        assert_eq!(
            InputMode::Leader(LeaderState::new()).display_name(),
            "LEADER"
        );
    }

    #[test]
    fn leader_state_display_path_builds_space_separated_keys() {
        let mut state = LeaderState::new();
        assert_eq!(state.display_path(), "SPC");

        state.push('b');
        assert_eq!(state.display_path(), "SPC b");

        state.push('m');
        assert_eq!(state.display_path(), "SPC b m");
    }

    #[test]
    fn is_text_input_returns_true_only_for_insert_mode() {
        assert!(!InputMode::Normal.is_text_input());
        assert!(InputMode::Insert.is_text_input());
        assert!(!InputMode::Leader(LeaderState::new()).is_text_input());
    }
}
