use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;

use crate::claude::conversation::{detect_status_fast, Conversation, ConversationStatus};
use crate::claude::grouping::{group_conversations, ConversationGroup};
use crate::claude::sessions::{parse_all_sessions, SessionEntry};
use crate::pty::PtyHandler;
use crate::session::RunningSession;
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
    /// All running sessions indexed by session_id
    pub running_sessions: HashMap<String, RunningSession>,
    /// Currently active session (receives input, is displayed)
    pub active_session_id: Option<String>,
    /// Currently selected conversation
    pub selected_conversation: Option<Conversation>,
    /// Should quit
    pub should_quit: bool,
    /// Terminal size
    pub term_size: (u16, u16),
    /// Counter for generating temp session IDs for new conversations
    new_session_counter: usize,
    /// Running sessions that haven't been saved yet (temp IDs)
    /// Maps temp session_id -> project path
    pub ephemeral_sessions: HashMap<String, PathBuf>,
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
            running_sessions: HashMap::new(),
            active_session_id: None,
            selected_conversation: None,
            should_quit: false,
            term_size: (80, 24),
            new_session_counter: 0,
            ephemeral_sessions: HashMap::new(),
        };

        app.load_conversations()?;

        Ok(app)
    }

    /// Load conversations from sessions-index.json files
    pub fn load_conversations(&mut self) -> Result<()> {
        let sessions = parse_all_sessions(&self.claude_dir)?;
        let conversations = self.sessions_to_conversations(sessions);
        self.groups = group_conversations(conversations);
        Ok(())
    }

    /// Convert SessionEntry list to Conversation list
    fn sessions_to_conversations(&self, sessions: Vec<SessionEntry>) -> Vec<Conversation> {
        sessions
            .into_iter()
            .map(|session| {
                // Check conversation file for status using the full path
                let conv_path = PathBuf::from(&session.full_path);
                let status = if conv_path.exists() {
                    detect_status_fast(&conv_path).unwrap_or(ConversationStatus::Idle)
                } else {
                    ConversationStatus::Idle
                };

                Conversation {
                    session_id: session.session_id,
                    display: session.first_prompt,
                    summary: session.summary,
                    timestamp: session.file_mtime,
                    modified: session.modified,
                    project_path: PathBuf::from(&session.project_path),
                    status,
                    message_count: session.message_count,
                    git_branch: session.git_branch,
                }
            })
            .collect()
    }

    /// Get the flattened sidebar items for navigation
    pub fn sidebar_items(&self) -> Vec<SidebarItem> {
        build_sidebar_items(
            &self.groups,
            &self.sidebar_state.collapsed_groups,
            self.sidebar_state.show_all_projects,
            &self.ephemeral_sessions,
        )
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
                SidebarItem::Conversation { group_key, .. }
                | SidebarItem::EphemeralSession { group_key, .. } => {
                    self.sidebar_state.toggle_group(group_key);
                }
                SidebarItem::ShowMoreProjects { .. } => {
                    self.sidebar_state.toggle_show_all_projects();
                }
            }
        }
    }

    fn update_selected_conversation(&mut self) {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        if let Some(item) = items.get(selected) {
            match item {
                SidebarItem::Conversation { group_key, index } => {
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
                SidebarItem::GroupHeader { .. }
                | SidebarItem::EphemeralSession { .. }
                | SidebarItem::ShowMoreProjects { .. } => {
                    // No conversation selected for headers, ephemeral sessions, or "show more" items
                }
            }
        }
        self.selected_conversation = None;
    }

    /// Set focus directly to a specific pane
    pub fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
    }

    /// Open the selected item in the terminal pane
    ///
    /// - For conversations: resumes the conversation with `claude --resume <session_id>`
    ///   If the session is already running, just switches to it.
    /// - For group headers: starts a new conversation in the group's project directory
    /// - For ephemeral sessions: switches to the already-running session
    pub fn open_selected(&mut self) -> Result<()> {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                // Resume existing conversation with session_id
                let mut target: Option<(PathBuf, String, Conversation)> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            target = Some((
                                conv.project_path.clone(),
                                conv.session_id.clone(),
                                conv.clone(),
                            ));
                            break;
                        }
                    }
                }

                if let Some((path, session_id, conv)) = target {
                    // If session is already running, just switch to it
                    if self.running_sessions.contains_key(&session_id) {
                        self.active_session_id = Some(session_id);
                    } else {
                        // Start new session
                        self.start_session(&path, Some(&session_id))?;
                    }
                    self.selected_conversation = Some(conv);
                    self.focus = Focus::Terminal;
                }
            }
            Some(SidebarItem::GroupHeader { key, .. }) => {
                // Start new conversation in the group's project directory
                let mut project_path: Option<PathBuf> = None;
                for group in &self.groups {
                    if &group.key() == key {
                        project_path = group.project_path();
                        break;
                    }
                }

                if let Some(path) = project_path {
                    self.start_session(&path, None)?;
                    self.selected_conversation = None;
                    self.focus = Focus::Terminal;
                }
            }
            Some(SidebarItem::EphemeralSession { session_id, .. }) => {
                // Switch to the already-running ephemeral session
                self.active_session_id = Some(session_id.clone());
                self.selected_conversation = None;
                self.focus = Focus::Terminal;
            }
            Some(SidebarItem::ShowMoreProjects { .. }) => {
                // Toggle showing all projects
                self.sidebar_state.toggle_show_all_projects();
            }
            None => {}
        }

        Ok(())
    }

    /// Start a new session (or resume one) in the given directory.
    ///
    /// If `session_id` is provided, resumes an existing conversation.
    /// Otherwise starts a new conversation with a temp ID.
    fn start_session(&mut self, working_dir: &std::path::Path, session_id: Option<&str>) -> Result<()> {
        // Calculate terminal pane size (75% of total width minus borders)
        let cols = (self.term_size.0 * 75 / 100).saturating_sub(2);
        let rows = self.term_size.1.saturating_sub(3); // Account for borders and help bar

        // Spawn new PTY
        let pty = PtyHandler::spawn(working_dir, rows, cols, session_id)?;

        // Determine session ID - use provided one or generate temp ID for new sessions
        let is_ephemeral = session_id.is_none();
        let sid = session_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                self.new_session_counter += 1;
                format!("__new_session_{}", self.new_session_counter)
            });

        // Create running session with its own vt_parser
        let session = RunningSession::new(sid.clone(), working_dir.to_path_buf(), pty, rows, cols);

        // Add to running sessions and set as active
        self.running_sessions.insert(sid.clone(), session);
        self.active_session_id = Some(sid.clone());

        // Track ephemeral sessions (new sessions without a saved conversation file)
        if is_ephemeral {
            self.ephemeral_sessions.insert(sid.clone(), working_dir.to_path_buf());
        }

        // Set conversation status to Active
        if let Some(ref mut conv) = self.selected_conversation {
            conv.status = ConversationStatus::Active;
            let session_id = conv.session_id.clone();
            self.update_conversation_status_in_groups(&session_id, ConversationStatus::Active);
        }

        Ok(())
    }

    /// Get reference to the active session
    pub fn get_active_session(&self) -> Option<&RunningSession> {
        self.active_session_id
            .as_ref()
            .and_then(|id| self.running_sessions.get(id))
    }

    /// Get mutable reference to the active session
    pub fn get_active_session_mut(&mut self) -> Option<&mut RunningSession> {
        match &self.active_session_id {
            Some(id) => self.running_sessions.get_mut(id),
            None => None,
        }
    }

    /// Get set of running session IDs for sidebar display
    pub fn running_session_ids(&self) -> HashSet<String> {
        self.running_sessions.keys().cloned().collect()
    }

    /// Process output from ALL running sessions
    pub fn process_all_sessions(&mut self) {
        for session in self.running_sessions.values_mut() {
            session.process_output();
        }
    }

    /// Scroll up by the specified number of lines (active session only)
    pub fn scroll_up(&mut self, lines: usize) {
        if let Some(session) = self.get_active_session_mut() {
            session.scroll_up(lines);
        }
    }

    /// Scroll down by the specified number of lines (active session only)
    pub fn scroll_down(&mut self, lines: usize) {
        if let Some(session) = self.get_active_session_mut() {
            session.scroll_down(lines);
        }
    }

    /// Jump to the bottom (live view) for active session
    pub fn scroll_to_bottom(&mut self) {
        if let Some(session) = self.get_active_session_mut() {
            session.scroll_to_bottom();
        }
    }

    /// Check if active session is scroll locked
    pub fn is_scroll_locked(&self) -> bool {
        self.get_active_session()
            .map(|s| s.scroll_locked)
            .unwrap_or(false)
    }

    /// Write input to active session's PTY
    pub fn write_to_pty(&mut self, data: &[u8]) -> Result<()> {
        if let Some(session) = self.get_active_session_mut() {
            session.pty.write(data)?;
        }
        Ok(())
    }

    /// Resize all running sessions
    pub fn resize(&mut self, width: u16, height: u16) -> Result<()> {
        self.term_size = (width, height);

        let cols = (width * 75 / 100).saturating_sub(2);
        let rows = height.saturating_sub(3);

        for session in self.running_sessions.values_mut() {
            session.resize(rows, cols)?;
        }

        Ok(())
    }

    /// Update the status of a conversation in the groups vector
    fn update_conversation_status_in_groups(&mut self, session_id: &str, status: ConversationStatus) {
        for group in &mut self.groups {
            for conv in group.conversations_mut() {
                if conv.session_id == session_id {
                    conv.status = status;
                    return;
                }
            }
        }
    }

    /// Check all sessions for dead PTYs and clean up
    pub fn check_all_session_status(&mut self) {
        // Find dead sessions
        let dead_sessions: Vec<String> = self
            .running_sessions
            .iter()
            .filter(|(_, session)| !session.is_alive())
            .map(|(id, _)| id.clone())
            .collect();

        // Remove dead sessions and update their status
        for session_id in dead_sessions {
            self.running_sessions.remove(&session_id);

            // Remove from ephemeral sessions if present
            self.ephemeral_sessions.remove(&session_id);

            // Re-read conversation status from file
            self.refresh_session_status(&session_id);

            // Clear active_session_id if it was the dead one
            if self.active_session_id.as_ref() == Some(&session_id) {
                self.active_session_id = None;
            }
        }
    }

    /// Refresh status of a session from its conversation file
    fn refresh_session_status(&mut self, session_id: &str) {
        // Skip temp session IDs (new sessions that haven't been persisted yet)
        if session_id.starts_with("__new_session_") {
            return;
        }

        // Find the conversation to get its project path
        let mut conv_info: Option<PathBuf> = None;
        for group in &self.groups {
            for conv in group.conversations() {
                if conv.session_id == session_id {
                    conv_info = Some(conv.project_path.clone());
                    break;
                }
            }
        }

        if let Some(project_path) = conv_info {
            // Build the conversation file path using escaped project path
            let escaped_path = project_path.to_string_lossy().replace('/', "-");
            let conv_path = self.claude_dir
                .join("projects")
                .join(&escaped_path)
                .join(format!("{}.jsonl", session_id));

            let status = if conv_path.exists() {
                detect_status_fast(&conv_path).unwrap_or(ConversationStatus::Idle)
            } else {
                ConversationStatus::Idle
            };

            self.update_conversation_status_in_groups(session_id, status);

            // Update selected_conversation if it matches
            if let Some(ref mut conv) = self.selected_conversation {
                if conv.session_id == session_id {
                    conv.status = status;
                }
            }
        }
    }
}
