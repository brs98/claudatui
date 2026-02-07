//! Sidebar item building logic: constructs the flat list of `SidebarItem`s for navigation.

use std::collections::{HashMap, HashSet};

use crate::app::EphemeralSession;
use crate::claude::conversation::ConversationStatus;
use crate::claude::grouping::ConversationGroup;

use super::{
    ArchiveFilter, SidebarContext, SidebarItem, DEFAULT_VISIBLE_CONVERSATIONS,
    DEFAULT_VISIBLE_PROJECTS,
};

/// Build a flat list of sidebar items for navigation.
pub fn build_sidebar_items(
    ctx: &SidebarContext,
    collapsed: &HashSet<String>,
    show_all_projects: bool,
    expanded_conversations: &HashSet<String>,
) -> Vec<SidebarItem> {
    let mut items = Vec::new();

    let filter_lower = ctx.filter_query.to_lowercase();
    let has_text_filter = !filter_lower.is_empty();

    // Insert bookmark items at the top, mirroring build_list_items
    let bookmarks = ctx.bookmark_manager.get_all();
    if !bookmarks.is_empty() {
        items.push(SidebarItem::BookmarkHeader);
        for bookmark in bookmarks {
            items.push(SidebarItem::BookmarkEntry {
                slot: bookmark.slot,
            });
        }
        items.push(SidebarItem::BookmarkSeparator);
    }

    let visible_groups = if show_all_projects || ctx.groups.len() <= DEFAULT_VISIBLE_PROJECTS {
        ctx.groups
    } else {
        &ctx.groups[..DEFAULT_VISIBLE_PROJECTS]
    };

    for group in visible_groups {
        // Check if a non-plan-impl conversation in this group has a running PTY (parent)
        let group_has_running_parent = group
            .conversations()
            .iter()
            .any(|c| !c.is_plan_implementation && ctx.running_sessions.contains(&c.session_id));

        // Skip groups with no active content when hide_inactive is enabled
        if ctx.hide_inactive
            && !group_has_active_content(group, ctx.running_sessions, ctx.ephemeral_sessions)
        {
            continue;
        }

        let group_key = group.key();

        // When text filter is active, check if group name matches
        let group_name_matches =
            has_text_filter && group.display_name().to_lowercase().contains(&filter_lower);

        // Check if group has any conversations visible with current archive + text filter
        let has_visible_conversations = group.conversations().iter().any(|conv| {
            !is_hidden_plan_implementation(conv, ctx.running_sessions, group_has_running_parent)
                && should_show_conversation(
                    conv,
                    ctx.archive_filter,
                    ctx.running_sessions,
                    ctx.hide_inactive,
                )
                && (!has_text_filter
                    || group_name_matches
                    || conv_matches_filter(conv, &filter_lower))
        });

        // Skip groups with no visible conversations
        if !has_visible_conversations
            && (ctx.archive_filter != ArchiveFilter::All || has_text_filter)
        {
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
                    !is_hidden_plan_implementation(
                        conv,
                        ctx.running_sessions,
                        group_has_running_parent,
                    )
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
    if !show_all_projects && ctx.groups.len() > DEFAULT_VISIBLE_PROJECTS {
        // When hide_inactive is enabled, count only hidden groups with active content
        let hidden_groups = &ctx.groups[DEFAULT_VISIBLE_PROJECTS..];
        let hidden_count = if ctx.hide_inactive {
            hidden_groups
                .iter()
                .filter(|g| {
                    group_has_active_content(g, ctx.running_sessions, ctx.ephemeral_sessions)
                })
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
