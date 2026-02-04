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
}

/// Sidebar widget for displaying conversations
pub struct Sidebar<'a> {
    groups: &'a [ConversationGroup],
    focused: bool,
    /// Session IDs that are currently running (have active PTYs)
    running_sessions: &'a std::collections::HashSet<String>,
    /// Ephemeral sessions: temp session_id -> project path
    ephemeral_sessions: &'a HashMap<String, PathBuf>,
}

impl<'a> Sidebar<'a> {
    pub fn new(
        groups: &'a [ConversationGroup],
        focused: bool,
        running_sessions: &'a std::collections::HashSet<String>,
        ephemeral_sessions: &'a HashMap<String, PathBuf>,
    ) -> Self {
        Self {
            groups,
            focused,
            running_sessions,
            ephemeral_sessions,
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

        let block = Block::default()
            .title(" Conversations ")
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

            // Determine how many conversations to show
            let conversations = group.conversations();
            let is_expanded = expanded_conversations.contains(&group_key);
            let visible_convos = if is_expanded || conversations.len() <= DEFAULT_VISIBLE_CONVERSATIONS {
                conversations
            } else {
                &conversations[..DEFAULT_VISIBLE_CONVERSATIONS]
            };

            // Then show saved conversations (limited or all)
            for conv in visible_convos {
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

            // Add "show more conversations" if truncated
            if !is_expanded && conversations.len() > DEFAULT_VISIBLE_CONVERSATIONS {
                let hidden = conversations.len() - DEFAULT_VISIBLE_CONVERSATIONS;
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

            // Determine how many conversations to show
            let conversations = group.conversations();
            let is_expanded = expanded_conversations.contains(&group_key);
            let visible_count = if is_expanded || conversations.len() <= DEFAULT_VISIBLE_CONVERSATIONS {
                conversations.len()
            } else {
                DEFAULT_VISIBLE_CONVERSATIONS
            };

            // Then add saved conversations (limited or all)
            for index in 0..visible_count {
                items.push(SidebarItem::Conversation {
                    group_key: group_key.clone(),
                    index,
                });
            }

            // Add "show more conversations" if truncated
            if !is_expanded && conversations.len() > DEFAULT_VISIBLE_CONVERSATIONS {
                items.push(SidebarItem::ShowMoreConversations {
                    group_key: group_key.clone(),
                    hidden_count: conversations.len() - DEFAULT_VISIBLE_CONVERSATIONS,
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
