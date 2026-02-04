use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, StatefulWidget, Widget},
};

use crate::claude::conversation::ConversationStatus;
use crate::claude::grouping::ConversationGroup;

/// Default number of projects shown before "Show more" appears
const DEFAULT_VISIBLE_PROJECTS: usize = 5;

/// Default number of conversations shown per project before "Show more" appears
const DEFAULT_VISIBLE_CONVERSATIONS: usize = 3;

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
}

/// Sidebar widget for displaying conversations
pub struct Sidebar<'a> {
    groups: &'a [ConversationGroup],
    focused: bool,
    /// Session IDs that are currently running (have active PTYs)
    running_sessions: &'a std::collections::HashSet<String>,
    /// Ephemeral sessions: temp session_id -> project path
    ephemeral_sessions: &'a HashMap<String, PathBuf>,
    /// Whether to hide inactive (Idle) sessions
    hide_inactive: bool,
}

impl<'a> Sidebar<'a> {
    pub fn new(
        groups: &'a [ConversationGroup],
        focused: bool,
        running_sessions: &'a std::collections::HashSet<String>,
        ephemeral_sessions: &'a HashMap<String, PathBuf>,
        hide_inactive: bool,
    ) -> Self {
        Self {
            groups,
            focused,
            running_sessions,
            ephemeral_sessions,
            hide_inactive,
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

        let title = if self.hide_inactive {
            " Conversations (active) "
        } else {
            " Conversations "
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner_area = block.inner(area);
        block.render(area, buf);

        let items = build_list_items(
            self.groups,
            &state.collapsed_groups,
            state.show_all_projects,
            &state.expanded_conversations,
            self.running_sessions,
            self.ephemeral_sessions,
            self.hide_inactive,
        );
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        StatefulWidget::render(list, inner_area, buf, &mut state.list_state);
    }
}

fn build_list_items(
    groups: &[ConversationGroup],
    collapsed: &std::collections::HashSet<String>,
    show_all_projects: bool,
    expanded_conversations: &std::collections::HashSet<String>,
    running_sessions: &std::collections::HashSet<String>,
    ephemeral_sessions: &HashMap<String, PathBuf>,
    hide_inactive: bool,
) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();

    let visible_groups = if show_all_projects || groups.len() <= DEFAULT_VISIBLE_PROJECTS {
        groups
    } else {
        &groups[..DEFAULT_VISIBLE_PROJECTS]
    };

    for group in visible_groups {
        let group_key = group.key();
        let is_collapsed = collapsed.contains(&group_key);

        // Group header with "+" indicator for new chat
        let arrow = if is_collapsed { "▸" } else { "▾" };
        let header = format!("{} {}", arrow, group.display_name());
        items.push(ListItem::new(Line::from(vec![
            Span::styled(header, Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" +", Style::default().fg(Color::Green)),
        ])));

        // Conversations and ephemeral sessions (if not collapsed)
        if !is_collapsed {
            // First, show ephemeral sessions for this group at the top
            // (ephemeral sessions are always shown - they're running by definition)
            let group_project_path = group.project_path();
            if let Some(project_path) = group_project_path {
                for (session_id, path) in ephemeral_sessions {
                    if path == &project_path {
                        // Render ephemeral session with distinctive styling
                        items.push(ListItem::new(Line::from(vec![
                            Span::raw("  "),
                            Span::styled("● ", Style::default().fg(Color::Green)),
                            Span::styled(
                                format!("New conversation ({})", &session_id[session_id.len().saturating_sub(1)..]),
                                Style::default().add_modifier(Modifier::ITALIC),
                            ),
                        ])));
                    }
                }
            }

            // Get all conversations and filter if hide_inactive is enabled
            let conversations = group.conversations();
            let filtered_convos: Vec<_> = if hide_inactive {
                conversations
                    .iter()
                    .filter(|conv| {
                        let is_running = running_sessions.contains(&conv.session_id);
                        is_running || !matches!(conv.status, ConversationStatus::Idle)
                    })
                    .collect()
            } else {
                conversations.iter().collect()
            };

            // Determine how many conversations to show (from filtered list)
            let is_expanded = expanded_conversations.contains(&group_key);
            let visible_convos: Vec<_> = if is_expanded || filtered_convos.len() <= DEFAULT_VISIBLE_CONVERSATIONS {
                filtered_convos.iter().copied().collect()
            } else {
                filtered_convos.iter().take(DEFAULT_VISIBLE_CONVERSATIONS).copied().collect()
            };

            // Then show saved conversations (limited or all)
            for conv in &visible_convos {
                // If session is running in background, show it as Active
                // regardless of the file-based status
                let is_running = running_sessions.contains(&conv.session_id);
                let status_indicator = if is_running {
                    Span::styled("● ", Style::default().fg(Color::Green))
                } else {
                    match conv.status {
                        ConversationStatus::Active => {
                            Span::styled("● ", Style::default().fg(Color::Green))
                        }
                        ConversationStatus::WaitingForInput => {
                            Span::styled("◐ ", Style::default().fg(Color::Yellow))
                        }
                        ConversationStatus::Idle => {
                            Span::styled("○ ", Style::default().fg(Color::DarkGray))
                        }
                    }
                };

                let display = truncate_string(&conv.display, 30);
                items.push(ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    status_indicator,
                    Span::raw(display),
                ])));
            }

            // Add "show more conversations" if truncated (use filtered count)
            if !is_expanded && filtered_convos.len() > DEFAULT_VISIBLE_CONVERSATIONS {
                let hidden = filtered_convos.len() - DEFAULT_VISIBLE_CONVERSATIONS;
                items.push(ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("↓ Show {} more...", hidden),
                        Style::default().fg(Color::Blue),
                    ),
                ])));
            }
        }
    }

    // Add "Show more" at end if truncated
    if !show_all_projects && groups.len() > DEFAULT_VISIBLE_PROJECTS {
        let hidden = groups.len() - DEFAULT_VISIBLE_PROJECTS;
        items.push(ListItem::new(Line::from(vec![Span::styled(
            format!("↓ Show {} more projects...", hidden),
            Style::default().fg(Color::Blue),
        )])));
    }

    items
}

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Represents an item in the flattened sidebar list
#[derive(Debug, Clone)]
pub enum SidebarItem {
    GroupHeader { key: String, #[allow(dead_code)] name: String },
    Conversation { group_key: String, index: usize },
    /// A running session that hasn't been saved yet (temp session)
    EphemeralSession { session_id: String, group_key: String },
    ShowMoreProjects { #[allow(dead_code)] hidden_count: usize },
    ShowMoreConversations { group_key: String, #[allow(dead_code)] hidden_count: usize },
}

/// Build a flat list of sidebar items for navigation
pub fn build_sidebar_items(
    groups: &[ConversationGroup],
    collapsed: &std::collections::HashSet<String>,
    show_all_projects: bool,
    expanded_conversations: &std::collections::HashSet<String>,
    ephemeral_sessions: &HashMap<String, PathBuf>,
    running_sessions: &std::collections::HashSet<String>,
    hide_inactive: bool,
) -> Vec<SidebarItem> {
    let mut items = Vec::new();

    let visible_groups = if show_all_projects || groups.len() <= DEFAULT_VISIBLE_PROJECTS {
        groups
    } else {
        &groups[..DEFAULT_VISIBLE_PROJECTS]
    };

    for group in visible_groups {
        let group_key = group.key();
        items.push(SidebarItem::GroupHeader {
            key: group_key.clone(),
            name: group.display_name(),
        });

        if !collapsed.contains(&group_key) {
            // First, add ephemeral sessions for this group
            // (ephemeral sessions are always shown - they're running by definition)
            let group_project_path = group.project_path();
            if let Some(project_path) = group_project_path {
                for (session_id, path) in ephemeral_sessions {
                    if path == &project_path {
                        items.push(SidebarItem::EphemeralSession {
                            session_id: session_id.clone(),
                            group_key: group_key.clone(),
                        });
                    }
                }
            }

            // Get all conversations and filter if hide_inactive is enabled
            // We keep track of original indices so lookup in app.rs still works
            let conversations = group.conversations();
            let filtered_indices: Vec<usize> = if hide_inactive {
                conversations
                    .iter()
                    .enumerate()
                    .filter(|(_, conv)| {
                        let is_running = running_sessions.contains(&conv.session_id);
                        is_running || !matches!(conv.status, ConversationStatus::Idle)
                    })
                    .map(|(idx, _)| idx)
                    .collect()
            } else {
                (0..conversations.len()).collect()
            };

            // Determine how many conversations to show (from filtered list)
            let is_expanded = expanded_conversations.contains(&group_key);
            let visible_indices: Vec<usize> = if is_expanded || filtered_indices.len() <= DEFAULT_VISIBLE_CONVERSATIONS {
                filtered_indices.clone()
            } else {
                filtered_indices.iter().take(DEFAULT_VISIBLE_CONVERSATIONS).copied().collect()
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
        items.push(SidebarItem::ShowMoreProjects {
            hidden_count: groups.len() - DEFAULT_VISIBLE_PROJECTS,
        });
    }

    items
}
