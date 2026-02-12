//! Sidebar widget rendering: the `Sidebar` struct and its `StatefulWidget` implementation.

use std::collections::{HashMap, HashSet};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, StatefulWidget, Widget},
};

use crate::claude::grouping::ConversationGroup;

use super::items::{
    conv_matches_filter, group_has_active_content, is_hidden_plan_implementation,
    project_has_active_content, should_show_conversation, visible_group_count,
};
use super::{ArchiveFilter, ControlAction, SectionKind, SidebarContext, SidebarState, PAGE_SIZE};

/// Sidebar widget for displaying conversations.
pub struct Sidebar<'a> {
    /// Common sidebar parameters
    ctx: &'a SidebarContext<'a>,
    /// Whether the sidebar has keyboard focus
    focused: bool,
}

impl<'a> Sidebar<'a> {
    /// Create a new sidebar widget.
    pub fn new(ctx: &'a SidebarContext<'a>, focused: bool) -> Self {
        Self { ctx, focused }
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
        let title = match (self.ctx.hide_inactive, self.ctx.archive_filter) {
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
        let show_filter_row = self.ctx.filter_active || !self.ctx.filter_query.is_empty();
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
                self.ctx.filter_query,
                self.ctx.filter_active,
                self.ctx.filter_cursor_pos,
            );
        }

        let selected_index = state.list_state.selected();
        let items = build_list_items(
            self.ctx,
            &state.collapsed_groups,
            &state.collapsed_projects,
            &state.visible_conversations,
            &state.visible_groups,
            selected_index,
            state.other_collapsed,
        );
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        // Ensure the Workspaces header (index 0) stays visible when selection is near the top
        if state.list_state.selected().is_some_and(|s| s <= 1) {
            *state.list_state.offset_mut() = 0;
        }

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
    ctx: &SidebarContext,
    collapsed: &HashSet<String>,
    collapsed_projects: &HashSet<String>,
    visible_conversations: &HashMap<String, usize>,
    visible_groups: &HashMap<String, usize>,
    selected_index: Option<usize>,
    other_collapsed: bool,
) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    let mut current_index: usize = 0;

    // Prepare case-insensitive filter query
    let filter_lower = ctx.filter_query.to_lowercase();
    let has_text_filter = !filter_lower.is_empty();

    let has_workspaces = !ctx.workspaces.is_empty();

    let is_in_workspace = |group: &ConversationGroup| -> bool {
        if let Some(path) = group.project_path() {
            let path_str = path.to_string_lossy();
            ctx.workspaces
                .iter()
                .any(|ws| path_str.starts_with(ws.as_str()))
        } else {
            false
        }
    };

    // WorkspaceSectionHeader always at top — show profile name if active
    let header_text = match ctx.active_profile_name {
        Some(name) => name.to_string(),
        None => "Workspaces".to_string(),
    };
    let line_num = format_relative_line_number(current_index, selected_index);
    items.push(ListItem::new(Line::from(vec![
        Span::styled(line_num, Style::default().fg(Color::DarkGray)),
        Span::styled(header_text, Style::default().add_modifier(Modifier::BOLD)),
    ])));
    current_index += 1;

    if has_workspaces {
        // Partition ALL groups first so workspace groups are never truncated
        let (workspace_groups, all_other_groups): (Vec<_>, Vec<_>) =
            ctx.groups.iter().partition(|g| is_in_workspace(g));

        // Group workspace groups by project
        let workspace_projects = group_by_project(&workspace_groups);

        for (project_key, project_name, groups) in &workspace_projects {
            // Skip projects with no active content in active mode
            if ctx.hide_inactive
                && !project_has_active_content(groups, ctx.running_sessions, ctx.ephemeral_sessions)
            {
                continue;
            }

            let is_project_collapsed = collapsed_projects.contains(project_key);
            render_project_header(
                &mut items,
                &mut current_index,
                project_name,
                is_project_collapsed,
                selected_index,
                " ",
            );

            if !is_project_collapsed {
                render_groups_list_with_limit(
                    &mut items,
                    &mut current_index,
                    project_key,
                    groups,
                    ctx,
                    collapsed,
                    visible_conversations,
                    visible_groups,
                    selected_index,
                    &filter_lower,
                    has_text_filter,
                    2,
                );
            }
        }

        // AddWorkspace after workspace projects
        if !has_text_filter && !ctx.hide_inactive {
            render_add_workspace(&mut items, &mut current_index, selected_index, " ");
        }

        // Render "Other" section if there are non-workspace groups
        if !all_other_groups.is_empty() {
            let other_projects = group_by_project(&all_other_groups);
            let total_group_count: usize = other_projects
                .iter()
                .map(|(_, _, g)| visible_group_count(g, ctx))
                .sum();

            // When hide_inactive is on, only show OtherHeader if at least one
            // "other" group has active content.
            let show_other = !ctx.hide_inactive
                || all_other_groups.iter().any(|g| {
                    group_has_active_content(g, ctx.running_sessions, ctx.ephemeral_sessions)
                });

            if show_other {
                let arrow = if other_collapsed {
                    "\u{25b8}"
                } else {
                    "\u{25be}"
                };
                let header = format!("{} Other ({} projects)", arrow, total_group_count);
                let line_num = format_relative_line_number(current_index, selected_index);
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        header,
                        Style::default()
                            .add_modifier(Modifier::BOLD)
                            .fg(Color::DarkGray),
                    ),
                ])));
                current_index += 1;

                if !other_collapsed {
                    for (project_key, project_name, groups) in &other_projects {
                        // Skip projects with no active content in active mode
                        if ctx.hide_inactive
                            && !project_has_active_content(
                                groups,
                                ctx.running_sessions,
                                ctx.ephemeral_sessions,
                            )
                        {
                            continue;
                        }

                        let is_project_collapsed = collapsed_projects.contains(project_key);
                        render_project_header(
                            &mut items,
                            &mut current_index,
                            project_name,
                            is_project_collapsed,
                            selected_index,
                            " ",
                        );

                        if !is_project_collapsed {
                            render_groups_list_with_limit(
                                &mut items,
                                &mut current_index,
                                project_key,
                                groups,
                                ctx,
                                collapsed,
                                visible_conversations,
                                visible_groups,
                                selected_index,
                                &filter_lower,
                                has_text_filter,
                                2,
                            );
                        }
                    }
                }
            }
        }
    } else {
        // No workspaces configured: AddWorkspace, then flat projects
        if !has_text_filter && !ctx.hide_inactive {
            render_add_workspace(&mut items, &mut current_index, selected_index, " ");
        }

        // Group all groups by project
        let all_groups_refs: Vec<&ConversationGroup> = ctx.groups.iter().collect();
        let projects = group_by_project(&all_groups_refs);

        for (project_key, project_name, groups) in &projects {
            // Skip projects with no active content in active mode
            if ctx.hide_inactive
                && !project_has_active_content(groups, ctx.running_sessions, ctx.ephemeral_sessions)
            {
                continue;
            }

            let is_project_collapsed = collapsed_projects.contains(project_key);
            render_project_header(
                &mut items,
                &mut current_index,
                project_name,
                is_project_collapsed,
                selected_index,
                "",
            );

            if !is_project_collapsed {
                render_groups_list_with_limit(
                    &mut items,
                    &mut current_index,
                    project_key,
                    groups,
                    ctx,
                    collapsed,
                    visible_conversations,
                    visible_groups,
                    selected_index,
                    &filter_lower,
                    has_text_filter,
                    1,
                );
            }
        }
    }

    items
}

/// Render a ProjectHeader item (bold cyan with collapse arrow).
fn render_project_header(
    items: &mut Vec<ListItem<'static>>,
    current_index: &mut usize,
    name: &str,
    is_collapsed: bool,
    selected_index: Option<usize>,
    indent: &str,
) {
    let arrow = if is_collapsed { "\u{25b8}" } else { "\u{25be}" };
    let header = format!("{}{} {}", indent, arrow, name);
    let line_num = format_relative_line_number(*current_index, selected_index);
    items.push(ListItem::new(Line::from(vec![
        Span::styled(line_num, Style::default().fg(Color::DarkGray)),
        Span::styled(
            header,
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ),
    ])));
    *current_index += 1;
}

/// Render an AddWorkspace item (dim "+ Add workspace").
fn render_add_workspace(
    items: &mut Vec<ListItem<'static>>,
    current_index: &mut usize,
    selected_index: Option<usize>,
    indent: &str,
) {
    let line_num = format_relative_line_number(*current_index, selected_index);
    items.push(ListItem::new(Line::from(vec![
        Span::styled(line_num, Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}+ Add workspace", indent),
            Style::default().fg(Color::DarkGray),
        ),
    ])));
    *current_index += 1;
}

/// Group a list of `ConversationGroup` references by their `project_key()`.
/// Returns `Vec<(project_key, project_display_name, Vec<&ConversationGroup>)>` preserving
/// the order of first appearance.
fn group_by_project<'a>(
    groups: &[&'a ConversationGroup],
) -> Vec<(String, String, Vec<&'a ConversationGroup>)> {
    let mut result: Vec<(String, String, Vec<&'a ConversationGroup>)> = Vec::new();

    for group in groups {
        let key = group.project_key();
        if let Some(entry) = result.iter_mut().find(|(k, _, _)| *k == key) {
            entry.2.push(group);
        } else {
            result.push((key, group.project_display_name(), vec![group]));
        }
    }

    result
}

/// Render groups within a project, applying the visible_groups limit.
#[allow(clippy::too_many_arguments)]
fn render_groups_list_with_limit(
    items: &mut Vec<ListItem<'static>>,
    current_index: &mut usize,
    project_key: &str,
    groups: &[&ConversationGroup],
    ctx: &SidebarContext,
    collapsed: &HashSet<String>,
    visible_conversations: &HashMap<String, usize>,
    visible_groups: &HashMap<String, usize>,
    selected_index: Option<usize>,
    filter_lower: &str,
    has_text_filter: bool,
    indent_offset: usize,
) {
    let filtered_groups: Vec<&ConversationGroup> = if ctx.hide_inactive {
        groups
            .iter()
            .copied()
            .filter(|g| group_has_active_content(g, ctx.running_sessions, ctx.ephemeral_sessions))
            .collect()
    } else {
        groups.to_vec()
    };
    let total = filtered_groups.len();
    let vis = if has_text_filter {
        total
    } else {
        SidebarState::visible_count(visible_groups, project_key).min(total)
    };

    for group in filtered_groups.iter().take(vis) {
        render_group_list_items(
            items,
            current_index,
            group,
            &group.group_label(),
            ctx,
            collapsed,
            visible_conversations,
            selected_index,
            filter_lower,
            has_text_filter,
            indent_offset,
        );
    }

    // Emit section controls for groups (only when not filtering)
    if !has_text_filter && total > PAGE_SIZE {
        let hidden = total.saturating_sub(vis);
        let is_expanded = visible_groups.contains_key(project_key);
        let indent = " ".repeat(indent_offset);

        if hidden > 0 {
            render_section_control(
                items,
                current_index,
                selected_index,
                &indent,
                &ControlAction::ShowMore(hidden),
                SectionKind::Groups,
            );
            if hidden > PAGE_SIZE {
                render_section_control(
                    items,
                    current_index,
                    selected_index,
                    &indent,
                    &ControlAction::ShowAll(total),
                    SectionKind::Groups,
                );
            }
        }
        if is_expanded && hidden == 0 {
            if vis > 2 * PAGE_SIZE {
                render_section_control(
                    items,
                    current_index,
                    selected_index,
                    &indent,
                    &ControlAction::ShowFewer,
                    SectionKind::Groups,
                );
            }
            render_section_control(
                items,
                current_index,
                selected_index,
                &indent,
                &ControlAction::Collapse,
                SectionKind::Groups,
            );
        }
    }
}

/// Render list items for a single group (header + ephemeral sessions + conversations + SectionControls).
#[allow(clippy::too_many_arguments)]
fn render_group_list_items(
    items: &mut Vec<ListItem<'static>>,
    current_index: &mut usize,
    group: &ConversationGroup,
    name: &str,
    ctx: &SidebarContext,
    collapsed: &HashSet<String>,
    visible_conversations: &HashMap<String, usize>,
    selected_index: Option<usize>,
    filter_lower: &str,
    has_text_filter: bool,
    indent_offset: usize,
) {
    // Check if a non-plan-impl conversation in this group has a running PTY (parent)
    let group_has_running_parent = group
        .conversations()
        .iter()
        .any(|c| !c.is_plan_implementation && ctx.running_sessions.contains(&c.session_id));

    // Skip groups with no active content when hide_inactive is enabled
    if ctx.hide_inactive
        && !group_has_active_content(group, ctx.running_sessions, ctx.ephemeral_sessions)
    {
        return;
    }

    // When text filter is active, check if group name matches or any conversation matches
    let group_name_matches =
        has_text_filter && group.display_name().to_lowercase().contains(filter_lower);

    // Check if group has any conversations visible with current archive + text filter
    let has_visible_convs = group.conversations().iter().any(|conv| {
        !is_hidden_plan_implementation(conv, ctx.running_sessions, group_has_running_parent)
            && should_show_conversation(
                conv,
                ctx.archive_filter,
                ctx.running_sessions,
                ctx.hide_inactive,
            )
            && (!has_text_filter || group_name_matches || conv_matches_filter(conv, filter_lower))
    });

    // Skip groups with no visible conversations (unless showing all and no text filter)
    if !has_visible_convs
        && (ctx.archive_filter != ArchiveFilter::All || has_text_filter || ctx.hide_inactive)
    {
        return;
    }

    let group_key = group.key();
    let is_collapsed = collapsed.contains(&group_key);

    // Build indent strings
    let group_indent = " ".repeat(indent_offset);
    let conv_indent = " ".repeat(indent_offset + 2);

    // Group header with "+" indicator for new chat
    let arrow = if is_collapsed { "\u{25b8}" } else { "\u{25be}" };
    let header = format!("{}{} {}", group_indent, arrow, name);
    let line_num = format_relative_line_number(*current_index, selected_index);
    items.push(ListItem::new(Line::from(vec![
        Span::styled(line_num, Style::default().fg(Color::DarkGray)),
        Span::styled(header, Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(" +", Style::default().fg(Color::Green)),
    ])));
    *current_index += 1;

    // Conversations and ephemeral sessions (if not collapsed)
    if !is_collapsed {
        // First, show ephemeral sessions for this group at the top
        let group_project_path = group.project_path();
        if let Some(project_path) = group_project_path {
            for (session_id, ephemeral) in ctx.ephemeral_sessions {
                if ephemeral.project_path == project_path {
                    // Render ephemeral session with distinctive styling
                    let line_num = format_relative_line_number(*current_index, selected_index);
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                        Span::raw(conv_indent.clone()),
                        Span::styled("\u{25d0} ", Style::default().fg(Color::Yellow)),
                        Span::styled(
                            format!(
                                "New conversation ({})",
                                &session_id[session_id.len().saturating_sub(1)..]
                            ),
                            Style::default().add_modifier(Modifier::ITALIC),
                        ),
                    ])));
                    *current_index += 1;
                }
            }
        }

        // Get all conversations, hiding plan impls only when parent is orchestrating
        // Also filter by archive status, inactive, and text filter
        let conversations = group.conversations();
        let filtered_convos: Vec<_> = conversations
            .iter()
            .filter(|conv| {
                !is_hidden_plan_implementation(conv, ctx.running_sessions, group_has_running_parent)
            })
            .filter(|conv| {
                should_show_conversation(
                    conv,
                    ctx.archive_filter,
                    ctx.running_sessions,
                    ctx.hide_inactive,
                )
            })
            .filter(|conv| {
                !has_text_filter || group_name_matches || conv_matches_filter(conv, filter_lower)
            })
            .collect();

        // Determine how many conversations to show (from filtered list)
        let total = filtered_convos.len();
        let vis = if has_text_filter {
            total
        } else {
            SidebarState::visible_count(visible_conversations, &group_key).min(total)
        };

        // Then show saved conversations (limited or all)
        for conv in filtered_convos.iter().take(vis) {
            // If session is running in background, show it as Active
            // regardless of the file-based status
            let is_running = ctx.running_sessions.contains(&conv.session_id);
            let (status_indicator, archive_indicator) = if is_running {
                (
                    Span::styled("\u{25cf} ", Style::default().fg(Color::Green)),
                    None,
                )
            } else {
                // Not running -- always show as idle regardless of JSONL state
                let status = Span::styled("\u{25cb} ", Style::default().fg(Color::DarkGray));
                // Show archive indicator when in "All" view
                let archive = if ctx.archive_filter == ArchiveFilter::All && conv.is_archived {
                    Some(Span::styled(
                        "\u{1f4e6} ",
                        Style::default().fg(Color::DarkGray),
                    ))
                } else {
                    None
                };
                (status, archive)
            };

            let display = truncate_string(&conv.display, 30);
            let line_num = format_relative_line_number(*current_index, selected_index);
            let mut line_parts = vec![
                Span::styled(line_num, Style::default().fg(Color::DarkGray)),
                Span::raw(conv_indent.clone()),
            ];

            // Add archive indicator if present (only in All view)
            if let Some(indicator) = archive_indicator {
                line_parts.push(indicator);
            }

            line_parts.push(status_indicator);
            line_parts.push(Span::raw(display));

            items.push(ListItem::new(Line::from(line_parts)));
            *current_index += 1;
        }

        // Emit section controls for conversations (only when not filtering)
        if !has_text_filter && total > PAGE_SIZE {
            let hidden = total.saturating_sub(vis);
            let is_expanded = visible_conversations.contains_key(&group_key);

            if hidden > 0 {
                render_section_control(
                    items,
                    current_index,
                    selected_index,
                    &conv_indent,
                    &ControlAction::ShowMore(hidden),
                    SectionKind::Conversations,
                );
                if hidden > PAGE_SIZE {
                    render_section_control(
                        items,
                        current_index,
                        selected_index,
                        &conv_indent,
                        &ControlAction::ShowAll(total),
                        SectionKind::Conversations,
                    );
                }
            }
            if is_expanded && hidden == 0 {
                if vis > 2 * PAGE_SIZE {
                    render_section_control(
                        items,
                        current_index,
                        selected_index,
                        &conv_indent,
                        &ControlAction::ShowFewer,
                        SectionKind::Conversations,
                    );
                }
                render_section_control(
                    items,
                    current_index,
                    selected_index,
                    &conv_indent,
                    &ControlAction::Collapse,
                    SectionKind::Conversations,
                );
            }
        }
    }
}

/// Render a single section control item (Show more/all/fewer/Collapse).
fn render_section_control(
    items: &mut Vec<ListItem<'static>>,
    current_index: &mut usize,
    selected_index: Option<usize>,
    indent: &str,
    action: &ControlAction,
    kind: SectionKind,
) {
    let kind_label = match kind {
        SectionKind::Conversations => "",
        SectionKind::Groups => " groups",
    };
    let label = match action {
        ControlAction::ShowMore(hidden) => {
            let show_count = PAGE_SIZE.min(*hidden);
            format!(
                "{}\u{25bc} Show {} more{}  ({} hidden)",
                indent, show_count, kind_label, hidden
            )
        }
        ControlAction::ShowAll(total) => {
            format!("{}\u{25bc} Show all{} ({})", indent, kind_label, total)
        }
        ControlAction::ShowFewer => {
            format!("{}\u{25b2} Show {} fewer{}", indent, PAGE_SIZE, kind_label)
        }
        ControlAction::Collapse => {
            format!("{}\u{25b2} Collapse{}", indent, kind_label)
        }
    };
    let line_num = format_relative_line_number(*current_index, selected_index);
    items.push(ListItem::new(Line::from(vec![
        Span::styled(line_num, Style::default().fg(Color::DarkGray)),
        Span::styled(label, Style::default().fg(Color::Blue)),
    ])));
    *current_index += 1;
}

/// Format a relative line number for display.
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
        let mut end = max_len.saturating_sub(3);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
