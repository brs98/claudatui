//! Sidebar item building logic: constructs the flat list of `SidebarItem`s for navigation.

use std::collections::{HashMap, HashSet};

use crate::app::EphemeralSession;
use crate::claude::conversation::ConversationStatus;
use crate::claude::grouping::ConversationGroup;

use super::{ArchiveFilter, ControlAction, SectionKind, SidebarContext, SidebarItem, SidebarState};

/// Build a flat list of sidebar items for navigation.
pub fn build_sidebar_items(
    ctx: &SidebarContext,
    collapsed: &HashSet<String>,
    collapsed_projects: &HashSet<String>,
    visible_conversations: &HashMap<String, usize>,
    visible_groups: &HashMap<String, usize>,
    other_collapsed: bool,
) -> Vec<SidebarItem> {
    let mut items = Vec::new();

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

    // Always show WorkspaceSectionHeader at top
    items.push(SidebarItem::WorkspaceSectionHeader);

    if has_workspaces {
        // Partition ALL groups first so workspace groups are never truncated
        let (workspace_groups, all_other_groups): (Vec<_>, Vec<_>) =
            ctx.groups.iter().partition(|g| is_in_workspace(g));

        // Group workspace groups by project
        let workspace_projects = group_by_project(&workspace_groups);

        for (project_key, project_name, groups) in &workspace_projects {
            items.push(SidebarItem::ProjectHeader {
                project_key: project_key.clone(),
                name: project_name.clone(),
                group_count: groups.len(),
            });

            if !collapsed_projects.contains(project_key) {
                render_groups_with_limit(
                    &mut items,
                    project_key,
                    groups,
                    ctx,
                    collapsed,
                    visible_conversations,
                    visible_groups,
                    &filter_lower,
                    has_text_filter,
                );
            }
        }

        // AddWorkspace after workspace projects (or after header if none)
        if !has_text_filter {
            items.push(SidebarItem::AddWorkspace);
        }

        // Render "Other" section if there are non-workspace groups
        if !all_other_groups.is_empty() {
            let other_projects = group_by_project(&all_other_groups);
            // Count total groups across all other projects
            let total_group_count: usize = other_projects.iter().map(|(_, _, g)| g.len()).sum();

            items.push(SidebarItem::OtherHeader {
                group_count: total_group_count,
            });

            if !other_collapsed {
                for (project_key, project_name, groups) in &other_projects {
                    items.push(SidebarItem::ProjectHeader {
                        project_key: project_key.clone(),
                        name: project_name.clone(),
                        group_count: groups.len(),
                    });

                    if !collapsed_projects.contains(project_key) {
                        render_groups_with_limit(
                            &mut items,
                            project_key,
                            groups,
                            ctx,
                            collapsed,
                            visible_conversations,
                            visible_groups,
                            &filter_lower,
                            has_text_filter,
                        );
                    }
                }
            }
        }
    } else {
        // No workspaces configured: WorkspaceSectionHeader + AddWorkspace, then flat projects
        if !has_text_filter {
            items.push(SidebarItem::AddWorkspace);
        }

        // Group all visible groups by project
        let all_groups_refs: Vec<&ConversationGroup> = ctx.groups.iter().collect();
        let projects = group_by_project(&all_groups_refs);

        for (project_key, project_name, groups) in &projects {
            items.push(SidebarItem::ProjectHeader {
                project_key: project_key.clone(),
                name: project_name.clone(),
                group_count: groups.len(),
            });

            if !collapsed_projects.contains(project_key) {
                render_groups_with_limit(
                    &mut items,
                    project_key,
                    groups,
                    ctx,
                    collapsed,
                    visible_conversations,
                    visible_groups,
                    &filter_lower,
                    has_text_filter,
                );
            }
        }
    }

    // During active text filter: remove empty ProjectHeaders and hide AddWorkspace
    if has_text_filter {
        // Already handled AddWorkspace above (not inserted when has_text_filter)
        // Remove ProjectHeaders that have no children following them
        remove_empty_project_headers(&mut items);
    }

    items
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

/// Remove ProjectHeader items that have no child items following them
/// (i.e., the next item is another ProjectHeader, OtherHeader, WorkspaceSectionHeader,
/// AddWorkspace, SectionControl, or end-of-list).
fn remove_empty_project_headers(items: &mut Vec<SidebarItem>) {
    let mut i = 0;
    while i < items.len() {
        if matches!(items[i], SidebarItem::ProjectHeader { .. }) {
            let next_is_empty = if i + 1 >= items.len() {
                true
            } else {
                matches!(
                    items[i + 1],
                    SidebarItem::ProjectHeader { .. }
                        | SidebarItem::OtherHeader { .. }
                        | SidebarItem::WorkspaceSectionHeader
                        | SidebarItem::AddWorkspace
                        | SidebarItem::SectionControl { .. }
                )
            };
            if next_is_empty {
                items.remove(i);
                continue;
            }
        }
        i += 1;
    }
}

/// Render groups within a project, applying the visible_groups limit.
#[allow(clippy::too_many_arguments)]
fn render_groups_with_limit(
    items: &mut Vec<SidebarItem>,
    project_key: &str,
    groups: &[&ConversationGroup],
    ctx: &SidebarContext,
    collapsed: &HashSet<String>,
    visible_conversations: &HashMap<String, usize>,
    visible_groups: &HashMap<String, usize>,
    filter_lower: &str,
    has_text_filter: bool,
) {
    let total = groups.len();
    let vis = if has_text_filter {
        total
    } else {
        SidebarState::visible_count(visible_groups, project_key).min(total)
    };

    for group in &groups[..vis] {
        render_group_items(
            items,
            group,
            &group.group_label(),
            ctx,
            collapsed,
            visible_conversations,
            filter_lower,
            has_text_filter,
        );
    }

    // Emit section controls for groups (only when not filtering)
    if !has_text_filter {
        let hidden = total.saturating_sub(vis);
        let is_expanded = visible_groups.contains_key(project_key);

        if hidden > 0 {
            items.push(SidebarItem::SectionControl {
                key: project_key.to_string(),
                kind: SectionKind::Groups,
                action: ControlAction::ShowMore(hidden),
            });
            items.push(SidebarItem::SectionControl {
                key: project_key.to_string(),
                kind: SectionKind::Groups,
                action: ControlAction::ShowAll(total),
            });
        }
        if is_expanded {
            items.push(SidebarItem::SectionControl {
                key: project_key.to_string(),
                kind: SectionKind::Groups,
                action: ControlAction::ShowFewer,
            });
            items.push(SidebarItem::SectionControl {
                key: project_key.to_string(),
                kind: SectionKind::Groups,
                action: ControlAction::Collapse,
            });
        }
    }
}

/// Render sidebar items for a single group (GroupHeader + ephemeral sessions + conversations + SectionControls).
#[allow(clippy::too_many_arguments)]
fn render_group_items(
    items: &mut Vec<SidebarItem>,
    group: &ConversationGroup,
    name: &str,
    ctx: &SidebarContext,
    collapsed: &HashSet<String>,
    visible_conversations: &HashMap<String, usize>,
    filter_lower: &str,
    has_text_filter: bool,
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

    let group_key = group.key();

    // When text filter is active, check if group name matches
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

    // Skip groups with no visible conversations
    if !has_visible_convs && (ctx.archive_filter != ArchiveFilter::All || has_text_filter) {
        return;
    }

    items.push(SidebarItem::GroupHeader {
        key: group_key.clone(),
        name: name.to_string(),
    });

    if !collapsed.contains(&group_key) {
        // First, add ephemeral sessions for this group
        // (ephemeral sessions are always shown - they're running by definition)
        // But only in Active or All views (not in Archived view)
        if ctx.archive_filter != ArchiveFilter::Archived {
            let group_project_path = group.project_path();
            if let Some(project_path) = group_project_path {
                for (session_id, ephemeral) in ctx.ephemeral_sessions {
                    if ephemeral.project_path == project_path {
                        items.push(SidebarItem::EphemeralSession {
                            session_id: session_id.clone(),
                            group_key: group_key.clone(),
                        });
                    }
                }
            }
        }

        // Get all conversations, hiding plan impls only when parent is orchestrating
        // Also filter by archive status, inactive, and text filter
        // We keep track of original indices so lookup in app.rs still works
        let conversations = group.conversations();
        let filtered_indices: Vec<usize> = conversations
            .iter()
            .enumerate()
            .filter(|(_, conv)| {
                !is_hidden_plan_implementation(conv, ctx.running_sessions, group_has_running_parent)
            })
            .filter(|(_, conv)| {
                should_show_conversation(
                    conv,
                    ctx.archive_filter,
                    ctx.running_sessions,
                    ctx.hide_inactive,
                )
            })
            .filter(|(_, conv)| {
                !has_text_filter || group_name_matches || conv_matches_filter(conv, filter_lower)
            })
            .map(|(idx, _)| idx)
            .collect();

        // Determine how many conversations to show (from filtered list)
        let total = filtered_indices.len();
        let vis = if has_text_filter {
            total
        } else {
            SidebarState::visible_count(visible_conversations, &group_key).min(total)
        };

        // Add visible conversations
        for &index in filtered_indices.iter().take(vis) {
            items.push(SidebarItem::Conversation {
                group_key: group_key.clone(),
                index,
            });
        }

        // Emit section controls for conversations (only when not filtering)
        if !has_text_filter {
            let hidden = total.saturating_sub(vis);
            let is_expanded = visible_conversations.contains_key(&group_key);

            if hidden > 0 {
                items.push(SidebarItem::SectionControl {
                    key: group_key.clone(),
                    kind: SectionKind::Conversations,
                    action: ControlAction::ShowMore(hidden),
                });
                items.push(SidebarItem::SectionControl {
                    key: group_key.clone(),
                    kind: SectionKind::Conversations,
                    action: ControlAction::ShowAll(total),
                });
            }
            if is_expanded {
                items.push(SidebarItem::SectionControl {
                    key: group_key.clone(),
                    kind: SectionKind::Conversations,
                    action: ControlAction::ShowFewer,
                });
                items.push(SidebarItem::SectionControl {
                    key: group_key.clone(),
                    kind: SectionKind::Conversations,
                    action: ControlAction::Collapse,
                });
            }
        }
    }
}

/// Check if a group has any active content (for hide_inactive filtering)
pub fn group_has_active_content(
    group: &ConversationGroup,
    running_sessions: &HashSet<String>,
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

    let group_has_running_parent = group
        .conversations()
        .iter()
        .any(|c| !c.is_plan_implementation && running_sessions.contains(&c.session_id));

    // Check for active/running conversations (excluding orchestrated plan implementations)
    for conv in group.conversations() {
        if is_hidden_plan_implementation(conv, running_sessions, group_has_running_parent) {
            continue;
        }
        let is_running = running_sessions.contains(&conv.session_id);
        if is_running || !matches!(conv.status, ConversationStatus::Idle) {
            return true;
        }
    }

    false
}

/// Check if a conversation matches the text filter (case-insensitive)
pub(super) fn conv_matches_filter(
    conv: &crate::claude::conversation::Conversation,
    filter_lower: &str,
) -> bool {
    conv.display.to_lowercase().contains(filter_lower)
        || conv
            .summary
            .as_ref()
            .is_some_and(|s| s.to_lowercase().contains(filter_lower))
}

/// Check if a plan implementation conversation should be hidden.
///
/// Plan implementations are hidden only when a parent session is orchestrating them.
/// During orchestration, the plan impl runs inside the parent's PTY (no PTY of its own)
/// while the parent (a non-plan conversation in the same group) has a running PTY.
/// Once the parent dies or the user directly activates the plan impl (giving it its own
/// PTY), it becomes visible.
pub(super) fn is_hidden_plan_implementation(
    conv: &crate::claude::conversation::Conversation,
    running_sessions: &HashSet<String>,
    group_has_running_parent: bool,
) -> bool {
    conv.is_plan_implementation
        && !running_sessions.contains(&conv.session_id)
        && group_has_running_parent
}

/// Check if a conversation should be shown based on archive filter and other criteria
pub(super) fn should_show_conversation(
    conv: &crate::claude::conversation::Conversation,
    archive_filter: ArchiveFilter,
    running_sessions: &HashSet<String>,
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
