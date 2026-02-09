//! Which-key style command discovery system.
//!
//! This module provides a hierarchical command menu system similar to
//! Emacs which-key or nvim-which-key, showing available commands as
//! the user presses leader key sequences.

/// Actions that can be triggered via the leader key menu
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LeaderAction {
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
    /// Open workspace management modal
    ManageWorkspaces,

    // View
    /// Toggle mosaic view (all active sessions in a grid)
    ToggleMosaic,
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
            // Project submenu
            LeaderCommand::submenu(
                'p',
                "project",
                vec![LeaderCommand::action(
                    'p',
                    "workspaces",
                    LeaderAction::ManageWorkspaces,
                )],
            ),
            // View submenu
            LeaderCommand::submenu(
                'v',
                "view",
                vec![LeaderCommand::action(
                    'm',
                    "mosaic",
                    LeaderAction::ToggleMosaic,
                )],
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
        let Some(commands) = self.commands_at_path(path) else {
            return LeaderKeyResult::Cancel;
        };

        match commands.iter().find(|cmd| cmd.key == key) {
            Some(LeaderCommand {
                action: Some(action),
                ..
            }) => LeaderKeyResult::Execute(*action),
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
    fn commands_at_root_includes_search_and_action_keys() {
        let config = WhichKeyConfig::new();
        let commands = config.commands_at_path(&[]).unwrap();
        assert!(!commands.is_empty());
        assert!(commands.iter().any(|c| c.key == '/'));
        assert!(commands.iter().any(|c| c.key == 'x'));
    }

    #[test]
    fn process_key_returns_submenu_for_group_key() {
        let config = WhichKeyConfig::new();
        assert_eq!(config.process_key(&[], 'x'), LeaderKeyResult::Submenu);
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
        assert_eq!(config.submenu_title(&['x']), "Archive");
    }

    #[test]
    fn project_submenu_is_accessible() {
        let config = WhichKeyConfig::new();
        assert_eq!(config.process_key(&[], 'p'), LeaderKeyResult::Submenu);
        assert_eq!(
            config.process_key(&['p'], 'p'),
            LeaderKeyResult::Execute(LeaderAction::ManageWorkspaces)
        );
        assert_eq!(config.submenu_title(&['p']), "Project");
    }

    #[test]
    fn worktree_submenu_has_no_workspace_key() {
        let config = WhichKeyConfig::new();
        assert_eq!(config.process_key(&['w'], 'p'), LeaderKeyResult::Cancel);
    }
}
