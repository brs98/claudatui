use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;

use crate::claude::conversation::{detect_status_fast, Conversation, ConversationStatus};
use crate::claude::grouping::{
    group_conversations, group_conversations_unordered, order_groups_by_keys, ConversationGroup,
};
use crate::claude::sessions::{parse_all_sessions, SessionEntry};
use crate::claude::SessionsWatcher;
use crate::session::{SessionManager, SessionState};
use crate::ui::modal::NewProjectModalState;
use crate::ui::sidebar::{build_sidebar_items, group_has_active_content, SidebarItem, SidebarState};

/// Clipboard status for feedback display
#[derive(Debug, Clone)]
pub enum ClipboardStatus {
    None,
    Copied { path: String, at: Instant },
}

/// Modal dialog state
pub enum ModalState {
    /// No modal is open
    None,
    /// New project modal is open
    NewProject(NewProjectModalState),
}

/// Which pane is currently focused
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Sidebar,
    Terminal,
}

/// State for tracking multi-key chord sequences (e.g., vim-style "dd" to delete)
#[derive(Debug, Clone, Default)]
pub enum ChordState {
    #[default]
    None,
    /// First 'd' pressed, waiting for second 'd' to close session
    DeletePending { started_at: Instant },
    /// Count prefix accumulating (e.g., "4" waiting for j/k to move 4 lines)
    CountPending { count: u32, started_at: Instant },
}

/// Info about an active session found in a group
#[derive(Debug, Clone)]
pub enum ActiveSessionInfo {
    /// An ephemeral session (new, not yet persisted)
    Ephemeral { index: usize, session_id: String },
    /// A persisted conversation
    Conversation {
        index: usize,
        session_id: String,
        conversation: Conversation,
    },
}

/// Tracks an ephemeral session (new conversation not yet persisted to disk)
#[derive(Clone, Debug)]
pub struct EphemeralSession {
    pub project_path: PathBuf,
    pub created_at: i64, // Unix timestamp when session was created
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
    /// Session manager for PTY sessions (owned directly, no daemon)
    pub session_manager: SessionManager,
    /// Currently active session ID
    pub active_session_id: Option<String>,
    /// Cached session state for active session
    pub session_state_cache: Option<SessionState>,
    /// Mapping from session ID to Claude session ID (for resuming)
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
    /// Maps daemon session_id -> ephemeral session info
    pub ephemeral_sessions: HashMap<String, EphemeralSession>,
    /// Watcher for sessions-index.json changes
    sessions_watcher: Option<SessionsWatcher>,
    /// Timestamp of last refresh (for UI feedback)
    last_refresh: Option<Instant>,
    /// Whether last refresh was automatic (from watcher) vs manual
    last_refresh_was_auto: bool,
    /// State for tracking chord key sequences (e.g., "dd" to close)
    pub chord_state: ChordState,
    /// Clipboard status for feedback display
    pub clipboard_status: ClipboardStatus,
    /// Ordered list of group keys to maintain stable group order during auto-refresh
    group_order: Vec<String>,
    /// Dangerous mode: when true, new sessions are started with --dangerously-skip-permissions
    pub dangerous_mode: bool,
    /// Current modal state
    pub modal_state: ModalState,
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
            session_manager: SessionManager::new(),
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
            chord_state: ChordState::None,
            clipboard_status: ClipboardStatus::None,
            group_order: Vec::new(),
            dangerous_mode: false,
            modal_state: ModalState::None,
        };

        app.load_conversations_full()?;

        Ok(app)
    }

    /// Load conversations with full re-sort (initial load and manual refresh).
    /// Groups are sorted by most recent activity.
    fn load_conversations_full(&mut self) -> Result<()> {
        let sessions = parse_all_sessions(&self.claude_dir)?;
        let conversations = self.sessions_to_conversations(sessions);
        self.groups = group_conversations(conversations);
        self.group_order = self.groups.iter().map(|g| g.key()).collect();
        Ok(())
    }

    /// Load conversations preserving existing group order (auto-refresh).
    /// New groups appear at the front, existing groups maintain their position.
    fn load_conversations_preserve_order(&mut self) -> Result<()> {
        let sessions = parse_all_sessions(&self.claude_dir)?;
        let conversations = self.sessions_to_conversations(sessions);
        let groups = group_conversations_unordered(conversations);
        let (ordered_groups, updated_order) = order_groups_by_keys(groups, &self.group_order);
        self.groups = ordered_groups;
        self.group_order = updated_order;
        Ok(())
    }

    /// Convert SessionEntry list to Conversation list
    fn sessions_to_conversations(&self, sessions: Vec<SessionEntry>) -> Vec<Conversation> {
        let running = self.running_session_ids();

        sessions
            .into_iter()
            .map(|session| {
                // Check conversation file for status using the full path
                let conv_path = PathBuf::from(&session.full_path);
                let mut status = if conv_path.exists() {
                    detect_status_fast(&conv_path).unwrap_or(ConversationStatus::Idle)
                } else {
                    ConversationStatus::Idle
                };

                // Only show WaitingForInput if session is actually running
                if status == ConversationStatus::WaitingForInput
                    && !running.contains(&session.session_id)
                {
                    status = ConversationStatus::Idle;
                }

                // Detect plan implementation conversations by checking first_prompt
                let is_plan_implementation = session
                    .first_prompt
                    .starts_with("Implement the following plan:");

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
                    is_plan_implementation,
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
            &self.running_session_ids(),
            self.sidebar_state.hide_inactive,
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

    /// Check if a group has active content (running session, ephemeral session, or non-idle conversation)
    fn is_group_active(&self, group_key: &str) -> bool {
        let running = self.running_session_ids();
        for group in &self.groups {
            if group.key() == group_key {
                return group_has_active_content(group, &running, &self.ephemeral_sessions);
            }
        }
        false
    }

    /// Find the active session within a group (ephemeral or running conversation)
    ///
    /// Priority:
    /// 1. Ephemeral sessions (new conversations not yet persisted)
    /// 2. Running conversations (have active PTYs)
    /// 3. Active/WaitingForInput status conversations
    ///
    /// Returns the sidebar item index and session info if found.
    fn find_active_session_in_group(&self, group_key: &str) -> Option<ActiveSessionInfo> {
        let items = self.sidebar_items();
        let running = self.running_session_ids();

        // Find the group
        let group = self.groups.iter().find(|g| g.key() == group_key)?;
        let group_project_path = group.project_path();

        // Priority 1: Check for ephemeral sessions
        if let Some(project_path) = group_project_path {
            for (session_id, ephemeral) in &self.ephemeral_sessions {
                if ephemeral.project_path == project_path {
                    // Find this ephemeral session's index in the sidebar items
                    let index = items.iter().position(|item| {
                        matches!(item, SidebarItem::EphemeralSession { session_id: sid, .. } if sid == session_id)
                    });
                    if let Some(idx) = index {
                        return Some(ActiveSessionInfo::Ephemeral {
                            index: idx,
                            session_id: session_id.clone(),
                        });
                    }
                }
            }
        }

        // Priority 2 & 3: Check conversations (running first, then active status)
        let conversations = group.conversations();

        // First pass: find running conversations (excluding plan implementations)
        for (conv_idx, conv) in conversations.iter().enumerate() {
            if conv.is_plan_implementation {
                continue;
            }
            if running.contains(&conv.session_id) {
                // Find this conversation's index in the sidebar items
                let index = items.iter().position(|item| {
                    matches!(item, SidebarItem::Conversation { group_key: gk, index } if gk == group_key && *index == conv_idx)
                });
                if let Some(idx) = index {
                    return Some(ActiveSessionInfo::Conversation {
                        index: idx,
                        session_id: conv.session_id.clone(),
                        conversation: conv.clone(),
                    });
                }
            }
        }

        // Second pass: find conversations with Active or WaitingForInput status
        for (conv_idx, conv) in conversations.iter().enumerate() {
            if conv.is_plan_implementation {
                continue;
            }
            if matches!(
                conv.status,
                ConversationStatus::Active | ConversationStatus::WaitingForInput
            ) {
                // Find this conversation's index in the sidebar items
                let index = items.iter().position(|item| {
                    matches!(item, SidebarItem::Conversation { group_key: gk, index } if gk == group_key && *index == conv_idx)
                });
                if let Some(idx) = index {
                    return Some(ActiveSessionInfo::Conversation {
                        index: idx,
                        session_id: conv.session_id.clone(),
                        conversation: conv.clone(),
                    });
                }
            }
        }

        None
    }

    /// Get the group key for the item at the given index
    fn get_group_key_for_index(&self, items: &[SidebarItem], index: usize) -> Option<String> {
        items.get(index).map(|item| match item {
            SidebarItem::GroupHeader { key, .. } => key.clone(),
            SidebarItem::Conversation { group_key, .. } => group_key.clone(),
            SidebarItem::EphemeralSession { group_key, .. } => group_key.clone(),
            SidebarItem::ShowMoreConversations { group_key, .. } => group_key.clone(),
            SidebarItem::ShowMoreProjects { .. } => String::new(),
        })
    }

    /// Cycle forward to the next active project's group header
    /// Returns true if successfully moved to a different active project
    pub fn cycle_next_active_project(&mut self) -> bool {
        let items = self.sidebar_items();
        if items.is_empty() {
            return false;
        }

        // Build list of (index, group_key) for active group headers
        let active_headers: Vec<(usize, String)> = items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                if let SidebarItem::GroupHeader { key, .. } = item {
                    if self.is_group_active(key) {
                        return Some((idx, key.clone()));
                    }
                }
                None
            })
            .collect();

        if active_headers.is_empty() {
            return false;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let current_group = self.get_group_key_for_index(&items, current);

        // Find the next active header after current position (with wrap)
        let next = active_headers
            .iter()
            .find(|(idx, key)| *idx > current && Some(key) != current_group.as_ref())
            .or_else(|| {
                // Wrap around - find first active header with different key
                active_headers
                    .iter()
                    .find(|(_, key)| Some(key) != current_group.as_ref())
            })
            .or_else(|| {
                // Only one active project - just go to its header
                active_headers.first()
            });

        if let Some((idx, _)) = next {
            self.sidebar_state.list_state.select(Some(*idx));
            self.update_selected_conversation();
            true
        } else {
            false
        }
    }

    /// Cycle backward to the previous active project's group header
    /// Returns true if successfully moved to a different active project
    pub fn cycle_prev_active_project(&mut self) -> bool {
        let items = self.sidebar_items();
        if items.is_empty() {
            return false;
        }

        // Build list of (index, group_key) for active group headers
        let active_headers: Vec<(usize, String)> = items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                if let SidebarItem::GroupHeader { key, .. } = item {
                    if self.is_group_active(key) {
                        return Some((idx, key.clone()));
                    }
                }
                None
            })
            .collect();

        if active_headers.is_empty() {
            return false;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let current_group = self.get_group_key_for_index(&items, current);

        // Find the previous active header before current position (with wrap)
        let prev = active_headers
            .iter()
            .rev()
            .find(|(idx, key)| *idx < current && Some(key) != current_group.as_ref())
            .or_else(|| {
                // Wrap around - find last active header with different key
                active_headers
                    .iter()
                    .rev()
                    .find(|(_, key)| Some(key) != current_group.as_ref())
            })
            .or_else(|| {
                // Only one active project - just go to its header
                active_headers.last()
            });

        if let Some((idx, _)) = prev {
            self.sidebar_state.list_state.select(Some(*idx));
            self.update_selected_conversation();
            true
        } else {
            false
        }
    }

    /// Cycle to next/previous active project AND switch to the active session within it.
    ///
    /// This method combines cycling and switching in one operation:
    /// 1. Cycles to the next (forward=true) or previous (forward=false) active project
    /// 2. Finds the active session within that project (ephemeral or running conversation)
    /// 3. Switches to that session and focuses the terminal
    ///
    /// Returns Ok(true) if successfully switched, Ok(false) if no switch occurred.
    pub fn cycle_and_switch_to_active(&mut self, forward: bool) -> Result<bool> {
        // First, cycle to the target group
        let cycled = if forward {
            self.cycle_next_active_project()
        } else {
            self.cycle_prev_active_project()
        };

        if !cycled {
            return Ok(false);
        }

        // Get the group key from the newly selected position
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);
        let group_key = match items.get(selected) {
            Some(SidebarItem::GroupHeader { key, .. }) => key.clone(),
            _ => return Ok(false),
        };

        // Find the active session in this group
        if let Some(session_info) = self.find_active_session_in_group(&group_key) {
            match session_info {
                ActiveSessionInfo::Ephemeral { index, session_id } => {
                    // Select the ephemeral session in sidebar
                    self.sidebar_state.list_state.select(Some(index));
                    // Switch to the already-running ephemeral session
                    self.active_session_id = Some(session_id);
                    self.selected_conversation = None;
                    self.focus = Focus::Terminal;
                }
                ActiveSessionInfo::Conversation {
                    index,
                    session_id: claude_session_id,
                    conversation,
                } => {
                    // Select the conversation in sidebar
                    self.sidebar_state.list_state.select(Some(index));
                    self.selected_conversation = Some(conversation.clone());

                    // Check if we already have a daemon session for this conversation
                    let existing_session = self
                        .session_to_claude_id
                        .iter()
                        .find(|(_, v)| **v == Some(claude_session_id.clone()))
                        .map(|(k, _)| k.clone());

                    if let Some(daemon_session_id) = existing_session {
                        // Switch to existing session
                        self.active_session_id = Some(daemon_session_id);
                    } else {
                        // Start new session with --resume
                        self.start_session(&conversation.project_path, Some(&claude_session_id))?;
                    }
                    self.focus = Focus::Terminal;
                }
            }
            return Ok(true);
        }

        Ok(false)
    }

    /// Navigate up by N items (clamping at top, no wrap)
    pub fn navigate_up_by(&mut self, count: usize) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let new_idx = current.saturating_sub(count);
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Navigate down by N items (clamping at bottom, no wrap)
    pub fn navigate_down_by(&mut self, count: usize) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let max_idx = items.len().saturating_sub(1);
        let new_idx = (current + count).min(max_idx);
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
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
        // Calculate terminal pane size (75% of total width minus borders)
        let cols = (self.term_size.0 * 75 / 100).saturating_sub(2);
        let rows = self.term_size.1.saturating_sub(3); // Account for borders and help bar

        let session_id = self.session_manager.create_session(
            working_dir,
            claude_session_id,
            rows,
            cols,
            self.dangerous_mode,
        )?;

        // Track the mapping from session ID to Claude session
        self.session_to_claude_id
            .insert(session_id.clone(), claude_session_id.map(|s| s.to_string()));

        // Track ephemeral sessions (new sessions without a saved conversation file)
        if claude_session_id.is_none() {
            self.ephemeral_sessions.insert(
                session_id.clone(),
                EphemeralSession {
                    project_path: working_dir.to_path_buf(),
                    created_at: chrono::Utc::now().timestamp_millis(),
                },
            );
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

    /// Update session state cache (call this in the main loop after processing output)
    pub fn update_session_state(&mut self) {
        if let Some(ref session_id) = self.active_session_id {
            self.session_state_cache = self.session_manager.get_session_state(session_id);
        } else {
            self.session_state_cache = None;
        }
    }

    /// Scroll up by the specified number of lines (active session only)
    pub fn scroll_up(&mut self, lines: usize) {
        if let Some(ref session_id) = self.active_session_id.clone() {
            if let Some(session) = self.session_manager.get_session_mut(&session_id) {
                session.scroll_up(lines);
            }
        }
    }

    /// Scroll down by the specified number of lines (active session only)
    pub fn scroll_down(&mut self, lines: usize) {
        if let Some(ref session_id) = self.active_session_id.clone() {
            if let Some(session) = self.session_manager.get_session_mut(&session_id) {
                session.scroll_down(lines);
            }
        }
    }

    /// Jump to the bottom (live view) for active session
    pub fn scroll_to_bottom(&mut self) {
        if let Some(ref session_id) = self.active_session_id.clone() {
            if let Some(session) = self.session_manager.get_session_mut(&session_id) {
                session.scroll_to_bottom();
            }
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
        if let Some(ref session_id) = self.active_session_id.clone() {
            if let Some(session) = self.session_manager.get_session_mut(&session_id) {
                session.write(data)?;
            }
        }
        Ok(())
    }

    /// Resize all running sessions
    pub fn resize(&mut self, width: u16, height: u16) -> Result<()> {
        self.term_size = (width, height);

        let cols = (width * 75 / 100).saturating_sub(2);
        let rows = height.saturating_sub(3);

        // Resize all sessions directly
        for session_id in self.session_manager.session_ids() {
            if let Some(session) = self.session_manager.get_session_mut(&session_id) {
                let _ = session.resize(rows, cols);
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
        // Clean up dead sessions from the session manager
        let dead_sessions = self.session_manager.cleanup_dead();

        for session_id in dead_sessions {
            // Get the Claude session ID before removing from our mapping
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

    /// Check for sessions-index.json changes and reload conversations if needed.
    /// Uses preserve-order loading to maintain stable group positions during auto-refresh.
    pub fn check_sessions_updates(&mut self) {
        if let Some(ref watcher) = self.sessions_watcher {
            // Drain all pending notifications
            let mut should_reload = false;
            while watcher.try_recv().is_some() {
                should_reload = true;
            }

            if should_reload {
                if self.load_conversations_preserve_order().is_ok() {
                    self.cleanup_persisted_ephemeral_sessions();
                    self.last_refresh = Some(Instant::now());
                    self.last_refresh_was_auto = true;
                }
            }
        }
    }

    /// Remove ephemeral sessions that have been persisted to disk
    /// and update session mappings to point to the discovered conversations.
    ///
    /// When a new conversation is started, it appears in `ephemeral_sessions` until
    /// Claude writes the .jsonl file. Once the file exists and is discovered during
    /// `load_conversations()`, we need to:
    /// 1. Update `session_to_claude_id` to map daemon_id -> actual Claude session ID
    /// 2. Update `selected_conversation` if this is the active session
    /// 3. Remove the ephemeral entry to avoid duplicate sidebar entries
    ///
    /// The matching algorithm ensures correctness by:
    /// - Only matching conversations created AFTER the ephemeral session was started
    /// - Not matching conversations already claimed by another daemon session
    fn cleanup_persisted_ephemeral_sessions(&mut self) {
        // Build list of all conversations
        let all_convs: Vec<&Conversation> = self
            .groups
            .iter()
            .flat_map(|g| g.conversations())
            .collect();

        // Track which Claude session IDs are already claimed by a daemon
        let claimed_ids: HashSet<String> = self
            .session_to_claude_id
            .values()
            .filter_map(|v| v.clone())
            .collect();

        // Collect ephemeral sessions to process (avoid borrow issues)
        let ephemeral_to_process: Vec<(String, EphemeralSession)> = self
            .ephemeral_sessions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // For each ephemeral session, find a matching conversation
        for (daemon_id, ephemeral) in ephemeral_to_process {
            // Check if this ephemeral session's Claude ID is still None
            let needs_update = self
                .session_to_claude_id
                .get(&daemon_id)
                .map(|opt| opt.is_none())
                .unwrap_or(false);

            if !needs_update {
                continue;
            }

            // Find matching conversation:
            // 1. Same project path
            // 2. Created AFTER this ephemeral session was started
            // 3. Not already claimed by another daemon session
            let matching_conv = all_convs
                .iter()
                .filter(|c| c.project_path == ephemeral.project_path)
                .filter(|c| c.timestamp > ephemeral.created_at)
                .filter(|c| !claimed_ids.contains(&c.session_id))
                .min_by_key(|c| c.timestamp); // Oldest matching = most likely match

            if let Some(conv) = matching_conv {
                // Update the daemon → Claude ID mapping
                self.session_to_claude_id
                    .insert(daemon_id.clone(), Some(conv.session_id.clone()));

                // If this is the active session, update selected_conversation
                if self.active_session_id.as_ref() == Some(&daemon_id) {
                    self.selected_conversation = Some((*conv).clone());
                }

                // Remove from ephemeral_sessions
                self.ephemeral_sessions.remove(&daemon_id);
            }
        }
    }

    /// Manual refresh triggered by user (e.g., pressing 'r').
    /// Performs a full re-sort of groups by most recent activity.
    pub fn manual_refresh(&mut self) -> Result<()> {
        self.load_conversations_full()?;
        self.cleanup_persisted_ephemeral_sessions();
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

    /// Check if chord has timed out and reset if so
    pub fn check_chord_timeout(&mut self) {
        if self.chord_state.is_expired() {
            self.chord_state = ChordState::None;
        }
    }

    /// Close a session by its ID, cleaning up all associated state
    pub fn close_session(&mut self, session_id: &str) {
        // Close the session in the manager
        self.session_manager.close_session(session_id);

        // Remove from session_to_claude_id mapping
        self.session_to_claude_id.remove(session_id);

        // Remove from ephemeral_sessions if present
        self.ephemeral_sessions.remove(session_id);

        // Clear active_session_id if it was the closed one
        if self.active_session_id.as_ref() == Some(&session_id.to_string()) {
            self.active_session_id = None;
            self.session_state_cache = None;
            // Return focus to sidebar
            self.focus = Focus::Sidebar;
        }
    }

    /// Close the currently selected session in the sidebar
    ///
    /// For conversations: finds the running daemon session and closes it
    /// For ephemeral sessions: closes the session directly
    /// For other items (headers, show more): no-op
    pub fn close_selected_session(&mut self) {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                // Find the conversation's Claude session ID
                let mut claude_session_id: Option<String> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            claude_session_id = Some(conv.session_id.clone());
                            break;
                        }
                    }
                }

                if let Some(cid) = claude_session_id {
                    // Find the daemon session ID that maps to this Claude session
                    let daemon_session_id = self
                        .session_to_claude_id
                        .iter()
                        .find(|(_, v)| **v == Some(cid.clone()))
                        .map(|(k, _)| k.clone());

                    if let Some(dsid) = daemon_session_id {
                        self.close_session(&dsid);
                    }
                }
            }
            Some(SidebarItem::EphemeralSession { session_id, .. }) => {
                // Close the ephemeral session directly
                let session_id = session_id.clone();
                self.close_session(&session_id);
            }
            Some(SidebarItem::GroupHeader { .. })
            | Some(SidebarItem::ShowMoreProjects { .. })
            | Some(SidebarItem::ShowMoreConversations { .. })
            | None => {
                // No-op for non-session items
            }
        }
    }

    /// Copy the selected item's project path to the clipboard
    pub fn copy_selected_path_to_clipboard(&mut self) {
        let path = match self.get_selected_path() {
            Some(p) => p.clone(),
            None => return,
        };

        // Copy to clipboard using arboard
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let path_str = path.to_string_lossy().to_string();
            if clipboard.set_text(&path_str).is_ok() {
                self.clipboard_status = ClipboardStatus::Copied {
                    path: path_str,
                    at: Instant::now(),
                };
            }
        }
    }

    /// Get the project path of the currently selected sidebar item
    fn get_selected_path(&self) -> Option<&PathBuf> {
        // First try selected conversation
        if let Some(ref conv) = self.selected_conversation {
            return Some(&conv.project_path);
        }

        // Also support group headers and ephemeral sessions (which represent a project)
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::GroupHeader { key, .. }) => {
                for group in &self.groups {
                    if &group.key() == key {
                        // Get path from first conversation in group
                        if let Some(conv) = group.conversations().first() {
                            return Some(&conv.project_path);
                        }
                    }
                }
                None
            }
            Some(SidebarItem::EphemeralSession { session_id, .. }) => {
                self.ephemeral_sessions
                    .get(session_id)
                    .map(|e| &e.project_path)
            }
            _ => None,
        }
    }

    /// Toggle dangerous mode on/off
    pub fn toggle_dangerous_mode(&mut self) {
        self.dangerous_mode = !self.dangerous_mode;
    }

    /// Check if clipboard copy happened recently (for UI feedback)
    pub fn recent_clipboard_copy(&self, within_ms: u64) -> Option<&str> {
        match &self.clipboard_status {
            ClipboardStatus::None => None,
            ClipboardStatus::Copied { path, at } => {
                let elapsed = at.elapsed().as_millis() as u64;
                if elapsed < within_ms {
                    Some(path.as_str())
                } else {
                    None
                }
            }
        }
    }

    /// Open the new project modal dialog
    pub fn open_new_project_modal(&mut self) {
        self.modal_state = ModalState::NewProject(NewProjectModalState::new());
    }

    /// Close any open modal dialog
    pub fn close_modal(&mut self) {
        self.modal_state = ModalState::None;
    }

    /// Check if a modal is currently open
    pub fn is_modal_open(&self) -> bool {
        !matches!(self.modal_state, ModalState::None)
    }

    /// Confirm the new project modal selection and start a session
    pub fn confirm_new_project(&mut self, path: PathBuf) -> Result<()> {
        // Close the modal first
        self.modal_state = ModalState::None;

        // Start a new session in the selected directory
        self.start_session(&path, None)?;
        self.selected_conversation = None;
        self.focus = Focus::Terminal;

        Ok(())
    }
}

/// Chord state timeout duration (500ms)
const CHORD_TIMEOUT_MS: u64 = 500;

impl ChordState {
    /// Check if the chord has expired (timed out)
    pub fn is_expired(&self) -> bool {
        match self {
            ChordState::None => false,
            ChordState::DeletePending { started_at }
            | ChordState::CountPending { started_at, .. } => {
                started_at.elapsed().as_millis() as u64 > CHORD_TIMEOUT_MS
            }
        }
    }

    /// Get display text for pending chord (for UI feedback)
    /// Returns the pending key sequence (e.g., "d" for delete, "4" for count)
    pub fn pending_display(&self) -> Option<String> {
        match self {
            ChordState::None => None,
            ChordState::DeletePending { .. } => Some("d".to_string()),
            ChordState::CountPending { count, .. } => Some(count.to_string()),
        }
    }
}
