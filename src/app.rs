use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use crossterm::event::KeyEvent;

use ratatui::layout::Rect;

use crate::bookmarks::{Bookmark, BookmarkManager, BookmarkTarget};
use crate::claude::archive::ArchiveManager;
use crate::config::{Config, SidebarPosition};
use crate::input::{InputMode, LeaderState};
use crate::input::which_key::WhichKeyConfig;
use crate::search::SearchEngine;
use crate::claude::conversation::{detect_status_fast, Conversation, ConversationStatus};
use crate::claude::grouping::{
    group_conversations, group_conversations_unordered, order_groups_by_keys, ConversationGroup,
};
use crate::claude::sessions::{parse_all_sessions, SessionEntry};
use crate::claude::SessionsWatcher;
use crate::session::{SessionManager, SessionState};
use crate::ui::modal::{NewProjectModalState, SearchModalState};
use crate::ui::sidebar::{
    build_sidebar_items, group_has_active_content, SidebarItem, SidebarState,
};
use crate::ui::toast::{ToastManager, ToastType};

/// Clipboard status for feedback display
#[derive(Debug, Clone)]
pub enum ClipboardStatus {
    None,
    Copied { path: String, at: Instant },
}

/// Archive status for feedback display
#[derive(Debug, Clone)]
pub enum ArchiveStatus {
    None,
    Archived { session_id: String, at: Instant },
    Unarchived { session_id: String, at: Instant },
}

/// A position within the terminal content grid (row/col in screen coordinates)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalPosition {
    pub row: usize,
    pub col: usize,
}

/// Text selection in the terminal pane (anchor = mouse down, cursor = current drag position)
#[derive(Debug, Clone)]
pub struct TextSelection {
    pub anchor: TerminalPosition,
    pub cursor: TerminalPosition,
}

impl TextSelection {
    /// Return (start, end) sorted top-left to bottom-right
    pub fn ordered(&self) -> (TerminalPosition, TerminalPosition) {
        if self.anchor.row < self.cursor.row
            || (self.anchor.row == self.cursor.row && self.anchor.col <= self.cursor.col)
        {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    /// Check if a cell is within the selection (standard terminal stream selection)
    pub fn contains(&self, row: usize, col: usize) -> bool {
        let (start, end) = self.ordered();
        if start.row == end.row {
            // Single line: col must be in [start.col, end.col]
            row == start.row && col >= start.col && col <= end.col
        } else if row == start.row {
            // First line: from start.col to end of line
            col >= start.col
        } else if row == end.row {
            // Last line: from start of line to end.col
            col <= end.col
        } else {
            // Middle lines: fully selected
            row > start.row && row < end.row
        }
    }

    /// True when anchor and cursor are the same position (no real selection)
    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }
}

/// Modal dialog state
pub enum ModalState {
    /// No modal is open
    None,
    /// New project modal is open
    NewProject(Box<NewProjectModalState>),
    /// Search modal is open
    Search(Box<SearchModalState>),
}

/// Split mode configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplitMode {
    #[default]
    None,       // Single pane (current behavior)
    Horizontal, // Side-by-side (left/right)
    Vertical,   // Stacked (top/bottom)
}

impl SplitMode {
    /// Cycle to the next split mode (None -> Horizontal -> Vertical -> None)
    pub fn cycle(&self) -> Self {
        match self {
            SplitMode::None => SplitMode::Horizontal,
            SplitMode::Horizontal => SplitMode::Vertical,
            SplitMode::Vertical => SplitMode::None,
        }
    }
}

/// Identifies which terminal pane is active in split mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalPaneId {
    #[default]
    Primary,   // Left or Top pane
    Secondary, // Right or Bottom pane (only in split mode)
}

impl TerminalPaneId {
    /// Convert to array index (0 for Primary, 1 for Secondary)
    pub fn index(&self) -> usize {
        match self {
            TerminalPaneId::Primary => 0,
            TerminalPaneId::Secondary => 1,
        }
    }

    /// Toggle to the other pane
    pub fn toggle(&self) -> Self {
        match self {
            TerminalPaneId::Primary => TerminalPaneId::Secondary,
            TerminalPaneId::Secondary => TerminalPaneId::Primary,
        }
    }
}

/// Configuration for each terminal pane
#[derive(Debug, Clone, Default)]
pub struct PaneConfig {
    pub session_id: Option<String>,
}

/// Which pane is currently focused
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Sidebar,
    Terminal(TerminalPaneId),
}

/// State for tracking jk/kj rapid-press escape sequence in insert mode
pub enum EscapeSequenceState {
    /// No pending escape key
    None,
    /// First key of a potential escape sequence has been pressed
    Pending {
        first_key: char,
        first_key_event: KeyEvent,
        started_at: Instant,
    },
}

impl Default for EscapeSequenceState {
    fn default() -> Self {
        EscapeSequenceState::None
    }
}

/// Timeout for jk/kj escape sequence detection (milliseconds)
pub const ESCAPE_SEQ_TIMEOUT_MS: u64 = 150;

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
    /// Currently active session ID (deprecated - use panes[].session_id)
    pub active_session_id: Option<String>,
    /// Session ID being previewed (shown in terminal pane while sidebar keeps focus)
    pub preview_session_id: Option<String>,
    /// Cached session state for active session (deprecated - use pane_state_caches)
    pub session_state_cache: Option<SessionState>,
    /// Current split mode
    pub split_mode: SplitMode,
    /// Pane configurations (indexed by TerminalPaneId)
    pub panes: [PaneConfig; 2],
    /// Which terminal pane has focus when in Terminal focus mode
    pub active_pane: TerminalPaneId,
    /// Cached session states for both panes
    pub pane_state_caches: [Option<SessionState>; 2],
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
    /// Timestamp when dangerous mode was last toggled (for temporary message display)
    pub dangerous_mode_toggled_at: Option<Instant>,
    /// Current modal state
    pub modal_state: ModalState,
    /// Toast notification manager
    pub toast_manager: ToastManager,
    /// Bookmark manager for quick access to projects/conversations
    pub bookmark_manager: BookmarkManager,
    /// Archive manager for persisting archive state
    pub archive_manager: ArchiveManager,
    /// Archive status for feedback display
    pub archive_status: ArchiveStatus,
    /// Search engine for finding conversations
    pub search_engine: SearchEngine,
    /// Application configuration (layout, etc.)
    pub config: Config,
    /// Current input mode (Normal, Insert, Leader)
    pub input_mode: InputMode,
    /// Which-key configuration and command tree
    pub which_key_config: WhichKeyConfig,
    /// State for jk/kj rapid-press escape sequence detection
    pub escape_seq_state: EscapeSequenceState,
    /// Active text selection in terminal pane (mouse drag)
    pub text_selection: Option<TextSelection>,
    /// Cached inner area of terminal pane (set during render, used for mouse coordinate mapping)
    pub terminal_inner_area: Option<Rect>,
}

impl App {
    /// Create a new application instance
    pub fn new() -> Result<Self> {
        let claude_dir = dirs::home_dir()
            .expect("Could not find home directory")
            .join(".claude");

        // Create sessions watcher (optional - app works without it)
        let sessions_watcher = SessionsWatcher::new(claude_dir.clone()).ok();

        // Create archive manager
        let archive_manager = ArchiveManager::new(&claude_dir).unwrap_or_else(|_| {
            ArchiveManager::new(&claude_dir).expect("Failed to create archive manager")
        });

        // Create search engine
        let search_engine = SearchEngine::new(claude_dir.clone());

        // Load configuration (use defaults if not found or invalid)
        let mut config = Config::load().unwrap_or_default();
        config.layout.validate();

        let mut app = Self {
            claude_dir,
            groups: Vec::new(),
            sidebar_state: SidebarState::new(),
            focus: Focus::Sidebar,
            session_manager: SessionManager::new(),
            active_session_id: None,
            preview_session_id: None,
            session_state_cache: None,
            split_mode: SplitMode::None,
            panes: [PaneConfig::default(), PaneConfig::default()],
            active_pane: TerminalPaneId::Primary,
            pane_state_caches: [None, None],
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
            dangerous_mode: config.dangerous_mode,
            dangerous_mode_toggled_at: None,
            modal_state: ModalState::None,
            toast_manager: ToastManager::new(),
            bookmark_manager: BookmarkManager::load().unwrap_or_else(|_| BookmarkManager::empty()),
            archive_manager,
            archive_status: ArchiveStatus::None,
            search_engine,
            config,
            input_mode: InputMode::default(),
            which_key_config: WhichKeyConfig::new(),
            escape_seq_state: EscapeSequenceState::None,
            text_selection: None,
            terminal_inner_area: None,
        };

        app.load_conversations_full()?;
        app.check_auto_archive();

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

                // Check archive status
                let is_archived = self.archive_manager.is_archived(&session.session_id);
                let archived_at = self.archive_manager.get_archived_at(&session.session_id);

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
                    is_archived,
                    archived_at,
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
            self.sidebar_state.archive_filter,
            &self.bookmark_manager,
        )
    }

    /// Navigate up in the sidebar, skipping non-selectable items
    pub fn navigate_up(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let len = items.len();
        let mut new_idx = if current == 0 { len - 1 } else { current - 1 };
        // Skip non-selectable items (bounded to prevent infinite loop)
        for _ in 0..len {
            if items[new_idx].is_selectable() {
                break;
            }
            new_idx = if new_idx == 0 { len - 1 } else { new_idx - 1 };
        }
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Navigate down in the sidebar, skipping non-selectable items
    pub fn navigate_down(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let len = items.len();
        let mut new_idx = if current >= len - 1 { 0 } else { current + 1 };
        // Skip non-selectable items (bounded to prevent infinite loop)
        for _ in 0..len {
            if items[new_idx].is_selectable() {
                break;
            }
            new_idx = if new_idx >= len - 1 { 0 } else { new_idx + 1 };
        }
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Jump to first selectable item
    pub fn jump_to_first(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }
        let first = items.iter().position(|item| item.is_selectable()).unwrap_or(0);
        self.sidebar_state.list_state.select(Some(first));
        self.update_selected_conversation();
    }

    /// Jump to last selectable item
    pub fn jump_to_last(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }
        let last = items.iter().rposition(|item| item.is_selectable()).unwrap_or(items.len() - 1);
        self.sidebar_state.list_state.select(Some(last));
        self.update_selected_conversation();
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
        items.get(index).and_then(|item| match item {
            SidebarItem::GroupHeader { key, .. } => Some(key.clone()),
            SidebarItem::Conversation { group_key, .. } => Some(group_key.clone()),
            SidebarItem::EphemeralSession { group_key, .. } => Some(group_key.clone()),
            SidebarItem::ShowMoreConversations { group_key, .. } => Some(group_key.clone()),
            SidebarItem::ShowMoreProjects { .. } => None,
            SidebarItem::BookmarkHeader
            | SidebarItem::BookmarkEntry { .. }
            | SidebarItem::BookmarkSeparator => None,
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
        self.clear_preview();
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
                    self.focus = Focus::Terminal(TerminalPaneId::Primary);
                    self.enter_insert_mode();
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
                    self.focus = Focus::Terminal(TerminalPaneId::Primary);
                    self.enter_insert_mode();
                }
            }
            return Ok(true);
        }

        Ok(false)
    }

    /// Navigate up by N selectable items (clamping at top, no wrap)
    pub fn navigate_up_by(&mut self, count: usize) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let mut new_idx = current;
        let mut remaining = count;
        while remaining > 0 && new_idx > 0 {
            new_idx -= 1;
            if items[new_idx].is_selectable() {
                remaining -= 1;
            }
        }
        // If we landed on a non-selectable item, scan forward to the nearest selectable
        if !items[new_idx].is_selectable() {
            if let Some(pos) = items[new_idx..].iter().position(|i| i.is_selectable()) {
                new_idx += pos;
            }
        }
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Navigate down by N selectable items (clamping at bottom, no wrap)
    pub fn navigate_down_by(&mut self, count: usize) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let max_idx = items.len() - 1;
        let mut new_idx = current;
        let mut remaining = count;
        while remaining > 0 && new_idx < max_idx {
            new_idx += 1;
            if items[new_idx].is_selectable() {
                remaining -= 1;
            }
        }
        // If we landed on a non-selectable item, scan backward to the nearest selectable
        if !items[new_idx].is_selectable() {
            if let Some(pos) = items[..=new_idx].iter().rposition(|i| i.is_selectable()) {
                new_idx = pos;
            }
        }
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
                SidebarItem::BookmarkHeader
                | SidebarItem::BookmarkEntry { .. }
                | SidebarItem::BookmarkSeparator => {}
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
                | SidebarItem::ShowMoreConversations { .. }
                | SidebarItem::BookmarkHeader
                | SidebarItem::BookmarkEntry { .. }
                | SidebarItem::BookmarkSeparator => {
                    // No conversation selected for headers, ephemeral sessions, bookmarks, or "show more" items
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
        self.clear_preview();
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
                    // Check if the working directory still exists
                    if !path.exists() {
                        // Directory was deleted (e.g., git worktree removed)
                        // Just select the conversation but don't start a session
                        self.selected_conversation = Some(conv);
                        return Ok(());
                    }

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
                    self.focus = Focus::Terminal(TerminalPaneId::Primary);
                    self.enter_insert_mode();
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
                    self.focus = Focus::Terminal(TerminalPaneId::Primary);
                    self.enter_insert_mode();
                }
            }
            Some(SidebarItem::EphemeralSession { session_id, .. }) => {
                // Switch to the already-running ephemeral session
                self.active_session_id = Some(session_id.clone());
                self.selected_conversation = None;
                self.focus = Focus::Terminal(TerminalPaneId::Primary);
                self.enter_insert_mode();
            }
            Some(SidebarItem::ShowMoreProjects { .. }) => {
                // Toggle showing all projects
                self.sidebar_state.toggle_show_all_projects();
            }
            Some(SidebarItem::ShowMoreConversations { group_key, .. }) => {
                // Toggle showing all conversations for this group
                self.sidebar_state.toggle_expanded_conversations(group_key);
            }
            Some(SidebarItem::BookmarkEntry { slot }) => {
                // Jump to bookmarked target
                self.jump_to_bookmark(*slot)?;
            }
            Some(SidebarItem::BookmarkHeader) | Some(SidebarItem::BookmarkSeparator) | None => {}
        }

        Ok(())
    }

    /// Create a new conversation in whichever group the selected sidebar item belongs to.
    ///
    /// Unlike `open_selected()`, this always starts a fresh conversation regardless
    /// of which item type is selected (GroupHeader, Conversation, EphemeralSession, etc.).
    pub fn new_conversation_in_selected_group(&mut self) -> Result<()> {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        // Extract the group key from whatever item is selected
        let group_key = match items.get(selected) {
            Some(SidebarItem::GroupHeader { key, .. }) => Some(key.clone()),
            Some(SidebarItem::Conversation { group_key, .. }) => Some(group_key.clone()),
            Some(SidebarItem::EphemeralSession { group_key, .. }) => Some(group_key.clone()),
            Some(SidebarItem::ShowMoreConversations { group_key, .. }) => Some(group_key.clone()),
            _ => None,
        };

        if let Some(key) = group_key {
            // Find the group's project path
            let project_path = self
                .groups
                .iter()
                .find(|g| g.key() == key)
                .and_then(|g| g.project_path());

            if let Some(path) = project_path {
                self.selected_conversation = None;
                self.start_session(&path, None)?;

                // Position sidebar cursor on the newly created ephemeral session
                if let Some(ref new_sid) = self.active_session_id {
                    let new_items = self.sidebar_items();
                    if let Some(idx) = new_items.iter().position(|item| {
                        matches!(item, SidebarItem::EphemeralSession { session_id, .. } if session_id == new_sid)
                    }) {
                        self.sidebar_state.list_state.select(Some(idx));
                    }
                }

                self.focus = Focus::Terminal(TerminalPaneId::Primary);
                self.enter_insert_mode();
            }
        }

        Ok(())
    }

    /// Preview the selected sidebar item in the terminal pane without leaving the sidebar.
    ///
    /// Shows the session output in the terminal pane but keeps focus on the sidebar
    /// and stays in normal mode. Toggle behavior: pressing `p` again on the same
    /// conversation clears the preview.
    pub fn preview_selected(&mut self) -> Result<()> {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                let mut target: Option<(PathBuf, String)> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            target = Some((conv.project_path.clone(), conv.session_id.clone()));
                            break;
                        }
                    }
                }

                if let Some((path, claude_session_id)) = target {
                    // Toggle: if already previewing this session, clear preview
                    let existing_session = self
                        .session_to_claude_id
                        .iter()
                        .find(|(_, v)| **v == Some(claude_session_id.clone()))
                        .map(|(k, _)| k.clone());

                    if let Some(ref sid) = existing_session {
                        if self.preview_session_id.as_ref() == Some(sid) {
                            self.preview_session_id = None;
                            return Ok(());
                        }
                    }

                    // Check if directory still exists
                    if !path.exists() {
                        return Ok(());
                    }

                    if let Some(session_id) = existing_session {
                        self.preview_session_id = Some(session_id);
                    } else {
                        // Need to spawn a session — save/restore active_session_id
                        let prev_active = self.active_session_id.clone();
                        self.start_session(&path, Some(&claude_session_id))?;
                        let new_session_id = self.active_session_id.clone();
                        self.active_session_id = prev_active;
                        self.preview_session_id = new_session_id;
                    }
                }
            }
            Some(SidebarItem::EphemeralSession { session_id, .. }) => {
                // Toggle: if already previewing this session, clear preview
                if self.preview_session_id.as_ref() == Some(session_id) {
                    self.preview_session_id = None;
                    return Ok(());
                }
                self.preview_session_id = Some(session_id.clone());
            }
            _ => {}
        }

        Ok(())
    }

    /// Clear the preview session
    pub fn clear_preview(&mut self) {
        self.preview_session_id = None;
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
        let (rows, cols) = self.calculate_terminal_dimensions();

        let result = self.session_manager.create_session(
            working_dir,
            claude_session_id,
            rows,
            cols,
            self.dangerous_mode,
        );

        match result {
            Ok(session_id) => {
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

                self.toast_success("Session started");
                Ok(())
            }
            Err(e) => {
                self.toast_error(format!("Failed to start session: {}", e));
                Err(e)
            }
        }
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
    ///
    /// Prefers preview_session_id when set (preview mode shows the previewed session),
    /// otherwise falls back to active_session_id.
    pub fn update_session_state(&mut self) {
        let display_session = self
            .preview_session_id
            .as_ref()
            .or(self.active_session_id.as_ref());

        if let Some(session_id) = display_session {
            self.session_state_cache = self.session_manager.get_session_state(session_id);
        } else {
            self.session_state_cache = None;
        }
    }

    /// Scroll up by the specified number of lines (active session only)
    pub fn scroll_up(&mut self, lines: usize) {
        if let Some(ref session_id) = self.active_session_id.clone() {
            if let Some(session) = self.session_manager.get_session_mut(session_id) {
                session.scroll_up(lines);
            }
        }
    }

    /// Scroll down by the specified number of lines (active session only)
    pub fn scroll_down(&mut self, lines: usize) {
        if let Some(ref session_id) = self.active_session_id.clone() {
            if let Some(session) = self.session_manager.get_session_mut(session_id) {
                session.scroll_down(lines);
            }
        }
    }

    /// Jump to the bottom (live view) for active session
    pub fn scroll_to_bottom(&mut self) {
        if let Some(ref session_id) = self.active_session_id.clone() {
            if let Some(session) = self.session_manager.get_session_mut(session_id) {
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
            if let Some(session) = self.session_manager.get_session_mut(session_id) {
                session.write(data)?;
            }
        }
        Ok(())
    }

    /// Resize all running sessions
    pub fn resize(&mut self, width: u16, height: u16) -> Result<()> {
        self.term_size = (width, height);

        let (rows, cols) = self.calculate_terminal_dimensions();

        // Resize all sessions directly
        for session_id in self.session_manager.session_ids() {
            if let Some(session) = self.session_manager.get_session_mut(&session_id) {
                let _ = session.resize(rows, cols);
            }
        }

        Ok(())
    }

    /// Calculate terminal pane dimensions based on current config and term size
    fn calculate_terminal_dimensions(&self) -> (u16, u16) {
        let sidebar_pct = if self.config.layout.sidebar_minimized {
            3 // Minimized sidebar takes ~3 columns
        } else {
            self.config.layout.sidebar_width_pct as u16
        };

        let terminal_pct = 100u16.saturating_sub(sidebar_pct);
        let cols = (self.term_size.0 * terminal_pct / 100).saturating_sub(2);
        let rows = self.term_size.1.saturating_sub(3); // Account for borders and help bar

        (rows, cols)
    }

    /// Update the status of a conversation in the groups vector
    fn update_conversation_status_in_groups(
        &mut self,
        session_id: &str,
        status: ConversationStatus,
    ) {
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

            // Clear preview if the dead session was being previewed
            if self.preview_session_id.as_ref() == Some(&session_id) {
                self.preview_session_id = None;
            }

            // Clear active_session_id if it was the dead one
            if self.active_session_id.as_ref() == Some(&session_id) {
                self.active_session_id = None;
                self.session_state_cache = None;
                // Return focus to sidebar when viewed session closes
                if matches!(self.focus, Focus::Terminal(_)) {
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

            if should_reload && self.load_conversations_preserve_order().is_ok() {
                self.cleanup_persisted_ephemeral_sessions();
                self.last_refresh = Some(Instant::now());
                self.last_refresh_was_auto = true;
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
        let all_convs: Vec<&Conversation> =
            self.groups.iter().flat_map(|g| g.conversations()).collect();

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

    /// Check if escape sequence has timed out and return the buffered key if so.
    /// Returns Some(KeyEvent) if the pending key expired and needs to be flushed.
    pub fn check_escape_seq_timeout(&mut self) -> Option<KeyEvent> {
        if let EscapeSequenceState::Pending {
            first_key_event,
            started_at,
            ..
        } = &self.escape_seq_state
        {
            if started_at.elapsed().as_millis() as u64 > ESCAPE_SEQ_TIMEOUT_MS {
                let event = *first_key_event;
                self.escape_seq_state = EscapeSequenceState::None;
                return Some(event);
            }
        }
        None
    }

    /// Check if chord has timed out and reset if so
    pub fn check_chord_timeout(&mut self) {
        if self.chord_state.is_expired() {
            self.chord_state = ChordState::None;
        }
    }

    /// Check if leader mode has timed out and reset if so.
    /// - At root level (no submenu entered): 2s auto-timeout applies
    /// - In a submenu: no auto-timeout, user browses freely
    /// - pending_escape timeout (150ms): if a buffered j/k expires, cancel leader mode
    pub fn check_leader_timeout(&mut self) {
        if let InputMode::Leader(ref state) = self.input_mode {
            // Only auto-timeout at root level (before entering any submenu)
            if state.path.is_empty()
                && state.is_expired(self.which_key_config.timeout_ms)
            {
                self.input_mode = InputMode::Normal;
                return;
            }
            // If a pending escape key (j or k) has timed out, it was an invalid key — cancel
            if let Some((_, started_at)) = state.pending_escape {
                if started_at.elapsed().as_millis() as u64 > ESCAPE_SEQ_TIMEOUT_MS {
                    self.input_mode = InputMode::Normal;
                }
            }
        }
    }

    /// Enter leader mode
    pub fn enter_leader_mode(&mut self) {
        self.input_mode = InputMode::Leader(LeaderState::new());
    }

    /// Exit leader mode and return to normal
    pub fn exit_leader_mode(&mut self) {
        self.input_mode = InputMode::Normal;
    }

    /// Enter insert mode
    pub fn enter_insert_mode(&mut self) {
        self.input_mode = InputMode::Insert;
        self.escape_seq_state = EscapeSequenceState::None;
    }

    /// Exit insert mode and return to normal
    pub fn exit_insert_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.escape_seq_state = EscapeSequenceState::None;
    }

    /// Close a session by its ID, cleaning up all associated state
    pub fn close_session(&mut self, session_id: &str) {
        self.session_manager.close_session(session_id);
        self.session_to_claude_id.remove(session_id);
        self.ephemeral_sessions.remove(session_id);

        // Clear preview if we're closing the previewed session
        if self.preview_session_id.as_ref() == Some(&session_id.to_string()) {
            self.preview_session_id = None;
        }

        if self.active_session_id.as_ref() == Some(&session_id.to_string()) {
            self.active_session_id = None;
            self.session_state_cache = None;
            if matches!(self.focus, Focus::Terminal(_)) {
                self.focus = Focus::Sidebar;
            }
        }

        self.toast_info("Session closed");
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
            | Some(SidebarItem::BookmarkHeader)
            | Some(SidebarItem::BookmarkEntry { .. })
            | Some(SidebarItem::BookmarkSeparator)
            | None => {
                // No-op for non-session items
            }
        }
    }

    /// Copy the selected item's project path to the clipboard
    pub fn copy_selected_path_to_clipboard(&mut self) {
        let path = match self.get_selected_path() {
            Some(p) => p.clone(),
            None => {
                self.toast_error("No path selected");
                return;
            }
        };

        // Copy to clipboard using arboard
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let path_str = path.to_string_lossy().to_string();
            if clipboard.set_text(&path_str).is_ok() {
                self.toast_success("Copied to clipboard");
            } else {
                self.toast_error("Failed to copy");
            }
        } else {
            self.toast_error("Clipboard unavailable");
        }
    }

    /// Clear the text selection
    pub fn clear_selection(&mut self) {
        self.text_selection = None;
    }

    /// Copy the current text selection to the system clipboard
    pub fn copy_selection_to_clipboard(&mut self) {
        let text = match (&self.text_selection, &self.session_state_cache) {
            (Some(sel), Some(state)) => extract_selected_text(&state.screen, sel),
            _ => return,
        };

        if text.is_empty() {
            return;
        }

        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if clipboard.set_text(&text).is_ok() {
                let lines = text.lines().count();
                let chars = text.len();
                self.toast_success(format!("Copied selection ({lines} lines, {chars} chars)"));
            } else {
                self.toast_error("Failed to copy selection");
            }
        } else {
            self.toast_error("Clipboard unavailable");
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
            Some(SidebarItem::EphemeralSession { session_id, .. }) => self
                .ephemeral_sessions
                .get(session_id)
                .map(|e| &e.project_path),
            _ => None,
        }
    }

    /// Toggle dangerous mode on/off
    pub fn toggle_dangerous_mode(&mut self) {
        self.dangerous_mode = !self.dangerous_mode;
        self.dangerous_mode_toggled_at = Some(Instant::now());
        self.config.dangerous_mode = self.dangerous_mode;
        let _ = self.config.save();
        if self.dangerous_mode {
            self.toast_warning("Dangerous mode enabled");
        } else {
            self.toast_info("Normal mode restored");
        }
    }

    /// Check if dangerous mode was recently toggled (for temporary message display)
    /// Returns Some(true) if entering dangerous, Some(false) if exiting, None if not recent
    pub fn recent_dangerous_mode_toggle(&self, within_ms: u64) -> Option<bool> {
        self.dangerous_mode_toggled_at.and_then(|t| {
            let elapsed = t.elapsed().as_millis() as u64;
            if elapsed < within_ms {
                Some(self.dangerous_mode)
            } else {
                None
            }
        })
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
        self.modal_state = ModalState::NewProject(Box::default());
        self.input_mode = InputMode::Insert;
    }

    /// Open the search modal dialog
    pub fn open_search_modal(&mut self) {
        self.modal_state = ModalState::Search(Box::default());
        self.input_mode = InputMode::Insert;
    }

    /// Close any open modal dialog
    pub fn close_modal(&mut self) {
        self.modal_state = ModalState::None;
        self.input_mode = InputMode::Normal;
        self.escape_seq_state = EscapeSequenceState::None;
    }

    /// Perform a search with the current query in the search modal
    pub fn perform_search(&mut self) {
        if let ModalState::Search(ref mut state) = self.modal_state {
            let query = state.search_query();
            let results = self.search_engine.search(&query, &self.groups);
            state.set_results(results);
        }
    }

    /// Navigate to a conversation by session_id (from search results)
    pub fn navigate_to_conversation(&mut self, session_id: &str) -> Result<bool> {
        // Close search modal
        self.modal_state = ModalState::None;

        // Find the conversation in groups
        for group in &self.groups {
            for (conv_idx, conv) in group.conversations().iter().enumerate() {
                if conv.session_id == session_id {
                    // Expand the group
                    self.sidebar_state.collapsed_groups.remove(&group.key());

                    // Find the conversation in sidebar items
                    let items = self.sidebar_items();
                    let group_key = group.key();
                    if let Some(item_idx) = items.iter().position(|item| {
                        matches!(item, SidebarItem::Conversation { group_key: gk, index }
                            if gk == &group_key && *index == conv_idx)
                    }) {
                        self.sidebar_state.list_state.select(Some(item_idx));
                        self.update_selected_conversation();

                        // Open the conversation
                        self.open_selected()?;
                        return Ok(true);
                    }
                }
            }
        }

        self.toast_error("Conversation not found");
        Ok(false)
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
        self.focus = Focus::Terminal(TerminalPaneId::Primary);
        self.enter_insert_mode();

        Ok(())
    }

    /// Show an info toast
    pub fn toast_info(&mut self, message: impl Into<String>) {
        self.toast_manager.push(message, ToastType::Info);
    }

    /// Show a success toast
    pub fn toast_success(&mut self, message: impl Into<String>) {
        self.toast_manager.push(message, ToastType::Success);
    }

    /// Show a warning toast
    pub fn toast_warning(&mut self, message: impl Into<String>) {
        self.toast_manager.push(message, ToastType::Warning);
    }

    /// Show an error toast
    pub fn toast_error(&mut self, message: impl Into<String>) {
        self.toast_manager.push(message, ToastType::Error);
    }

    // =========================================================================
    // Bookmark Methods
    // =========================================================================

    /// Jump to a bookmark by slot (1-9)
    /// Returns true if successful, false if bookmark doesn't exist or target not found
    pub fn jump_to_bookmark(&mut self, slot: u8) -> Result<bool> {
        let bookmark = match self.bookmark_manager.get(slot) {
            Some(b) => b.clone(),
            None => {
                self.toast_error(format!("No bookmark at slot {}", slot));
                return Ok(false);
            }
        };

        match &bookmark.target {
            BookmarkTarget::Project { group_key, .. } => {
                self.jump_to_group(group_key, &bookmark.name)
            }
            BookmarkTarget::Conversation {
                session_id,
                group_key,
                ..
            } => self.jump_to_conversation(group_key, session_id, &bookmark.name),
        }
    }

    /// Jump to a group by key
    fn jump_to_group(&mut self, group_key: &str, bookmark_name: &str) -> Result<bool> {
        // Find the group
        let group_idx = self.groups.iter().position(|g| g.key() == group_key);

        if let Some(_idx) = group_idx {
            // Ensure group is expanded
            self.sidebar_state.collapsed_groups.remove(group_key);

            // Find the group header in sidebar items
            let items = self.sidebar_items();
            if let Some(item_idx) = items.iter().position(
                |item| matches!(item, SidebarItem::GroupHeader { key, .. } if key == group_key),
            ) {
                self.sidebar_state.list_state.select(Some(item_idx));
                self.update_selected_conversation();
                self.toast_success(format!("Jumped to '{}'", bookmark_name));
                return Ok(true);
            }
        }

        self.toast_error(format!("Project '{}' no longer exists", bookmark_name));
        Ok(false)
    }

    /// Jump to a specific conversation
    fn jump_to_conversation(
        &mut self,
        group_key: &str,
        session_id: &str,
        bookmark_name: &str,
    ) -> Result<bool> {
        // Find the group
        let group_idx = self.groups.iter().position(|g| g.key() == group_key);

        if let Some(gidx) = group_idx {
            // Ensure group is expanded
            self.sidebar_state.collapsed_groups.remove(group_key);

            // Find the conversation within the group
            let group = &self.groups[gidx];
            let conv_idx = group
                .conversations()
                .iter()
                .position(|c| c.session_id == session_id);

            if let Some(cidx) = conv_idx {
                // Find the conversation in sidebar items
                let items = self.sidebar_items();
                if let Some(item_idx) = items.iter().position(|item| {
                    matches!(item, SidebarItem::Conversation { group_key: gk, index }
                        if gk == group_key && *index == cidx)
                }) {
                    self.sidebar_state.list_state.select(Some(item_idx));
                    self.update_selected_conversation();
                    self.toast_success(format!("Jumped to '{}'", bookmark_name));
                    return Ok(true);
                }
            }
        }

        self.toast_error(format!("Conversation '{}' no longer exists", bookmark_name));
        Ok(false)
    }

    /// Bookmark the currently selected item to the given slot
    /// Returns true if successful
    pub fn bookmark_current(&mut self, slot: u8) -> Result<bool> {
        if slot < 1 || slot > 9 {
            self.toast_error("Bookmark slot must be 1-9");
            return Ok(false);
        }

        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        let bookmark = match items.get(selected) {
            Some(SidebarItem::GroupHeader { key, .. }) => {
                // Bookmark the group/project
                self.create_group_bookmark(slot, key)
            }
            Some(SidebarItem::Conversation { group_key, index }) => {
                // Bookmark the specific conversation
                self.create_conversation_bookmark(slot, group_key, *index)
            }
            Some(SidebarItem::EphemeralSession {
                session_id: _,
                group_key: _,
            }) => {
                // Can't bookmark ephemeral sessions (they don't have stable IDs)
                self.toast_warning("Cannot bookmark new conversations until they're saved");
                return Ok(false);
            }
            _ => {
                self.toast_warning("Cannot bookmark this item");
                return Ok(false);
            }
        };

        if let Some(bookmark) = bookmark {
            let name = bookmark.name.clone();
            let replaced = self.bookmark_manager.has_slot(slot);
            self.bookmark_manager.set(bookmark)?;

            if replaced {
                self.toast_success(format!("Replaced bookmark {} with '{}'", slot, name));
            } else {
                self.toast_success(format!("Bookmarked '{}' to slot {}", name, slot));
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Create a bookmark for a group
    fn create_group_bookmark(&self, slot: u8, group_key: &str) -> Option<Bookmark> {
        for group in &self.groups {
            if &group.key() == group_key {
                let project_path = group.project_path()?;
                let name = group.display_name();

                return Some(Bookmark::new_project(
                    slot,
                    name,
                    project_path,
                    group_key.to_string(),
                ));
            }
        }
        None
    }

    /// Create a bookmark for a conversation
    fn create_conversation_bookmark(
        &self,
        slot: u8,
        group_key: &str,
        index: usize,
    ) -> Option<Bookmark> {
        for group in &self.groups {
            if &group.key() == group_key {
                if let Some(conv) = group.conversations().get(index) {
                    let name = conv
                        .summary
                        .clone()
                        .unwrap_or_else(|| conv.display.clone())
                        .chars()
                        .take(30)
                        .collect();

                    return Some(Bookmark::new_conversation(
                        slot,
                        name,
                        conv.session_id.clone(),
                        conv.project_path.clone(),
                        group_key.to_string(),
                    ));
                }
            }
        }
        None
    }

    /// Remove a bookmark from the given slot
    /// Returns true if a bookmark was removed
    pub fn remove_bookmark(&mut self, slot: u8) -> Result<bool> {
        if slot < 1 || slot > 9 {
            self.toast_error("Bookmark slot must be 1-9");
            return Ok(false);
        }

        let removed = self.bookmark_manager.remove(slot)?;
        if removed {
            self.toast_success(format!("Removed bookmark from slot {}", slot));
        } else {
            self.toast_warning(format!("No bookmark at slot {}", slot));
        }
        Ok(removed)
    }

    // =========================================================================
    // Archive Methods
    // =========================================================================

    /// Archive the currently selected conversation
    /// Only works for conversations that are Idle (closed/not running)
    pub fn archive_selected_conversation(&mut self) -> Result<bool> {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                // Find the conversation
                let mut conv_info: Option<(String, ConversationStatus, bool)> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            conv_info = Some((
                                conv.session_id.clone(),
                                conv.status,
                                conv.is_archived,
                            ));
                            break;
                        }
                    }
                }

                if let Some((session_id, status, is_archived)) = conv_info {
                    // Check if already archived
                    if is_archived {
                        self.toast_warning("Already archived");
                        return Ok(false);
                    }

                    // Check if the conversation is running
                    let is_running = self.running_session_ids().contains(&session_id);
                    if is_running || status != ConversationStatus::Idle {
                        self.toast_warning("Cannot archive active conversation");
                        return Ok(false);
                    }

                    // Archive the conversation
                    self.archive_manager.archive(&session_id, false);

                    // Update the conversation's is_archived flag in groups
                    for group in &mut self.groups {
                        if &group.key() == group_key {
                            for conv in group.conversations_mut() {
                                if conv.session_id == session_id {
                                    conv.is_archived = true;
                                    conv.archived_at = Some(chrono::Utc::now());
                                    break;
                                }
                            }
                            break;
                        }
                    }

                    // Save archive state
                    if let Err(e) = self.archive_manager.save() {
                        self.toast_error(format!("Failed to save archive: {}", e));
                        return Err(e);
                    }

                    self.toast_success("Conversation archived");
                    return Ok(true);
                }
            }
            Some(SidebarItem::EphemeralSession { .. }) => {
                self.toast_warning("Cannot archive unsaved conversation");
            }
            _ => {
                self.toast_warning("Select a conversation to archive");
            }
        }

        Ok(false)
    }

    /// Unarchive the currently selected conversation
    pub fn unarchive_selected_conversation(&mut self) -> Result<bool> {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                // Find the conversation
                let mut conv_info: Option<(String, bool)> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            conv_info = Some((conv.session_id.clone(), conv.is_archived));
                            break;
                        }
                    }
                }

                if let Some((session_id, is_archived)) = conv_info {
                    // Check if not archived
                    if !is_archived {
                        self.toast_warning("Not archived");
                        return Ok(false);
                    }

                    // Unarchive the conversation
                    self.archive_manager.unarchive(&session_id);

                    // Update the conversation's is_archived flag in groups
                    for group in &mut self.groups {
                        if &group.key() == group_key {
                            for conv in group.conversations_mut() {
                                if conv.session_id == session_id {
                                    conv.is_archived = false;
                                    conv.archived_at = None;
                                    break;
                                }
                            }
                            break;
                        }
                    }

                    // Save archive state
                    if let Err(e) = self.archive_manager.save() {
                        self.toast_error(format!("Failed to save archive: {}", e));
                        return Err(e);
                    }

                    self.toast_success("Conversation unarchived");
                    return Ok(true);
                }
            }
            _ => {
                self.toast_warning("Select a conversation to unarchive");
            }
        }

        Ok(false)
    }

    /// Cycle the archive filter mode (Active -> Archived -> All -> Active)
    pub fn cycle_archive_filter(&mut self) {
        self.sidebar_state.cycle_archive_filter();
        let mode = match self.sidebar_state.archive_filter {
            crate::ui::sidebar::ArchiveFilter::Active => "active",
            crate::ui::sidebar::ArchiveFilter::Archived => "archived",
            crate::ui::sidebar::ArchiveFilter::All => "all",
        };
        self.toast_info(format!("Showing {} conversations", mode));
    }

    /// Check for conversations that should be auto-archived
    /// Archives Idle conversations older than the configured threshold
    pub fn check_auto_archive(&mut self) {
        // Collect sessions that need to be archived
        let mut to_archive: Vec<String> = Vec::new();

        for group in &self.groups {
            for conv in group.conversations() {
                // Skip if already archived or if running
                if conv.is_archived {
                    continue;
                }
                if self.running_session_ids().contains(&conv.session_id) {
                    continue;
                }
                if conv.status != ConversationStatus::Idle {
                    continue;
                }

                // Check if conversation should be auto-archived based on age
                if self.archive_manager.should_auto_archive(conv.timestamp) {
                    to_archive.push(conv.session_id.clone());
                }
            }
        }

        // Archive the conversations
        for session_id in &to_archive {
            self.archive_manager.archive(session_id, true);

            // Update the conversation's is_archived flag in groups
            for group in &mut self.groups {
                for conv in group.conversations_mut() {
                    if &conv.session_id == session_id {
                        conv.is_archived = true;
                        conv.archived_at = Some(chrono::Utc::now());
                        break;
                    }
                }
            }
        }

        // Save if we archived anything
        if !to_archive.is_empty() {
            if let Err(e) = self.archive_manager.save() {
                // Log error but don't show toast for auto-archive
                eprintln!("Failed to save auto-archive state: {}", e);
            }
        }
    }

    // =========================================================================
    // Layout Methods
    // =========================================================================

    /// Adjust sidebar width by delta percentage (positive = wider, negative = narrower)
    /// Clamps to valid range (10-50%) and resizes all sessions
    pub fn resize_sidebar(&mut self, delta: i8) {
        let current = self.config.layout.sidebar_width_pct as i16;
        let new_width = (current + delta as i16).clamp(10, 50) as u8;

        if new_width != self.config.layout.sidebar_width_pct {
            self.config.layout.sidebar_width_pct = new_width;
            self.save_config_silent();
            self.resize_sessions_to_layout();
            self.toast_info(format!("Sidebar: {}%", new_width));
        }
    }

    /// Toggle sidebar position between left and right
    pub fn toggle_sidebar_position(&mut self) {
        self.config.layout.sidebar_position = self.config.layout.sidebar_position.toggle();
        self.save_config_silent();
        let pos = match self.config.layout.sidebar_position {
            SidebarPosition::Left => "left",
            SidebarPosition::Right => "right",
        };
        self.toast_info(format!("Sidebar: {}", pos));
    }

    /// Toggle sidebar minimized state
    pub fn toggle_sidebar_minimized(&mut self) {
        self.config.layout.sidebar_minimized = !self.config.layout.sidebar_minimized;
        self.save_config_silent();
        self.resize_sessions_to_layout();
        if self.config.layout.sidebar_minimized {
            self.toast_info("Sidebar minimized");
        } else {
            self.toast_info("Sidebar restored");
        }
    }

    /// Resize all sessions to match current layout config
    fn resize_sessions_to_layout(&mut self) {
        let (rows, cols) = self.calculate_terminal_dimensions();
        for session_id in self.session_manager.session_ids() {
            if let Some(session) = self.session_manager.get_session_mut(&session_id) {
                let _ = session.resize(rows, cols);
            }
        }
    }

    /// Save config silently (don't show error toast, just log)
    fn save_config_silent(&self) {
        if let Err(e) = self.config.save() {
            eprintln!("Failed to save config: {}", e);
        }
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
