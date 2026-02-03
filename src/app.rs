use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;

use crate::claude::conversation::{parse_conversation, Conversation, ConversationStatus};
use crate::claude::grouping::{group_conversations, ConversationGroup};
use crate::claude::history::{get_conversation_path, parse_history, HistoryEntry};
use crate::pty::PtyHandler;
use crate::ui::sidebar::{build_sidebar_items, SidebarItem, SidebarState};

/// Which pane is currently focused
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Sidebar,
    Terminal,
}

/// Application state
pub struct App {
    /// Path to ~/.claude
    pub claude_dir: PathBuf,
    /// Conversation groups
    pub groups: Vec<ConversationGroup>,
    /// Sidebar state
    pub sidebar_state: SidebarState,
    /// Current focus
    pub focus: Focus,
    /// PTY handler for embedded terminal
    pub pty: Option<PtyHandler>,
    /// VT100 parser for terminal emulation
    pub vt_parser: vt100::Parser,
    /// Currently selected conversation
    pub selected_conversation: Option<Conversation>,
    /// Should quit
    pub should_quit: bool,
    /// Terminal size
    pub term_size: (u16, u16),
}

impl App {
    /// Create a new application instance
    pub fn new() -> Result<Self> {
        let claude_dir = dirs::home_dir()
            .expect("Could not find home directory")
            .join(".claude");

        let mut app = Self {
            claude_dir,
            groups: Vec::new(),
            sidebar_state: SidebarState::new(),
            focus: Focus::Sidebar,
            pty: None,
            vt_parser: vt100::Parser::new(24, 80, 0),
            selected_conversation: None,
            should_quit: false,
            term_size: (80, 24),
        };

        app.load_conversations()?;

        Ok(app)
    }

    /// Load conversations from history
    pub fn load_conversations(&mut self) -> Result<()> {
        let entries = parse_history(&self.claude_dir)?;
        let conversations = self.entries_to_conversations(entries)?;
        self.groups = group_conversations(conversations);
        Ok(())
    }

    fn entries_to_conversations(&self, entries: Vec<HistoryEntry>) -> Result<Vec<Conversation>> {
        let mut conversations = Vec::new();
        let mut seen_sessions: HashSet<String> = HashSet::new();

        for entry in entries {
            // Skip duplicate sessions (history can have multiple entries)
            if seen_sessions.contains(&entry.session_id) {
                continue;
            }
            seen_sessions.insert(entry.session_id.clone());

            // Use display from history - it's already the first user message
            let display = entry.display.clone();

            // Check conversation file for status
            let conv_path = get_conversation_path(&self.claude_dir, &entry);
            let status = if conv_path.exists() {
                parse_conversation(&conv_path)
                    .map(|(_, s)| s)
                    .unwrap_or(ConversationStatus::Idle)
            } else {
                ConversationStatus::Idle
            };

            conversations.push(Conversation {
                session_id: entry.session_id,
                display,
                timestamp: entry.timestamp,
                project_path: PathBuf::from(&entry.project),
                status,
            });
        }

        Ok(conversations)
    }

    /// Get the flattened sidebar items for navigation
    pub fn sidebar_items(&self) -> Vec<SidebarItem> {
        build_sidebar_items(&self.groups, &self.sidebar_state.collapsed_groups)
    }

    /// Navigate up in the sidebar
    pub fn navigate_up(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let new_idx = if current == 0 {
            items.len() - 1
        } else {
            current - 1
        };
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Navigate down in the sidebar
    pub fn navigate_down(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let new_idx = if current >= items.len() - 1 {
            0
        } else {
            current + 1
        };
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Jump to first item
    pub fn jump_to_first(&mut self) {
        let items = self.sidebar_items();
        if !items.is_empty() {
            self.sidebar_state.list_state.select(Some(0));
            self.update_selected_conversation();
        }
    }

    /// Jump to last item
    pub fn jump_to_last(&mut self) {
        let items = self.sidebar_items();
        if !items.is_empty() {
            self.sidebar_state.list_state.select(Some(items.len() - 1));
            self.update_selected_conversation();
        }
    }

    /// Toggle collapse state of current group
    pub fn toggle_current_group(&mut self) {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        if let Some(item) = items.get(selected) {
            match item {
                SidebarItem::GroupHeader { key, .. } => {
                    self.sidebar_state.toggle_group(key);
                }
                SidebarItem::Conversation { group_key, .. } => {
                    self.sidebar_state.toggle_group(group_key);
                }
            }
        }
    }

    fn update_selected_conversation(&mut self) {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        if let Some(item) = items.get(selected) {
            if let SidebarItem::Conversation { group_key, index } = item {
                // Find the conversation
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            self.selected_conversation = Some(conv.clone());
                            return;
                        }
                    }
                }
            }
        }
        self.selected_conversation = None;
    }

    /// Toggle focus between sidebar and terminal
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Terminal,
            Focus::Terminal => Focus::Sidebar,
        };
    }

    /// Open the selected conversation in the terminal pane
    pub fn open_selected(&mut self) -> Result<()> {
        // Get the current selected item
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        if let Some(SidebarItem::Conversation { group_key, index }) = items.get(selected) {
            // Find the conversation and clone the path
            let mut target: Option<(PathBuf, Conversation)> = None;
            for group in &self.groups {
                if &group.key() == group_key {
                    if let Some(conv) = group.conversations().get(*index) {
                        target = Some((conv.project_path.clone(), conv.clone()));
                        break;
                    }
                }
            }

            // Now spawn the PTY with the cloned path
            if let Some((path, conv)) = target {
                self.spawn_pty(&path)?;
                self.selected_conversation = Some(conv);
                self.focus = Focus::Terminal;
            }
        }

        Ok(())
    }

    /// Spawn a PTY in the given directory
    fn spawn_pty(&mut self, working_dir: &PathBuf) -> Result<()> {
        // Calculate terminal pane size (75% of total width minus borders)
        let cols = (self.term_size.0 * 75 / 100).saturating_sub(2);
        let rows = self.term_size.1.saturating_sub(3); // Account for borders and help bar

        // Reset VT parser for new size
        self.vt_parser = vt100::Parser::new(rows, cols, 0);

        // Spawn new PTY
        self.pty = Some(PtyHandler::spawn(working_dir, rows, cols)?);

        Ok(())
    }

    /// Process PTY output
    pub fn process_pty_output(&mut self) {
        if let Some(ref pty) = self.pty {
            while let Some(data) = pty.try_recv_output() {
                self.vt_parser.process(&data);
            }
        }
    }

    /// Write input to PTY
    pub fn write_to_pty(&mut self, data: &[u8]) -> Result<()> {
        if let Some(ref mut pty) = self.pty {
            pty.write(data)?;
        }
        Ok(())
    }

    /// Resize terminal
    pub fn resize(&mut self, width: u16, height: u16) -> Result<()> {
        self.term_size = (width, height);

        let cols = (width * 75 / 100).saturating_sub(2);
        let rows = height.saturating_sub(3);

        self.vt_parser = vt100::Parser::new(rows, cols, 0);

        if let Some(ref pty) = self.pty {
            pty.resize(rows, cols)?;
        }

        Ok(())
    }
}
