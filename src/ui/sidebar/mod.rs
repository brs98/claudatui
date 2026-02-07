//! Sidebar widget for browsing conversation groups, bookmarks, and sessions.

mod filter;
mod items;
mod rendering;

use std::collections::{HashMap, HashSet};

use ratatui::widgets::ListState;

use crate::app::EphemeralSession;
use crate::bookmarks::BookmarkManager;
use crate::claude::grouping::ConversationGroup;

// Re-export public API
pub use filter::FilterKeyResult;
pub use items::{build_sidebar_items, group_has_active_content};
pub use rendering::Sidebar;

/// Default number of projects shown before "Show more" appears
const DEFAULT_VISIBLE_PROJECTS: usize = 5;

/// Default number of conversations shown per project before "Show more" appears
const DEFAULT_VISIBLE_CONVERSATIONS: usize = 3;

/// Bundles the common parameters shared across sidebar rendering functions.
pub struct SidebarContext<'a> {
    /// Conversation groups to display
    pub groups: &'a [ConversationGroup],
    /// Session IDs that are currently running (have active PTYs)
    pub running_sessions: &'a HashSet<String>,
    /// Ephemeral sessions: temp session_id -> session info
    pub ephemeral_sessions: &'a HashMap<String, EphemeralSession>,
    /// Whether to hide inactive (Idle) sessions
    pub hide_inactive: bool,
    /// Archive filter mode
    pub archive_filter: ArchiveFilter,
    /// Bookmark manager for displaying bookmarks
    pub bookmark_manager: &'a BookmarkManager,
    /// Current filter query text (empty = no filter)
    pub filter_query: &'a str,
    /// Whether the filter input is actively accepting keystrokes
    pub filter_active: bool,
    /// Cursor position within the filter input (only used when filter_active)
    pub filter_cursor_pos: usize,
}

/// Archive filter modes for the sidebar
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchiveFilter {
    /// Show only non-archived conversations
    #[default]
    Active,
    /// Show only archived conversations
    Archived,
    /// Show all conversations
    All,
}

/// Sidebar widget state tracking selection, collapsed groups, and filter state.
#[derive(Default)]
pub struct SidebarState {
    /// Ratatui list selection state
    pub list_state: ListState,
    /// Group keys that are collapsed (conversations hidden)
    pub collapsed_groups: HashSet<String>,
    /// Whether to show all projects or limit to recent ones
    pub show_all_projects: bool,
    /// Group keys that have all conversations expanded (not limited to DEFAULT_VISIBLE_CONVERSATIONS)
    pub expanded_conversations: HashSet<String>,
    /// When true, hide Idle sessions (only show Active, WaitingForInput, or running sessions)
    pub hide_inactive: bool,
    /// Current archive filter mode
    pub archive_filter: ArchiveFilter,
    /// Current inline filter query text
    pub filter_query: String,
    /// Cursor position within the filter input
    pub filter_cursor_pos: usize,
    /// Whether the filter input is actively accepting keystrokes
    pub filter_active: bool,
}

impl SidebarState {
    /// Create a new sidebar state with the first item selected.
    pub fn new() -> Self {
        let mut state = Self::default();
        state.list_state.select(Some(0));
        state
    }

    /// Toggle a group between collapsed and expanded.
    pub fn toggle_group(&mut self, group_key: &str) {
        if self.collapsed_groups.contains(group_key) {
            self.collapsed_groups.remove(group_key);
        } else {
            self.collapsed_groups.insert(group_key.to_string());
        }
    }

    /// Toggle between showing all projects and showing only recent ones.
    pub fn toggle_show_all_projects(&mut self) {
        self.show_all_projects = !self.show_all_projects;
    }

    /// Toggle between showing all conversations in a group and limiting to default count.
    pub fn toggle_expanded_conversations(&mut self, group_key: &str) {
        if self.expanded_conversations.contains(group_key) {
            self.expanded_conversations.remove(group_key);
        } else {
            self.expanded_conversations.insert(group_key.to_string());
        }
    }

    /// Toggle filtering to show only active/running sessions.
    pub fn toggle_hide_inactive(&mut self) {
        self.hide_inactive = !self.hide_inactive;
    }

    /// Cycle through archive filter modes: Active -> Archived -> All -> Active
    pub fn cycle_archive_filter(&mut self) {
        self.archive_filter = match self.archive_filter {
            ArchiveFilter::Active => ArchiveFilter::Archived,
            ArchiveFilter::Archived => ArchiveFilter::All,
            ArchiveFilter::All => ArchiveFilter::Active,
        };
    }

    /// Get the display title based on current filter
    pub fn get_title(&self, base_title: &str) -> String {
        match self.archive_filter {
            ArchiveFilter::Active => format!(" {} ", base_title),
            ArchiveFilter::Archived => format!(" {} (archived) ", base_title),
            ArchiveFilter::All => format!(" {} (all) ", base_title),
        }
    }
}

/// Represents an item in the flattened sidebar list
#[derive(Debug, Clone)]
pub enum SidebarItem {
    /// Header row for the bookmarks section
    BookmarkHeader,
    /// A single bookmark slot
    BookmarkEntry {
        /// Bookmark slot number (1-9)
        slot: u8,
    },
    /// Visual separator after bookmarks
    BookmarkSeparator,
    /// Header row for a project group
    GroupHeader {
        /// Unique group key for collapse/expand tracking
        key: String,
        /// Display name for the group
        name: String,
    },
    /// A conversation within a group
    Conversation {
        /// Key of the parent group
        group_key: String,
        /// Index into the group's conversation list
        index: usize,
    },
    /// A running session that hasn't been saved yet (temp session)
    EphemeralSession {
        /// Session identifier
        session_id: String,
        /// Key of the parent group
        group_key: String,
    },
    /// "Show N more projects" expandable row
    ShowMoreProjects {
        /// Number of hidden projects
        hidden_count: usize,
    },
    /// "Show N more conversations" expandable row within a group
    ShowMoreConversations {
        /// Key of the parent group
        group_key: String,
        /// Number of hidden conversations
        hidden_count: usize,
    },
}

impl SidebarItem {
    /// Whether this item can be selected/highlighted by the cursor.
    /// Non-interactive decorative items (headers, separators) return false.
    pub fn is_selectable(&self) -> bool {
        !matches!(
            self,
            SidebarItem::BookmarkHeader | SidebarItem::BookmarkSeparator
        )
    }
}
