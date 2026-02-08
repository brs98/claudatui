//! Sidebar widget for browsing conversation groups, workspaces, and sessions.

mod filter;
mod items;
mod rendering;

use std::collections::{HashMap, HashSet};

use ratatui::widgets::ListState;

use crate::app::EphemeralSession;
use crate::claude::grouping::ConversationGroup;

// Re-export public API
pub use filter::FilterKeyResult;
pub use items::{build_sidebar_items, group_has_active_content};
pub use rendering::Sidebar;

/// Number of items shown per page when expanding/collapsing sidebar sections.
pub const PAGE_SIZE: usize = 3;

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
    /// Current filter query text (empty = no filter)
    pub filter_query: &'a str,
    /// Whether the filter input is actively accepting keystrokes
    pub filter_active: bool,
    /// Cursor position within the filter input (only used when filter_active)
    pub filter_cursor_pos: usize,
    /// Workspace directory prefixes for filtering
    pub workspaces: &'a [String],
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
pub struct SidebarState {
    /// Ratatui list selection state
    pub list_state: ListState,
    /// Group keys that are collapsed (conversations hidden)
    pub collapsed_groups: HashSet<String>,
    /// Per-group visible conversation count (key absent = PAGE_SIZE default)
    pub visible_conversations: HashMap<String, usize>,
    /// Per-project visible group count (key absent = PAGE_SIZE default)
    pub visible_groups: HashMap<String, usize>,
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
    /// Whether the "Other" section is collapsed (groups hidden)
    pub other_collapsed: bool,
    /// Project keys that are collapsed (child groups hidden)
    pub collapsed_projects: HashSet<String>,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            list_state: ListState::default(),
            collapsed_groups: HashSet::default(),
            visible_conversations: HashMap::default(),
            visible_groups: HashMap::default(),
            hide_inactive: false,
            archive_filter: ArchiveFilter::default(),
            filter_query: String::new(),
            filter_cursor_pos: 0,
            filter_active: false,
            other_collapsed: true,
            collapsed_projects: HashSet::default(),
        }
    }
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

    /// Get the visible count for a key, defaulting to PAGE_SIZE.
    pub fn visible_count(map: &HashMap<String, usize>, key: &str) -> usize {
        map.get(key).copied().unwrap_or(PAGE_SIZE)
    }

    /// Increment visible count by PAGE_SIZE, clamping to total.
    pub fn show_more(map: &mut HashMap<String, usize>, key: &str, total: usize) {
        let current = Self::visible_count(map, key);
        let new_count = (current + PAGE_SIZE).min(total);
        map.insert(key.to_string(), new_count);
    }

    /// Set visible count to total (show all items).
    pub fn show_all(map: &mut HashMap<String, usize>, key: &str, total: usize) {
        map.insert(key.to_string(), total);
    }

    /// Decrement visible count by PAGE_SIZE, removing the key if at or below PAGE_SIZE.
    pub fn show_fewer(map: &mut HashMap<String, usize>, key: &str) {
        let current = Self::visible_count(map, key);
        if current <= PAGE_SIZE * 2 {
            // Would go to PAGE_SIZE or below — just remove (back to default)
            map.remove(key);
        } else {
            map.insert(key.to_string(), current - PAGE_SIZE);
        }
    }

    /// Reset to default PAGE_SIZE by removing the key.
    pub fn collapse_to_default(map: &mut HashMap<String, usize>, key: &str) {
        map.remove(key);
    }

    /// Toggle filtering to show only active/running sessions.
    pub fn toggle_hide_inactive(&mut self) {
        self.hide_inactive = !self.hide_inactive;
    }

    /// Toggle collapse state of the "Other" section.
    pub fn toggle_other_collapsed(&mut self) {
        self.other_collapsed = !self.other_collapsed;
    }

    /// Toggle collapse state of a project header.
    pub fn toggle_project(&mut self, project_key: &str) {
        if self.collapsed_projects.contains(project_key) {
            self.collapsed_projects.remove(project_key);
        } else {
            self.collapsed_projects.insert(project_key.to_string());
        }
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

/// Which kind of section a `SectionControl` operates on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    /// Controls visibility of conversations within a group.
    Conversations,
    /// Controls visibility of groups within a project.
    Groups,
}

/// Action that a `SectionControl` sidebar item triggers when activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlAction {
    /// Show PAGE_SIZE more items (param = remaining hidden count).
    ShowMore(usize),
    /// Show all items (param = total count).
    ShowAll(usize),
    /// Hide PAGE_SIZE items.
    ShowFewer,
    /// Reset to default PAGE_SIZE.
    Collapse,
}

/// Represents an item in the flattened sidebar list
#[derive(Debug, Clone)]
pub enum SidebarItem {
    /// Always-visible section header for workspaces
    WorkspaceSectionHeader,
    /// Collapsible project header grouping worktrees/directories
    ProjectHeader {
        /// Collapse tracking key (e.g., repo_path for worktrees)
        project_key: String,
        /// Display name for the project
        name: String,
        /// Number of child groups under this project
        group_count: usize,
    },
    /// Action item to add a new workspace
    AddWorkspace,
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
    /// Incremental expand/collapse control for a section (conversations or groups).
    SectionControl {
        /// Key identifying the section (group_key for conversations, project_key for groups).
        key: String,
        /// What kind of section this controls.
        kind: SectionKind,
        /// What action activating this item performs.
        action: ControlAction,
    },
    /// Header for "Other" section (non-workspace groups)
    OtherHeader {
        /// Number of non-workspace project groups
        group_count: usize,
    },
}

impl SidebarItem {
    /// Whether this item can be selected/highlighted by the cursor.
    /// Non-interactive decorative items (section headers) return false.
    pub fn is_selectable(&self) -> bool {
        !matches!(self, SidebarItem::WorkspaceSectionHeader)
    }
}
