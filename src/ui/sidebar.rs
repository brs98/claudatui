use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget},
};

use crossterm::event::{KeyCode, KeyEvent};

use crate::app::EphemeralSession;
use crate::bookmarks::BookmarkManager;
use crate::claude::conversation::ConversationStatus;
use crate::claude::grouping::ConversationGroup;

/// Default number of projects shown before "Show more" appears
const DEFAULT_VISIBLE_PROJECTS: usize = 5;

/// Default number of conversations shown per project before "Show more" appears
const DEFAULT_VISIBLE_CONVERSATIONS: usize = 3;

/// Archive filter modes for the sidebar
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchiveFilter {
    #[default]
    Active, // Show only non-archived
    Archived, // Show only archived
    All,      // Show both
}

/// Sidebar widget state
#[derive(Default)]
pub struct SidebarState {
    pub list_state: ListState,
    pub collapsed_groups: std::collections::HashSet<String>,
    pub show_all_projects: bool,
    /// Group keys that have all conversations expanded (not limited to DEFAULT_VISIBLE_CONVERSATIONS)
    pub expanded_conversations: std::collections::HashSet<String>,
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
    pub fn new() -> Self {
        let mut state = Self::default();
        state.list_state.select(Some(0));
        state
    }

    pub fn toggle_group(&mut self, group_key: &str) {
        if self.collapsed_groups.contains(group_key) {
            self.collapsed_groups.remove(group_key);
        } else {
            self.collapsed_groups.insert(group_key.to_string());
        }
    }

    pub fn toggle_show_all_projects(&mut self) {
        self.show_all_projects = !self.show_all_projects;
    }

    pub fn toggle_expanded_conversations(&mut self, group_key: &str) {
        if self.expanded_conversations.contains(group_key) {
            self.expanded_conversations.remove(group_key);
        } else {
            self.expanded_conversations.insert(group_key.to_string());
        }
    }

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

    /// Activate the inline filter input (enter insert mode on filter)
    pub fn activate_filter(&mut self) {
        self.filter_active = true;
        self.filter_cursor_pos = self.filter_query.len();
    }

    /// Deactivate the filter input (keep text visible but stop accepting keystrokes)
    pub fn deactivate_filter(&mut self) {
        self.filter_active = false;
    }

    /// Clear the filter entirely (text, cursor, active state)
    pub fn clear_filter(&mut self) {
        self.filter_query.clear();
        self.filter_cursor_pos = 0;
        self.filter_active = false;
    }

    /// Whether there is a non-empty filter query
    pub fn has_filter(&self) -> bool {
        !self.filter_query.is_empty()
    }

    /// Handle a key event while the filter input is active
    pub fn handle_filter_key(&mut self, key: KeyEvent) -> FilterKeyResult {
        match key.code {
            KeyCode::Char(c) => {
                self.filter_query.insert(self.filter_cursor_pos, c);
                self.filter_cursor_pos += 1;
                FilterKeyResult::QueryChanged
            }
            KeyCode::Backspace => {
                if self.filter_cursor_pos > 0 {
                    self.filter_cursor_pos -= 1;
                    self.filter_query.remove(self.filter_cursor_pos);
                    FilterKeyResult::QueryChanged
                } else {
                    FilterKeyResult::Continue
                }
            }
            KeyCode::Delete => {
                if self.filter_cursor_pos < self.filter_query.len() {
                    self.filter_query.remove(self.filter_cursor_pos);
                    FilterKeyResult::QueryChanged
                } else {
                    FilterKeyResult::Continue
                }
            }
            KeyCode::Left => {
                self.filter_cursor_pos = self.filter_cursor_pos.saturating_sub(1);
                FilterKeyResult::Continue
            }
            KeyCode::Right => {
                self.filter_cursor_pos = (self.filter_cursor_pos + 1).min(self.filter_query.len());
                FilterKeyResult::Continue
            }
            KeyCode::Home => {
                self.filter_cursor_pos = 0;
                FilterKeyResult::Continue
            }
            KeyCode::End => {
                self.filter_cursor_pos = self.filter_query.len();
                FilterKeyResult::Continue
            }
            KeyCode::Enter => FilterKeyResult::Deactivated,
            _ => FilterKeyResult::Continue,
        }
    }
}

/// Result of processing a key in the filter input
pub enum FilterKeyResult {
    /// No visual change needed
    Continue,
    /// Query text changed — re-filter and reset selection
    QueryChanged,
    /// Enter pressed — exit insert mode, keep filter text visible
    Deactivated,
}

/// Sidebar widget for displaying conversations
pub struct Sidebar<'a> {
    groups: &'a [ConversationGroup],
    focused: bool,
    /// Session IDs that are currently running (have active PTYs)
    running_sessions: &'a std::collections::HashSet<String>,
    /// Ephemeral sessions: temp session_id -> session info
    ephemeral_sessions: &'a HashMap<String, EphemeralSession>,
    /// Whether to hide inactive (Idle) sessions
    hide_inactive: bool,
    /// Archive filter mode
    archive_filter: ArchiveFilter,
    /// Bookmark manager for displaying bookmarks
    bookmark_manager: &'a BookmarkManager,
    /// Current filter query text (empty = no filter)
    filter_query: &'a str,
    /// Whether the filter input is actively accepting keystrokes
    filter_active: bool,
    /// Cursor position within the filter input (only used when filter_active)
    filter_cursor_pos: usize,
}

impl<'a> Sidebar<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        groups: &'a [ConversationGroup],
        focused: bool,
        running_sessions: &'a std::collections::HashSet<String>,
        ephemeral_sessions: &'a HashMap<String, EphemeralSession>,
        hide_inactive: bool,
        archive_filter: ArchiveFilter,
        bookmark_manager: &'a BookmarkManager,
        filter_query: &'a str,
        filter_active: bool,
        filter_cursor_pos: usize,
    ) -> Self {
        Self {
            groups,
            focused,
            running_sessions,
            ephemeral_sessions,
            hide_inactive,
            archive_filter,
            bookmark_manager,
            filter_query,
            filter_active,
            filter_cursor_pos,
        }
    }
}

impl<'a> StatefulWidget for Sidebar<'a> {
    type State = SidebarState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // Get title based on hide_inactive and archive filter
        let title = match (self.hide_inactive, self.archive_filter) {
            (true, ArchiveFilter::Active) => " Conversations (active) ",
            (false, ArchiveFilter::Active) => " Conversations ",
            (_, ArchiveFilter::Archived) => " Conversations (archived) ",
            (_, ArchiveFilter::All) => " Conversations (all) ",
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner_area = block.inner(area);
        block.render(area, buf);

        // Split inner area: filter row at top (when visible), list below
        let show_filter_row = self.filter_active || !self.filter_query.is_empty();
        let (filter_area, list_area) = if show_filter_row && inner_area.height > 1 {
            (
                Rect {
                    height: 1,
                    ..inner_area
                },
                Rect {
                    y: inner_area.y + 1,
                    height: inner_area.height - 1,
                    ..inner_area
                },
            )
        } else {
            (Rect::default(), inner_area)
        };

        // Render filter input row
        if show_filter_row && filter_area.height > 0 {
            render_filter_row(
                filter_area,
                buf,
                self.filter_query,
                self.filter_active,
                self.filter_cursor_pos,
            );
        }

        let selected_index = state.list_state.selected();
        let items = build_list_items(
            self.groups,
            &state.collapsed_groups,
            state.show_all_projects,
            &state.expanded_conversations,
            self.running_sessions,
            self.ephemeral_sessions,
            self.hide_inactive,
            self.archive_filter,
            selected_index,
            self.bookmark_manager,
            self.filter_query,
        );
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        StatefulWidget::render(list, list_area, buf, &mut state.list_state);
    }
}

/// Render the inline filter input row at the top of the sidebar
fn render_filter_row(area: Rect, buf: &mut Buffer, query: &str, active: bool, cursor_pos: usize) {
    let prefix = "/ ";
    let prefix_len = prefix.len();

    if active {
        // Active: show prefix + query with cursor highlight
        let mut spans = vec![Span::styled(prefix, Style::default().fg(Color::Yellow))];

        let available_width = (area.width as usize).saturating_sub(prefix_len);
        let (visible_text, cursor_offset) = if query.len() <= available_width {
            (query, cursor_pos)
        } else {
            let start = if cursor_pos >= available_width {
                cursor_pos - available_width + 1
            } else {
                0
            };
            let end = (start + available_width).min(query.len());
            (&query[start..end], cursor_pos - start)
        };

        for (i, c) in visible_text.chars().enumerate() {
            if i == cursor_offset {
                spans.push(Span::styled(
                    c.to_string(),
                    Style::default().bg(Color::White).fg(Color::Black),
                ));
            } else {
                spans.push(Span::raw(c.to_string()));
            }
        }
        // Block cursor at end of text
        if cursor_offset >= visible_text.len() {
            spans.push(Span::styled(" ", Style::default().bg(Color::White)));
        }

        Paragraph::new(Line::from(spans)).render(area, buf);
    } else {
        // Inactive (persistent display): dimmed style, no cursor
        let line = Line::from(vec![
            Span::styled(prefix, Style::default().fg(Color::DarkGray)),
            Span::styled(query, Style::default().fg(Color::DarkGray)),
        ]);
        Paragraph::new(line).render(area, buf);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_list_items(
    groups: &[ConversationGroup],
    collapsed: &std::collections::HashSet<String>,
    show_all_projects: bool,
    expanded_conversations: &std::collections::HashSet<String>,
    running_sessions: &std::collections::HashSet<String>,
    ephemeral_sessions: &HashMap<String, EphemeralSession>,
    hide_inactive: bool,
    archive_filter: ArchiveFilter,
    selected_index: Option<usize>,
    bookmark_manager: &BookmarkManager,
    filter_query: &str,
) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    let mut current_index: usize = 0;

    // Prepare case-insensitive filter query
    let filter_lower = filter_query.to_lowercase();
    let has_text_filter = !filter_lower.is_empty();

    // Render bookmarks section at the top
    let bookmarks = bookmark_manager.get_all();
    if !bookmarks.is_empty() {
        // Bookmark section header
        items.push(ListItem::new(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "Bookmarks",
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Cyan),
            ),
            Span::styled(" [b:edit]", Style::default().fg(Color::DarkGray)),
        ])));
        current_index += 1;

        // Individual bookmarks
        for bookmark in bookmarks {
            let slot = bookmark.slot;
            let name = truncate_string(&bookmark.name, 20);
            let line_num = format_relative_line_number(current_index, selected_index);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(
                    format!("[{}] ", slot),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(name),
            ])));
            current_index += 1;
        }

        // Separator between bookmarks and conversations
        items.push(ListItem::new(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("─".repeat(20), Style::default().fg(Color::DarkGray)),
        ])));
        current_index += 1;
    }

    let visible_groups = if show_all_projects || groups.len() <= DEFAULT_VISIBLE_PROJECTS {
        groups
    } else {
        &groups[..DEFAULT_VISIBLE_PROJECTS]
    };

    for group in visible_groups {
        // Skip groups with no active content when hide_inactive is enabled
        if hide_inactive && !group_has_active_content(group, running_sessions, ephemeral_sessions) {
            continue;
        }

        // When text filter is active, check if group name matches or any conversation matches
        let group_name_matches =
            has_text_filter && group.display_name().to_lowercase().contains(&filter_lower);

        // Check if group has any conversations visible with current archive + text filter
        let has_visible_conversations = group.conversations().iter().any(|conv| {
            !conv.is_plan_implementation
                && should_show_conversation(conv, archive_filter, running_sessions, hide_inactive)
                && (!has_text_filter
                    || group_name_matches
                    || conv_matches_filter(conv, &filter_lower))
        });

        // Skip groups with no visible conversations (unless showing all and no text filter)
        if !has_visible_conversations && (archive_filter != ArchiveFilter::All || has_text_filter) {
            continue;
        }

        let group_key = group.key();
        let is_collapsed = collapsed.contains(&group_key);

        // Check if this group is bookmarked
        let bookmark_slot = bookmark_manager.is_group_bookmarked(&group_key);
        let star_indicator = if let Some(slot) = bookmark_slot {
            Span::styled(format!(" [{}]", slot), Style::default().fg(Color::Yellow))
        } else {
            Span::raw("")
        };

        // Group header with "+" indicator for new chat
        let arrow = if is_collapsed { "▸" } else { "▾" };
        let header = format!("{} {}", arrow, group.display_name());
        let line_num = format_relative_line_number(current_index, selected_index);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(line_num, Style::default().fg(Color::DarkGray)),
            Span::styled(header, Style::default().add_modifier(Modifier::BOLD)),
            star_indicator,
            Span::styled(" +", Style::default().fg(Color::Green)),
        ])));
        current_index += 1;

        // Conversations and ephemeral sessions (if not collapsed)
        if !is_collapsed {
            // First, show ephemeral sessions for this group at the top
            // (ephemeral sessions are always shown - they're running by definition)
            let group_project_path = group.project_path();
            if let Some(project_path) = group_project_path {
                for (session_id, ephemeral) in ephemeral_sessions {
                    if ephemeral.project_path == project_path {
                        // Render ephemeral session with distinctive styling
                        let line_num = format_relative_line_number(current_index, selected_index);
                        items.push(ListItem::new(Line::from(vec![
                            Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                            Span::raw("  "),
                            Span::styled("● ", Style::default().fg(Color::Green)),
                            Span::styled(
                                format!(
                                    "New conversation ({})",
                                    &session_id[session_id.len().saturating_sub(1)..]
                                ),
                                Style::default().add_modifier(Modifier::ITALIC),
                            ),
                        ])));
                        current_index += 1;
                    }
                }
            }

            // Get all conversations and filter out plan implementations
            // Also filter by archive status, inactive, and text filter
            let conversations = group.conversations();
            let filtered_convos: Vec<_> = conversations
                .iter()
                .filter(|conv| !conv.is_plan_implementation)
                .filter(|conv| {
                    should_show_conversation(conv, archive_filter, running_sessions, hide_inactive)
                })
                .filter(|conv| {
                    !has_text_filter
                        || group_name_matches
                        || conv_matches_filter(conv, &filter_lower)
                })
                .collect();

            // Determine how many conversations to show (from filtered list)
            let is_expanded = expanded_conversations.contains(&group_key);
            let visible_convos: Vec<_> =
                if is_expanded || filtered_convos.len() <= DEFAULT_VISIBLE_CONVERSATIONS {
                    filtered_convos.to_vec()
                } else {
                    filtered_convos
                        .iter()
                        .take(DEFAULT_VISIBLE_CONVERSATIONS)
                        .copied()
                        .collect()
                };

            // Then show saved conversations (limited or all)
            for conv in &visible_convos {
                // If session is running in background, show it as Active
                // regardless of the file-based status
                let is_running = running_sessions.contains(&conv.session_id);
                let (status_indicator, archive_indicator) = if is_running {
                    (Span::styled("● ", Style::default().fg(Color::Green)), None)
                } else {
                    let status = match conv.status {
                        ConversationStatus::Active => {
                            Span::styled("● ", Style::default().fg(Color::Green))
                        }
                        ConversationStatus::WaitingForInput => {
                            Span::styled("◐ ", Style::default().fg(Color::Yellow))
                        }
                        ConversationStatus::Idle => {
                            Span::styled("○ ", Style::default().fg(Color::DarkGray))
                        }
                    };
                    // Show archive indicator when in "All" view
                    let archive = if archive_filter == ArchiveFilter::All && conv.is_archived {
                        Some(Span::styled("📦 ", Style::default().fg(Color::DarkGray)))
                    } else {
                        None
                    };
                    (status, archive)
                };

                let display = truncate_string(&conv.display, 30);
                let line_num = format_relative_line_number(current_index, selected_index);
                let mut line_parts = vec![
                    Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                ];

                // Add archive indicator if present (only in All view)
                if let Some(indicator) = archive_indicator {
                    line_parts.push(indicator);
                }

                line_parts.push(status_indicator);
                line_parts.push(Span::raw(display));

                items.push(ListItem::new(Line::from(line_parts)));
                current_index += 1;
            }

            // Add "show more conversations" if truncated (use filtered count)
            if !is_expanded && filtered_convos.len() > DEFAULT_VISIBLE_CONVERSATIONS {
                let hidden = filtered_convos.len() - DEFAULT_VISIBLE_CONVERSATIONS;
                let line_num = format_relative_line_number(current_index, selected_index);
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled(
                        format!("↓ Show {} more...", hidden),
                        Style::default().fg(Color::Blue),
                    ),
                ])));
                current_index += 1;
            }
        }
    }

    // Add "Show more" at end if truncated
    if !show_all_projects && groups.len() > DEFAULT_VISIBLE_PROJECTS {
        // When hide_inactive is enabled, count only hidden groups with active content
        let hidden_groups = &groups[DEFAULT_VISIBLE_PROJECTS..];
        let hidden = if hide_inactive {
            hidden_groups
                .iter()
                .filter(|g| group_has_active_content(g, running_sessions, ephemeral_sessions))
                .count()
        } else {
            hidden_groups.len()
        };
        if hidden > 0 {
            let line_num = format_relative_line_number(current_index, selected_index);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("↓ Show {} more projects...", hidden),
                    Style::default().fg(Color::Blue),
                ),
            ])));
        }
    }

    items
}

/// Format a relative line number for display
/// Returns "0 " for selected item, or distance from selection (e.g., "1 ", "2 ")
fn format_relative_line_number(index: usize, selected: Option<usize>) -> String {
    match selected {
        Some(sel) => {
            let distance = index.abs_diff(sel);
            format!("{:2} ", distance)
        }
        None => "   ".to_string(),
    }
}

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Check if a conversation matches the text filter (case-insensitive)
fn conv_matches_filter(
    conv: &crate::claude::conversation::Conversation,
    filter_lower: &str,
) -> bool {
    conv.display.to_lowercase().contains(filter_lower)
        || conv
            .summary
            .as_ref()
            .is_some_and(|s| s.to_lowercase().contains(filter_lower))
}

/// Check if a conversation should be shown based on archive filter and other criteria
fn should_show_conversation(
    conv: &crate::claude::conversation::Conversation,
    archive_filter: ArchiveFilter,
    running_sessions: &std::collections::HashSet<String>,
    hide_inactive: bool,
) -> bool {
    // First check archive filter
    let passes_archive_filter = match archive_filter {
        ArchiveFilter::Active => !conv.is_archived,
        ArchiveFilter::Archived => conv.is_archived,
        ArchiveFilter::All => true,
    };

    if !passes_archive_filter {
        return false;
    }

    // Then check hide_inactive filter
    if hide_inactive {
        let is_running = running_sessions.contains(&conv.session_id);
        return is_running
            || !matches!(
                conv.status,
                crate::claude::conversation::ConversationStatus::Idle
            );
    }

    true
}

/// Check if a group has any active content (for hide_inactive filtering)
pub fn group_has_active_content(
    group: &ConversationGroup,
    running_sessions: &std::collections::HashSet<String>,
    ephemeral_sessions: &HashMap<String, EphemeralSession>,
) -> bool {
    // Check for ephemeral sessions in this group
    if let Some(project_path) = group.project_path() {
        for ephemeral in ephemeral_sessions.values() {
            if ephemeral.project_path == project_path {
                return true;
            }
        }
    }

    // Check for active/running conversations (excluding plan implementations)
    for conv in group.conversations() {
        if conv.is_plan_implementation {
            continue;
        }
        let is_running = running_sessions.contains(&conv.session_id);
        if is_running || !matches!(conv.status, ConversationStatus::Idle) {
            return true;
        }
    }

    false
}

/// Represents an item in the flattened sidebar list
#[derive(Debug, Clone)]
pub enum SidebarItem {
    BookmarkHeader,
    BookmarkEntry {
        slot: u8,
    },
    BookmarkSeparator,
    GroupHeader {
        key: String,
        #[allow(dead_code)]
        name: String,
    },
    Conversation {
        group_key: String,
        index: usize,
    },
    /// A running session that hasn't been saved yet (temp session)
    EphemeralSession {
        session_id: String,
        group_key: String,
    },
    ShowMoreProjects {
        #[allow(dead_code)]
        hidden_count: usize,
    },
    ShowMoreConversations {
        group_key: String,
        #[allow(dead_code)]
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

/// Build a flat list of sidebar items for navigation
#[allow(clippy::too_many_arguments)]
pub fn build_sidebar_items(
    groups: &[ConversationGroup],
    collapsed: &std::collections::HashSet<String>,
    show_all_projects: bool,
    expanded_conversations: &std::collections::HashSet<String>,
    ephemeral_sessions: &HashMap<String, EphemeralSession>,
    running_sessions: &std::collections::HashSet<String>,
    hide_inactive: bool,
    archive_filter: ArchiveFilter,
    bookmark_manager: &BookmarkManager,
    filter_query: &str,
) -> Vec<SidebarItem> {
    let mut items = Vec::new();

    let filter_lower = filter_query.to_lowercase();
    let has_text_filter = !filter_lower.is_empty();

    // Insert bookmark items at the top, mirroring build_list_items
    let bookmarks = bookmark_manager.get_all();
    if !bookmarks.is_empty() {
        items.push(SidebarItem::BookmarkHeader);
        for bookmark in bookmarks {
            items.push(SidebarItem::BookmarkEntry {
                slot: bookmark.slot,
            });
        }
        items.push(SidebarItem::BookmarkSeparator);
    }

    let visible_groups = if show_all_projects || groups.len() <= DEFAULT_VISIBLE_PROJECTS {
        groups
    } else {
        &groups[..DEFAULT_VISIBLE_PROJECTS]
    };

    for group in visible_groups {
        // Skip groups with no active content when hide_inactive is enabled
        if hide_inactive && !group_has_active_content(group, running_sessions, ephemeral_sessions) {
            continue;
        }

        let group_key = group.key();

        // When text filter is active, check if group name matches
        let group_name_matches =
            has_text_filter && group.display_name().to_lowercase().contains(&filter_lower);

        // Check if group has any conversations visible with current archive + text filter
        let has_visible_conversations = group.conversations().iter().any(|conv| {
            !conv.is_plan_implementation
                && should_show_conversation(conv, archive_filter, running_sessions, hide_inactive)
                && (!has_text_filter
                    || group_name_matches
                    || conv_matches_filter(conv, &filter_lower))
        });

        // Skip groups with no visible conversations
        if !has_visible_conversations && (archive_filter != ArchiveFilter::All || has_text_filter) {
            continue;
        }

        items.push(SidebarItem::GroupHeader {
            key: group_key.clone(),
            name: group.display_name(),
        });

        if !collapsed.contains(&group_key) {
            // First, add ephemeral sessions for this group
            // (ephemeral sessions are always shown - they're running by definition)
            // But only in Active or All views (not in Archived view)
            if archive_filter != ArchiveFilter::Archived {
                let group_project_path = group.project_path();
                if let Some(project_path) = group_project_path {
                    for (session_id, ephemeral) in ephemeral_sessions {
                        if ephemeral.project_path == project_path {
                            items.push(SidebarItem::EphemeralSession {
                                session_id: session_id.clone(),
                                group_key: group_key.clone(),
                            });
                        }
                    }
                }
            }

            // Get all conversations and filter out plan implementations
            // Also filter by archive status, inactive, and text filter
            // We keep track of original indices so lookup in app.rs still works
            let conversations = group.conversations();
            let filtered_indices: Vec<usize> = conversations
                .iter()
                .enumerate()
                .filter(|(_, conv)| !conv.is_plan_implementation)
                .filter(|(_, conv)| {
                    should_show_conversation(conv, archive_filter, running_sessions, hide_inactive)
                })
                .filter(|(_, conv)| {
                    !has_text_filter
                        || group_name_matches
                        || conv_matches_filter(conv, &filter_lower)
                })
                .map(|(idx, _)| idx)
                .collect();

            // Determine how many conversations to show (from filtered list)
            let is_expanded = expanded_conversations.contains(&group_key);
            let visible_indices: Vec<usize> =
                if is_expanded || filtered_indices.len() <= DEFAULT_VISIBLE_CONVERSATIONS {
                    filtered_indices.clone()
                } else {
                    filtered_indices
                        .iter()
                        .take(DEFAULT_VISIBLE_CONVERSATIONS)
                        .copied()
                        .collect()
                };

            // Then add saved conversations (limited or all)
            for index in visible_indices {
                items.push(SidebarItem::Conversation {
                    group_key: group_key.clone(),
                    index,
                });
            }

            // Add "show more conversations" if truncated (use filtered count)
            if !is_expanded && filtered_indices.len() > DEFAULT_VISIBLE_CONVERSATIONS {
                items.push(SidebarItem::ShowMoreConversations {
                    group_key: group_key.clone(),
                    hidden_count: filtered_indices.len() - DEFAULT_VISIBLE_CONVERSATIONS,
                });
            }
        }
    }

    // Add "Show more" item if there are hidden projects
    if !show_all_projects && groups.len() > DEFAULT_VISIBLE_PROJECTS {
        // When hide_inactive is enabled, count only hidden groups with active content
        let hidden_groups = &groups[DEFAULT_VISIBLE_PROJECTS..];
        let hidden_count = if hide_inactive {
            hidden_groups
                .iter()
                .filter(|g| group_has_active_content(g, running_sessions, ephemeral_sessions))
                .count()
        } else {
            hidden_groups.len()
        };
        if hidden_count > 0 {
            items.push(SidebarItem::ShowMoreProjects { hidden_count });
        }
    }

    items
}
