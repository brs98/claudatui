//! Which-key style command discovery system.
//!
//! This module provides a hierarchical command menu system similar to
//! Emacs which-key or nvim-which-key, showing available commands as
//! the user presses leader key sequences.

/// Actions that can be triggered via the leader key menu
#[derive(Debug, Clone, PartialEq)]
pub enum LeaderAction {
    // Bookmark actions
    /// Jump to bookmark slot (1-9)
    BookmarkJump(u8),
    /// Set bookmark at slot (1-9)
    BookmarkSet(u8),
    /// Delete bookmark at slot (1-9)
    BookmarkDelete(u8),

    // Navigation/Search
    /// Open search modal
    SearchOpen,
    /// Open new project modal
    NewProject,

    // Session actions
    /// Close current session (dd equivalent)
    CloseSession,

    // Archive actions
    /// Archive current conversation
    Archive,
    /// Unarchive current conversation
    Unarchive,
    /// Cycle archive filter
    CycleArchiveFilter,

    // Other actions
    /// Refresh sessions list
    Refresh,
    /// Yank (copy) path to clipboard
    YankPath,
    /// Toggle dangerous mode
    ToggleDangerous,

    // Navigation
    /// Add new conversation in selected group
    AddConversation,

    // Worktree
    /// Create a new git worktree from selected group
    CreateWorktree,
    /// Open worktree search modal (pick any project)
    WorktreeSearch,
}

/// A command entry in the which-key menu
#[derive(Debug, Clone)]
pub struct LeaderCommand {
    /// The key that triggers this command
    pub key: char,
    /// Display label for the key
    pub label: String,
    /// The action to execute, or None if this opens a submenu
    pub action: Option<LeaderAction>,
    /// Subcommands if this is a menu node
    pub subcommands: Vec<LeaderCommand>,
}

impl LeaderCommand {
    /// Create a new command that executes an action
    pub fn action(key: char, label: impl Into<String>, action: LeaderAction) -> Self {
        Self {
            key,
            label: label.into(),
            action: Some(action),
            subcommands: Vec::new(),
        }
    }

    /// Create a new submenu
    pub fn submenu(key: char, label: impl Into<String>, subcommands: Vec<LeaderCommand>) -> Self {
        Self {
            key,
            label: label.into(),
            action: None,
            subcommands,
        }
    }

    /// Check if this is a submenu (has subcommands)
    pub fn is_submenu(&self) -> bool {
        !self.subcommands.is_empty()
    }
}

/// Configuration for the which-key system
#[derive(Debug, Clone)]
pub struct WhichKeyConfig {
    /// Root commands available from the leader key
    pub commands: Vec<LeaderCommand>,
    /// Timeout in milliseconds before leader mode auto-cancels
    pub timeout_ms: u64,
}

impl Default for WhichKeyConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl WhichKeyConfig {
    /// Create a new which-key config with the default command tree
    pub fn new() -> Self {
        Self {
            commands: Self::default_commands(),
            timeout_ms: 2000, // 2 second timeout
        }
    }

    /// Build the default command tree
    fn default_commands() -> Vec<LeaderCommand> {
        vec![
            // Bookmarks submenu
            LeaderCommand::submenu(
                'b',
                "bookmarks",
                vec![
                    // Jump to bookmark slots 1-9
                    LeaderCommand::action('1', "slot 1", LeaderAction::BookmarkJump(1)),
                    LeaderCommand::action('2', "slot 2", LeaderAction::BookmarkJump(2)),
                    LeaderCommand::action('3', "slot 3", LeaderAction::BookmarkJump(3)),
                    LeaderCommand::action('4', "slot 4", LeaderAction::BookmarkJump(4)),
                    LeaderCommand::action('5', "slot 5", LeaderAction::BookmarkJump(5)),
                    LeaderCommand::action('6', "slot 6", LeaderAction::BookmarkJump(6)),
                    LeaderCommand::action('7', "slot 7", LeaderAction::BookmarkJump(7)),
                    LeaderCommand::action('8', "slot 8", LeaderAction::BookmarkJump(8)),
                    LeaderCommand::action('9', "slot 9", LeaderAction::BookmarkJump(9)),
                    // Mark (set bookmark) submenu
                    LeaderCommand::submenu(
                        'm',
                        "mark",
                        (1..=9)
                            .map(|i| {
                                LeaderCommand::action(
                                    char::from_digit(i, 10).expect("digit index is in range 1..=9"),
                                    format!("slot {}", i),
                                    LeaderAction::BookmarkSet(i as u8),
                                )
                            })
                            .collect(),
                    ),
                    // Delete bookmark submenu
                    LeaderCommand::submenu(
                        'd',
                        "delete",
                        (1..=9)
                            .map(|i| {
                                LeaderCommand::action(
                                    char::from_digit(i, 10).expect("digit index is in range 1..=9"),
                                    format!("slot {}", i),
                                    LeaderAction::BookmarkDelete(i as u8),
                                )
                            })
                            .collect(),
                    ),
                ],
            ),
            // Direct actions
            LeaderCommand::action('/', "search", LeaderAction::SearchOpen),
            LeaderCommand::action('n', "new project", LeaderAction::NewProject),
            LeaderCommand::action('c', "close session", LeaderAction::CloseSession),
            LeaderCommand::action('a', "add conversation", LeaderAction::AddConversation),
            // Archive submenu
            LeaderCommand::submenu(
                'x',
                "archive",
                vec![
                    LeaderCommand::action('a', "archive", LeaderAction::Archive),
                    LeaderCommand::action('u', "unarchive", LeaderAction::Unarchive),
                    LeaderCommand::action('f', "cycle filter", LeaderAction::CycleArchiveFilter),
                ],
            ),
            // Worktree submenu
            LeaderCommand::submenu(
                'w',
                "worktree",
                vec![
                    LeaderCommand::action('w', "from group", LeaderAction::CreateWorktree),
                    LeaderCommand::action('s', "search", LeaderAction::WorktreeSearch),
                ],
            ),
            // Other actions
            LeaderCommand::action('r', "refresh", LeaderAction::Refresh),
            LeaderCommand::action('y', "yank path", LeaderAction::YankPath),
            LeaderCommand::action('D', "dangerous mode", LeaderAction::ToggleDangerous),
        ]
    }

    /// Find commands at the given path
    /// Returns the commands available at that level, or None if path is invalid
    pub fn commands_at_path(&self, path: &[char]) -> Option<&[LeaderCommand]> {
        if path.is_empty() {
            return Some(&self.commands);
        }

        let mut current = &self.commands;
        for &key in path {
            let found = current.iter().find(|cmd| cmd.key == key)?;
            if found.subcommands.is_empty() {
                return None; // Hit a leaf node before end of path
            }
            current = &found.subcommands;
        }
        Some(current)
    }

    /// Process a key press in leader mode
    /// Returns Some(action) if an action should be executed,
    /// None if navigating to a submenu or invalid key
    pub fn process_key(&self, path: &[char], key: char) -> LeaderKeyResult {
        let commands = match self.commands_at_path(path) {
            Some(cmds) => cmds,
            None => return LeaderKeyResult::Cancel,
        };

        match commands.iter().find(|cmd| cmd.key == key) {
            Some(LeaderCommand {
                action: Some(action),
                ..
            }) => LeaderKeyResult::Execute(action.clone()),
            Some(cmd) if !cmd.subcommands.is_empty() => LeaderKeyResult::Submenu,
            _ => LeaderKeyResult::Cancel, // Invalid key
        }
    }

    /// Get the title for a submenu at the given path
    pub fn submenu_title(&self, path: &[char]) -> String {
        if path.is_empty() {
            return "Leader".to_string();
        }

        let mut current = &self.commands;
        let mut title = String::new();
        for &key in path {
            if let Some(cmd) = current.iter().find(|c| c.key == key) {
                title = cmd.label.clone();
                current = &cmd.subcommands;
            }
        }
        // Capitalize first letter
        let mut chars = title.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            None => title,
        }
    }
}

/// Result of processing a key in leader mode
#[derive(Debug, Clone, PartialEq)]
pub enum LeaderKeyResult {
    /// Execute an action and return to normal mode
    Execute(LeaderAction),
    /// Navigate to a submenu
    Submenu,
    /// Cancel leader mode (invalid key or escape)
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_at_root_includes_bookmark_and_search_keys() {
        let config = WhichKeyConfig::new();
        let commands = config.commands_at_path(&[]).unwrap();
        assert!(!commands.is_empty());
        assert!(commands.iter().any(|c| c.key == 'b'));
        assert!(commands.iter().any(|c| c.key == '/'));
    }

    #[test]
    fn commands_at_bookmark_path_includes_slot_and_mark_keys() {
        let config = WhichKeyConfig::new();
        let commands = config.commands_at_path(&['b']).unwrap();
        assert!(commands.iter().any(|c| c.key == '1'));
        assert!(commands.iter().any(|c| c.key == 'm'));
    }

    #[test]
    fn process_key_executes_bookmark_jump_for_slot_key() {
        let config = WhichKeyConfig::new();
        match config.process_key(&['b'], '1') {
            LeaderKeyResult::Execute(LeaderAction::BookmarkJump(1)) => {}
            other => panic!("Expected BookmarkJump(1), got {:?}", other),
        }
    }

    #[test]
    fn process_key_returns_submenu_for_group_key() {
        let config = WhichKeyConfig::new();
        assert_eq!(config.process_key(&[], 'b'), LeaderKeyResult::Submenu);
    }

    #[test]
    fn process_key_cancels_for_unbound_key() {
        let config = WhichKeyConfig::new();
        assert_eq!(config.process_key(&[], 'z'), LeaderKeyResult::Cancel);
    }

    #[test]
    fn submenu_title_returns_correct_label_for_each_path() {
        let config = WhichKeyConfig::new();
        assert_eq!(config.submenu_title(&[]), "Leader");
        assert_eq!(config.submenu_title(&['b']), "Bookmarks");
    }
}
