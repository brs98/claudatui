use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;

use crate::claude::conversation::{detect_status_fast, Conversation, ConversationStatus};
use crate::claude::grouping::{group_conversations, ConversationGroup};
use crate::claude::sessions::{parse_all_sessions, SessionEntry};
use crate::claude::SessionsWatcher;
use crate::daemon::protocol::SessionState;
use crate::daemon_client::DaemonClient;
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
    /// Daemon client for session management
    pub daemon: Option<DaemonClient>,
    /// Currently active session ID (daemon's session ID)
    pub active_session_id: Option<String>,
    /// Cached session state for active session
    pub session_state_cache: Option<SessionState>,
    /// Mapping from daemon session ID to Claude session ID (for resuming)
    /// When a session is created with --resume, we store the Claude session ID here
    pub session_to_claude_id: HashMap<String, Option<String>>,
    /// Currently selected conversation
    pub selected_conversation: Option<Conversation>,
    /// Should quit
    pub should_quit: bool,
    /// Terminal size
    pub term_size: (u16, u16),
    /// Counter for generating temp session IDs for new conversations
    #[allow(dead_code)]
    new_session_counter: usize,
    /// Running sessions that haven't been saved yet (temp IDs)
    /// Maps daemon session_id -> project path
    pub ephemeral_sessions: HashMap<String, PathBuf>,
    /// Watcher for sessions-index.json changes
    sessions_watcher: Option<SessionsWatcher>,
    /// Timestamp of last refresh (for UI feedback)
    last_refresh: Option<Instant>,
    /// Whether last refresh was automatic (from watcher) vs manual
    last_refresh_was_auto: bool,
}

impl App {
    /// Create a new application instance
    pub fn new() -> Result<Self> {
        let claude_dir = dirs::home_dir()
            .expect("Could not find home directory")
            .join(".claude");

        // Create sessions watcher (optional - app works without it)
        let sessions_watcher = SessionsWatcher::new(claude_dir.clone()).ok();

        let mut app = Self {
            claude_dir,
            groups: Vec::new(),
            sidebar_state: SidebarState::new(),
            focus: Focus::Sidebar,
            daemon: None,
            active_session_id: None,
            session_state_cache: None,
            session_to_claude_id: HashMap::new(),
            selected_conversation: None,
            should_quit: false,
            term_size: (80, 24),
            new_session_counter: 0,
            ephemeral_sessions: HashMap::new(),
            sessions_watcher,
            last_refresh: None,
            last_refresh_was_auto: false,
        };

        app.load_conversations()?;

        Ok(app)
    }

    /// Connect to the daemon (lazily, on first session creation)
    fn ensure_daemon(&mut self) -> Result<()> {
        // Check if existing connection is healthy
        let needs_reconnect = match &self.daemon {
            Some(daemon) => !daemon.is_connected(),
            None => true,
        };

        if needs_reconnect {
            self.daemon = Some(DaemonClient::connect()?);
        }
        Ok(())
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
                    display: session.summary.clone().unwrap_or(session.first_prompt),
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
            &self.sidebar_state.expanded_conversations,
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
                SidebarItem::ShowMoreConversations { group_key, .. } => {
                    self.sidebar_state.toggle_expanded_conversations(group_key);
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
                | SidebarItem::ShowMoreProjects { .. }
                | SidebarItem::ShowMoreConversations { .. } => {
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

                if let Some((path, claude_session_id, conv)) = target {
                    // Check if we already have a daemon session for this Claude session
                    let existing_session = self
                        .session_to_claude_id
                        .iter()
                        .find(|(_, v)| **v == Some(claude_session_id.clone()))
                        .map(|(k, _)| k.clone());

                    if let Some(session_id) = existing_session {
                        self.active_session_id = Some(session_id);
                    } else {
                        // Start new session with --resume
                        self.start_session(&path, Some(&claude_session_id))?;
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
            Some(SidebarItem::ShowMoreConversations { group_key, .. }) => {
                // Toggle showing all conversations for this group
                self.sidebar_state.toggle_expanded_conversations(group_key);
            }
            None => {}
        }

        Ok(())
    }

    /// Start a new session (or resume one) in the given directory.
    ///
    /// If `claude_session_id` is provided, resumes an existing conversation.
    /// Otherwise starts a new conversation.
    fn start_session(
        &mut self,
        working_dir: &std::path::Path,
        claude_session_id: Option<&str>,
    ) -> Result<()> {
        self.ensure_daemon()?;

        // Calculate terminal pane size (75% of total width minus borders)
        let cols = (self.term_size.0 * 75 / 100).saturating_sub(2);
        let rows = self.term_size.1.saturating_sub(3); // Account for borders and help bar

        let daemon = self.daemon.as_ref().unwrap();
        let session_id = daemon.create_session(
            &working_dir.to_string_lossy(),
            claude_session_id,
            rows,
            cols,
        )?;

        // Track the mapping from daemon session to Claude session
        self.session_to_claude_id
            .insert(session_id.clone(), claude_session_id.map(|s| s.to_string()));

        // Track ephemeral sessions (new sessions without a saved conversation file)
        if claude_session_id.is_none() {
            self.ephemeral_sessions
                .insert(session_id.clone(), working_dir.to_path_buf());
        }

        self.active_session_id = Some(session_id);

        // Set conversation status to Active
        if let Some(ref mut conv) = self.selected_conversation {
            conv.status = ConversationStatus::Active;
            let sid = conv.session_id.clone();
            self.update_conversation_status_in_groups(&sid, ConversationStatus::Active);
        }

        Ok(())
    }

    /// Get the cached session state for rendering
    pub fn get_session_state(&self) -> Option<&SessionState> {
        self.session_state_cache.as_ref()
    }

    /// Get set of running session IDs for sidebar display
    pub fn running_session_ids(&self) -> HashSet<String> {
        // Return Claude session IDs for sessions that are running
        self.session_to_claude_id
            .iter()
            .filter_map(|(_, claude_id)| claude_id.clone())
            .collect()
    }

    /// Update session state from daemon (call this in the main loop)
    pub fn update_session_state(&mut self) {
        if let Some(ref session_id) = self.active_session_id {
            if let Some(ref daemon) = self.daemon {
                match daemon.get_session_state(session_id) {
                    Ok(state) => {
                        self.session_state_cache = Some(state);
                    }
                    Err(_) => {
                        // Session may have died
                        self.session_state_cache = None;
                    }
                }
            }
        } else {
            self.session_state_cache = None;
        }
    }

    /// Scroll up by the specified number of lines (active session only)
    pub fn scroll_up(&mut self, lines: usize) {
        // For now, scrolling is handled locally with cached state
        // The daemon maintains the actual scroll state
        if let Some(ref mut state) = self.session_state_cache {
            state.scroll_offset = state.scroll_offset.saturating_add(lines);
            state.scroll_locked = state.scroll_offset > 0;
        }
    }

    /// Scroll down by the specified number of lines (active session only)
    pub fn scroll_down(&mut self, lines: usize) {
        if let Some(ref mut state) = self.session_state_cache {
            state.scroll_offset = state.scroll_offset.saturating_sub(lines);
            state.scroll_locked = state.scroll_offset > 0;
        }
    }

    /// Jump to the bottom (live view) for active session
    pub fn scroll_to_bottom(&mut self) {
        if let Some(ref mut state) = self.session_state_cache {
            state.scroll_offset = 0;
            state.scroll_locked = false;
        }
    }

    /// Check if active session is scroll locked
    pub fn is_scroll_locked(&self) -> bool {
        self.session_state_cache
            .as_ref()
            .map(|s| s.scroll_locked)
            .unwrap_or(false)
    }

    /// Write input to active session's PTY
    pub fn write_to_pty(&mut self, data: &[u8]) -> Result<()> {
        if let Some(ref session_id) = self.active_session_id {
            if let Some(ref daemon) = self.daemon {
                daemon.write_to_session(session_id, data)?;
            }
        }
        Ok(())
    }

    /// Resize all running sessions
    pub fn resize(&mut self, width: u16, height: u16) -> Result<()> {
        self.term_size = (width, height);

        let cols = (width * 75 / 100).saturating_sub(2);
        let rows = height.saturating_sub(3);

        // Resize all sessions in daemon
        if let Some(ref daemon) = self.daemon {
            for session_id in self.session_to_claude_id.keys() {
                let _ = daemon.resize_session(session_id, rows, cols);
            }
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
        if self.daemon.is_none() {
            return;
        }

        let daemon = self.daemon.as_ref().unwrap();

        // Get list of sessions from daemon
        let sessions = match daemon.list_sessions() {
            Ok(s) => s,
            Err(_) => return,
        };

        // Find alive session IDs
        let alive_ids: HashSet<String> = sessions.iter().map(|s| s.session_id.clone()).collect();

        // Remove sessions that are no longer in daemon
        let dead_sessions: Vec<String> = self
            .session_to_claude_id
            .keys()
            .filter(|id| !alive_ids.contains(*id))
            .cloned()
            .collect();

        for session_id in dead_sessions {
            // Get the Claude session ID before removing
            let claude_id = self.session_to_claude_id.remove(&session_id);

            // Remove from ephemeral sessions if present
            self.ephemeral_sessions.remove(&session_id);

            // Re-read conversation status from file
            if let Some(Some(cid)) = claude_id {
                self.refresh_session_status(&cid);
            }

            // Clear active_session_id if it was the dead one
            if self.active_session_id.as_ref() == Some(&session_id) {
                self.active_session_id = None;
                self.session_state_cache = None;
                // Return focus to sidebar when viewed session closes
                if self.focus == Focus::Terminal {
                    self.focus = Focus::Sidebar;
                }
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
            let conv_path = self
                .claude_dir
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

    /// Check for sessions-index.json changes and reload conversations if needed
    pub fn check_sessions_updates(&mut self) {
        if let Some(ref watcher) = self.sessions_watcher {
            // Drain all pending notifications
            let mut should_reload = false;
            while watcher.try_recv().is_some() {
                should_reload = true;
            }

            if should_reload {
                if self.load_conversations().is_ok() {
                    self.last_refresh = Some(Instant::now());
                    self.last_refresh_was_auto = true;
                }
            }
        }
    }

    /// Manual refresh triggered by user (e.g., pressing 'r')
    pub fn manual_refresh(&mut self) -> Result<()> {
        self.load_conversations()?;
        self.last_refresh = Some(Instant::now());
        self.last_refresh_was_auto = false;
        Ok(())
    }

    /// Check if a refresh happened recently (within the given duration)
    /// Returns Some((is_auto, elapsed)) if refresh was recent, None otherwise
    pub fn recent_refresh(&self, within_ms: u64) -> Option<(bool, u64)> {
        self.last_refresh.and_then(|t| {
            let elapsed = t.elapsed().as_millis() as u64;
            if elapsed < within_ms {
                Some((self.last_refresh_was_auto, elapsed))
            } else {
                None
            }
        })
    }

    /// Get the page size for scrolling based on terminal dimensions
    pub fn get_page_size(&self) -> usize {
        self.session_state_cache
            .as_ref()
            .map(|s| s.rows.saturating_sub(2) as usize)
            .unwrap_or(20)
    }
}
