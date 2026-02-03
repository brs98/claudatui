use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, StatefulWidget, Widget},
};

use crate::claude::conversation::ConversationStatus;
use crate::claude::grouping::ConversationGroup;

/// Sidebar widget state
#[derive(Default)]
pub struct SidebarState {
    pub list_state: ListState,
    pub collapsed_groups: std::collections::HashSet<String>,
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
}

/// Sidebar widget for displaying conversations
pub struct Sidebar<'a> {
    groups: &'a [ConversationGroup],
    focused: bool,
}

impl<'a> Sidebar<'a> {
    pub fn new(groups: &'a [ConversationGroup], focused: bool) -> Self {
        Self { groups, focused }
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

        let items = build_list_items(self.groups, &state.collapsed_groups);
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
) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();

    for group in groups {
        let group_key = group.key();
        let is_collapsed = collapsed.contains(&group_key);

        // Group header
        let arrow = if is_collapsed { "▸" } else { "▾" };
        let header = format!("{} {}", arrow, group.display_name());
        items.push(ListItem::new(Line::from(vec![
            Span::styled(header, Style::default().add_modifier(Modifier::BOLD)),
        ])));

        // Conversations (if not collapsed)
        if !is_collapsed {
            for conv in group.conversations() {
                let status_indicator = match conv.status {
                    ConversationStatus::Active => Span::styled("● ", Style::default().fg(Color::Green)),
                    ConversationStatus::WaitingForInput => {
                        Span::styled("◐ ", Style::default().fg(Color::Yellow))
                    }
                    ConversationStatus::Idle => Span::styled("○ ", Style::default().fg(Color::DarkGray)),
                };

                let display = truncate_string(&conv.display, 30);
                items.push(ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    status_indicator,
                    Span::raw(display),
                ])));
            }
        }
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
}

/// Build a flat list of sidebar items for navigation
pub fn build_sidebar_items(
    groups: &[ConversationGroup],
    collapsed: &std::collections::HashSet<String>,
) -> Vec<SidebarItem> {
    let mut items = Vec::new();

    for group in groups {
        let group_key = group.key();
        items.push(SidebarItem::GroupHeader {
            key: group_key.clone(),
            name: group.display_name(),
        });

        if !collapsed.contains(&group_key) {
            for (index, _conv) in group.conversations().iter().enumerate() {
                items.push(SidebarItem::Conversation {
                    group_key: group_key.clone(),
                    index,
                });
            }
        }
    }

    items
}
