//! Application state and core data types for claudatui.

mod actions;
mod navigation;
mod panes;
mod sessions;
mod state;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use crossterm::event::KeyEvent;

use ratatui::layout::Rect;

use crate::claude::archive::ArchiveManager;
use crate::claude::conversation::Conversation;
use crate::claude::grouping::{
    group_conversations, group_conversations_unordered, order_groups_by_keys,
    retain_existing_groups, ConversationGroup,
};
use crate::claude::sessions::{parse_all_sessions, SessionEntry};
use crate::claude::SessionsWatcher;
use crate::config::{Config, SidebarPosition};
use crate::input::which_key::WhichKeyConfig;
use crate::input::{InputMode, LeaderState};
use crate::search::SearchEngine;
use crate::session::{ScreenState, SessionManager, SessionState};
use crate::ui::modal::{
    Modal, NewProjectModalState, SearchModalState, WorkspaceModalState, WorktreeModalState,
    WorktreeSearchModalState,
};
use crate::ui::sidebar::{
    build_sidebar_items, group_has_active_content, SidebarContext, SidebarItem, SidebarState,
};
use crate::ui::toast::{ToastManager, ToastType};

// Re-export all public types from submodules
pub use state::{
    ArchiveStatus, ClipboardStatus, EphemeralSession, PaneConfig, SplitMode, TerminalPaneId,
    TerminalPosition, TextSelection,
};

/// Modal dialog state
pub enum ModalState {
    /// No modal is open
    None,
    /// New project modal is open
    NewProject(Box<NewProjectModalState>),
    /// Search modal is open
    Search(Box<SearchModalState>),
    /// Worktree creation modal is open
    Worktree(Box<WorktreeModalState>),
    /// Worktree search modal (project picker + branch input)
    WorktreeSearch(Box<WorktreeSearchModalState>),
    /// Workspace management modal
    Workspace(Box<WorkspaceModalState>),
}

impl ModalState {
    /// Get a mutable reference to the inner modal via trait dispatch.
    pub fn as_modal_mut(&mut self) -> Option<&mut dyn Modal> {
        match self {
            ModalState::None => None,
            ModalState::NewProject(state) => Some(state.as_mut()),
            ModalState::Search(state) => Some(state.as_mut()),
            ModalState::Worktree(state) => Some(state.as_mut()),
            ModalState::WorktreeSearch(state) => Some(state.as_mut()),
            ModalState::Workspace(state) => Some(state.as_mut()),
        }
    }
}

/// Which UI pane currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    /// Sidebar has focus
    #[default]
    Sidebar,
    /// A terminal pane has focus
    Terminal(TerminalPaneId),
    /// Mosaic grid view has focus
    Mosaic,
}

/// State for tracking jk/kj rapid-press escape sequence in insert mode
#[derive(Default)]
pub enum EscapeSequenceState {
    /// No pending escape key
    #[default]
    None,
    /// First key of a potential escape sequence has been pressed
    Pending {
        first_key: char,
        first_key_event: KeyEvent,
        started_at: Instant,
    },
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
    #[expect(dead_code, reason = "planned for future use")]
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
    /// Whether the help menu overlay is open (toggled by '?')
    pub help_menu_open: bool,
    /// Index of the selected pane in mosaic grid view
    pub mosaic_selected: usize,
    /// Cached session states for mosaic rendering (session_id, display_name, state)
    pub mosaic_state_cache: Vec<(String, String, SessionState)>,
}

impl App {
    /// Create a new application instance
    pub fn new() -> Result<Self> {
        let claude_dir = dirs::home_dir()
            .context("Could not find home directory")?
            .join(".claude");

        // Create sessions watcher (optional - app works without it)
        let sessions_watcher = SessionsWatcher::new(&claude_dir).ok();

        // Create archive manager
        let archive_manager =
            ArchiveManager::new(&claude_dir).context("Failed to create archive manager")?;

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
            archive_manager,
            archive_status: ArchiveStatus::None,
            search_engine,
            config,
            input_mode: InputMode::default(),
            which_key_config: WhichKeyConfig::new(),
            escape_seq_state: EscapeSequenceState::None,
            text_selection: None,
            terminal_inner_area: None,
            help_menu_open: false,
            mosaic_selected: 0,
            mosaic_state_cache: Vec::new(),
        };

        app.load_conversations_full()?;
        app.check_auto_archive();

        Ok(app)
    }

    /// Load conversations with full re-sort (initial load and manual refresh).
    /// Groups are sorted by most recent activity.
    pub(crate) fn load_conversations_full(&mut self) -> Result<()> {
        let sessions = parse_all_sessions(&self.claude_dir)?;
        let conversations = self.sessions_to_conversations(sessions);
        let mut groups = group_conversations(conversations);
        retain_existing_groups(&mut groups);
        self.groups = groups;
        self.group_order = self.groups.iter().map(ConversationGroup::key).collect();
        Ok(())
    }

    /// Load conversations preserving existing group order (auto-refresh).
    /// New groups appear at the front, existing groups maintain their position.
    pub(crate) fn load_conversations_preserve_order(&mut self) -> Result<()> {
        let sessions = parse_all_sessions(&self.claude_dir)?;
        let conversations = self.sessions_to_conversations(sessions);
        let mut groups = group_conversations_unordered(conversations);
        retain_existing_groups(&mut groups);
        let (ordered_groups, updated_order) = order_groups_by_keys(groups, &self.group_order);
        self.groups = ordered_groups;
        self.group_order = updated_order;
        Ok(())
    }

    /// Convert SessionEntry list to Conversation list
    fn sessions_to_conversations(&self, sessions: Vec<SessionEntry>) -> Vec<Conversation> {
        sessions
            .into_iter()
            .map(|session| {
                // Detect plan implementation conversations by checking first_prompt
                let is_plan_implementation = session
                    .first_prompt
                    .starts_with("Implement the following plan:");

                // Check archive status
                let is_archived = self.archive_manager.is_archived(&session.session_id);
                let archived_at = self.archive_manager.get_archived_at(&session.session_id);

                Conversation {
                    session_id: session.session_id,
                    display: session.summary.clone().unwrap_or_else(|| {
                        let stripped = session
                            .first_prompt
                            .strip_prefix("Implement the following plan:\n")
                            .or_else(|| {
                                session
                                    .first_prompt
                                    .strip_prefix("Implement the following plan:")
                            })
                            .unwrap_or(&session.first_prompt)
                            .trim();
                        stripped.strip_prefix("# ").unwrap_or(stripped).to_string()
                    }),
                    summary: session.summary,
                    timestamp: session.file_mtime,
                    modified: session.modified,
                    project_path: PathBuf::from(&session.project_path),
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
        let running = self.running_session_ids();
        let ctx = SidebarContext {
            groups: &self.groups,
            running_sessions: &running,
            ephemeral_sessions: &self.ephemeral_sessions,
            hide_inactive: self.sidebar_state.hide_inactive,
            archive_filter: self.sidebar_state.archive_filter,
            filter_query: &self.sidebar_state.filter_query,
            filter_active: self.sidebar_state.filter_active,
            filter_cursor_pos: self.sidebar_state.filter_cursor_pos,
            workspaces: &self.config.workspaces,
        };
        build_sidebar_items(
            &ctx,
            &self.sidebar_state.collapsed_groups,
            &self.sidebar_state.collapsed_projects,
            &self.sidebar_state.visible_conversations,
            &self.sidebar_state.visible_groups,
            self.sidebar_state.other_collapsed,
        )
    }

    /// Set focus directly to a specific pane
    pub fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
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
            if state.path.is_empty() && state.is_expired(self.which_key_config.timeout_ms) {
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

    /// Enter insert mode and focus the terminal pane
    pub fn enter_insert_mode(&mut self) {
        self.input_mode = InputMode::Insert;
        self.escape_seq_state = EscapeSequenceState::None;
        self.focus = Focus::Terminal(TerminalPaneId::Primary);
    }

    /// Exit insert mode, focus the sidebar, and sync sidebar cursor to active session
    pub fn exit_insert_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.escape_seq_state = EscapeSequenceState::None;
        self.focus = Focus::Sidebar;
        self.select_sidebar_for_active_session();
    }
}

/// Extract text from the screen state within a selection range.
/// Trims trailing whitespace per line and joins with newlines.
pub fn extract_selected_text(screen: &ScreenState, selection: &TextSelection) -> String {
    let (start, end) = selection.ordered();
    let mut lines: Vec<String> = Vec::new();

    for row_idx in start.row..=end.row {
        if row_idx >= screen.rows.len() {
            break;
        }
        let row = &screen.rows[row_idx];
        let col_start = if row_idx == start.row { start.col } else { 0 };
        let col_end = if row_idx == end.row {
            end.col
        } else {
            row.cells.len().saturating_sub(1)
        };

        let mut line = String::new();
        for col_idx in col_start..=col_end {
            if col_idx < row.cells.len() {
                let cell = &row.cells[col_idx];
                if cell.contents.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(&cell.contents);
                }
            }
        }
        lines.push(line.trim_end().to_string());
    }

    // Remove trailing empty lines
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    lines.join("\n")
}
