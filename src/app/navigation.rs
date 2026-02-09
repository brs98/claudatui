//! Sidebar navigation methods on App.

use super::*;

impl App {
    /// Navigate up in the sidebar, skipping non-selectable items
    pub fn navigate_up(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let len = items.len();
        let mut new_idx = if current == 0 { len - 1 } else { current - 1 };
        // Skip non-selectable items (bounded to prevent infinite loop)
        for _ in 0..len {
            if items[new_idx].is_selectable() {
                break;
            }
            new_idx = if new_idx == 0 { len - 1 } else { new_idx - 1 };
        }
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Navigate down in the sidebar, skipping non-selectable items
    pub fn navigate_down(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let len = items.len();
        let mut new_idx = if current >= len - 1 { 0 } else { current + 1 };
        // Skip non-selectable items (bounded to prevent infinite loop)
        for _ in 0..len {
            if items[new_idx].is_selectable() {
                break;
            }
            new_idx = if new_idx >= len - 1 { 0 } else { new_idx + 1 };
        }
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Jump to first selectable item
    pub fn jump_to_first(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }
        let first = items
            .iter()
            .position(SidebarItem::is_selectable)
            .unwrap_or(0);
        self.sidebar_state.list_state.select(Some(first));
        self.update_selected_conversation();
    }

    /// Jump to last selectable item
    pub fn jump_to_last(&mut self) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }
        let last = items
            .iter()
            .rposition(SidebarItem::is_selectable)
            .unwrap_or(items.len() - 1);
        self.sidebar_state.list_state.select(Some(last));
        self.update_selected_conversation();
    }

    /// Check if a group has active content (running session, ephemeral session, or non-idle conversation)
    fn is_group_active(&self, group_key: &str) -> bool {
        let running = self.running_session_ids();
        for group in &self.groups {
            if group.key() == group_key {
                return group_has_active_content(group, &running, &self.ephemeral_sessions);
            }
        }
        false
    }

    /// Find the active session within a group (ephemeral or running conversation)
    ///
    /// Priority:
    /// 1. Ephemeral sessions (new conversations not yet persisted)
    /// 2. Running conversations (have active PTYs)
    ///
    /// Only trusts live PTY state — does NOT fall back to JSONL-based `conv.status`,
    /// which can be stale when Claude exits externally.
    ///
    /// Returns the sidebar item index and session info if found.
    fn find_active_session_in_group(&self, group_key: &str) -> Option<ActiveSessionInfo> {
        let items = self.sidebar_items();
        let running = self.running_session_ids();

        // Find the group
        let group = self.groups.iter().find(|g| g.key() == group_key)?;
        let group_project_path = group.project_path();

        // Priority 1: Check for ephemeral sessions
        if let Some(project_path) = group_project_path {
            for (session_id, ephemeral) in &self.ephemeral_sessions {
                if ephemeral.project_path == project_path {
                    // Find this ephemeral session's index in the sidebar items
                    let index = items.iter().position(|item| {
                        matches!(item, SidebarItem::EphemeralSession { session_id: sid, .. } if sid == session_id)
                    });
                    if let Some(idx) = index {
                        return Some(ActiveSessionInfo::Ephemeral {
                            index: idx,
                            session_id: session_id.clone(),
                        });
                    }
                }
            }
        }

        // Priority 2 & 3: Check conversations (running first, then active status)
        let conversations = group.conversations();

        // Plan impls are only hidden when orchestrated by a running parent
        let group_has_running_parent = conversations
            .iter()
            .any(|c| !c.is_plan_implementation && running.contains(&c.session_id));

        // First pass: find running conversations (excluding orchestrated plan implementations)
        for (conv_idx, conv) in conversations.iter().enumerate() {
            if conv.is_plan_implementation
                && !running.contains(&conv.session_id)
                && group_has_running_parent
            {
                continue;
            }
            if running.contains(&conv.session_id) {
                // Find this conversation's index in the sidebar items
                let index = items.iter().position(|item| {
                    matches!(item, SidebarItem::Conversation { group_key: gk, index } if gk == group_key && *index == conv_idx)
                });
                if let Some(idx) = index {
                    return Some(ActiveSessionInfo::Conversation {
                        index: idx,
                        session_id: conv.session_id.clone(),
                        conversation: conv.clone(),
                    });
                }
            }
        }

        None
    }

    /// Get the group key for the item at the given index
    pub(crate) fn get_group_key_for_index(items: &[SidebarItem], index: usize) -> Option<String> {
        items.get(index).and_then(|item| match item {
            SidebarItem::GroupHeader { key, .. } => Some(key.clone()),
            SidebarItem::Conversation { group_key, .. } => Some(group_key.clone()),
            SidebarItem::EphemeralSession { group_key, .. } => Some(group_key.clone()),
            SidebarItem::SectionControl {
                key,
                kind: crate::ui::sidebar::SectionKind::Conversations,
                ..
            } => Some(key.clone()),
            SidebarItem::SectionControl { .. }
            | SidebarItem::OtherHeader { .. }
            | SidebarItem::WorkspaceSectionHeader
            | SidebarItem::AddWorkspace
            | SidebarItem::ProjectHeader { .. } => None,
        })
    }

    /// Cycle forward to the next active project's group header
    /// Returns true if successfully moved to a different active project
    pub fn cycle_next_active_project(&mut self) -> bool {
        let items = self.sidebar_items();
        if items.is_empty() {
            return false;
        }

        // Build list of (index, group_key) for active group headers
        let active_headers: Vec<(usize, String)> = items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                if let SidebarItem::GroupHeader { key, .. } = item {
                    if self.is_group_active(key) {
                        return Some((idx, key.clone()));
                    }
                }
                None
            })
            .collect();

        if active_headers.is_empty() {
            return false;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let current_group = Self::get_group_key_for_index(&items, current);

        // Find the next active header after current position (with wrap)
        let next = active_headers
            .iter()
            .find(|(idx, key)| *idx > current && Some(key) != current_group.as_ref())
            .or_else(|| {
                // Wrap around - find first active header with different key
                active_headers
                    .iter()
                    .find(|(_, key)| Some(key) != current_group.as_ref())
            })
            .or_else(|| {
                // Only one active project - just go to its header
                active_headers.first()
            });

        if let Some((idx, _)) = next {
            self.sidebar_state.list_state.select(Some(*idx));
            self.update_selected_conversation();
            true
        } else {
            false
        }
    }

    /// Cycle backward to the previous active project's group header
    /// Returns true if successfully moved to a different active project
    pub fn cycle_prev_active_project(&mut self) -> bool {
        let items = self.sidebar_items();
        if items.is_empty() {
            return false;
        }

        // Build list of (index, group_key) for active group headers
        let active_headers: Vec<(usize, String)> = items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                if let SidebarItem::GroupHeader { key, .. } = item {
                    if self.is_group_active(key) {
                        return Some((idx, key.clone()));
                    }
                }
                None
            })
            .collect();

        if active_headers.is_empty() {
            return false;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let current_group = Self::get_group_key_for_index(&items, current);

        // Find the previous active header before current position (with wrap)
        let prev = active_headers
            .iter()
            .rev()
            .find(|(idx, key)| *idx < current && Some(key) != current_group.as_ref())
            .or_else(|| {
                // Wrap around - find last active header with different key
                active_headers
                    .iter()
                    .rev()
                    .find(|(_, key)| Some(key) != current_group.as_ref())
            })
            .or_else(|| {
                // Only one active project - just go to its header
                active_headers.last()
            });

        if let Some((idx, _)) = prev {
            self.sidebar_state.list_state.select(Some(*idx));
            self.update_selected_conversation();
            true
        } else {
            false
        }
    }

    /// Cycle to next/previous active project AND switch to the active session within it.
    ///
    /// This method combines cycling and switching in one operation:
    /// 1. Cycles to the next (forward=true) or previous (forward=false) active project
    /// 2. Finds the active session within that project (ephemeral or running conversation)
    /// 3. Switches to that session and focuses the terminal
    ///
    /// Returns Ok(true) if successfully switched, Ok(false) if no switch occurred.
    pub fn cycle_and_switch_to_active(&mut self, forward: bool) -> Result<bool> {
        self.clear_preview();
        // First, cycle to the target group
        let cycled = if forward {
            self.cycle_next_active_project()
        } else {
            self.cycle_prev_active_project()
        };

        if !cycled {
            return Ok(false);
        }

        // Get the group key from the newly selected position
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);
        let group_key = match items.get(selected) {
            Some(SidebarItem::GroupHeader { key, .. }) => key.clone(),
            _ => return Ok(false),
        };

        // Find the active session in this group
        if let Some(session_info) = self.find_active_session_in_group(&group_key) {
            match session_info {
                ActiveSessionInfo::Ephemeral { index, session_id } => {
                    // Select the ephemeral session in sidebar
                    self.sidebar_state.list_state.select(Some(index));
                    // Switch to the already-running ephemeral session
                    self.active_session_id = Some(session_id);
                    self.selected_conversation = None;
                    self.focus = Focus::Terminal(TerminalPaneId::Primary);
                    self.enter_insert_mode();
                }
                ActiveSessionInfo::Conversation {
                    index,
                    session_id: claude_session_id,
                    conversation,
                } => {
                    // Select the conversation in sidebar
                    self.sidebar_state.list_state.select(Some(index));
                    self.selected_conversation = Some(conversation.clone());

                    // Check if we already have a daemon session for this conversation
                    let existing_session = self
                        .session_to_claude_id
                        .iter()
                        .find(|(_, v)| v.as_deref() == Some(claude_session_id.as_str()))
                        .map(|(k, _)| k.clone());

                    if let Some(daemon_session_id) = existing_session {
                        // Switch to existing session
                        self.active_session_id = Some(daemon_session_id);
                    } else {
                        // Start new session with --resume
                        self.start_session(&conversation.project_path, Some(&claude_session_id))?;
                        self.record_resume_jsonl_size(
                            &claude_session_id,
                            &conversation.project_path,
                        );
                        self.update_conversation_status_in_groups(
                            &claude_session_id,
                            ConversationStatus::WaitingForInput,
                        );
                    }
                    self.focus = Focus::Terminal(TerminalPaneId::Primary);
                    self.enter_insert_mode();
                }
            }
            return Ok(true);
        }

        Ok(false)
    }

    /// Navigate up by N selectable items (clamping at top, no wrap)
    pub fn navigate_up_by(&mut self, count: usize) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let mut new_idx = current;
        let mut remaining = count;
        while remaining > 0 && new_idx > 0 {
            new_idx -= 1;
            if items[new_idx].is_selectable() {
                remaining -= 1;
            }
        }
        // If we landed on a non-selectable item, scan forward to the nearest selectable
        if !items[new_idx].is_selectable() {
            if let Some(pos) = items[new_idx..].iter().position(SidebarItem::is_selectable) {
                new_idx += pos;
            }
        }
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Navigate down by N selectable items (clamping at bottom, no wrap)
    pub fn navigate_down_by(&mut self, count: usize) {
        let items = self.sidebar_items();
        if items.is_empty() {
            return;
        }

        let current = self.sidebar_state.list_state.selected().unwrap_or(0);
        let max_idx = items.len() - 1;
        let mut new_idx = current;
        let mut remaining = count;
        while remaining > 0 && new_idx < max_idx {
            new_idx += 1;
            if items[new_idx].is_selectable() {
                remaining -= 1;
            }
        }
        // If we landed on a non-selectable item, scan backward to the nearest selectable
        if !items[new_idx].is_selectable() {
            if let Some(pos) = items[..=new_idx]
                .iter()
                .rposition(SidebarItem::is_selectable)
            {
                new_idx = pos;
            }
        }
        self.sidebar_state.list_state.select(Some(new_idx));
        self.update_selected_conversation();
    }

    /// Toggle collapse state of current group
    pub fn toggle_current_group(&mut self) {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        if let Some(item) = items.get(selected) {
            match item {
                SidebarItem::GroupHeader { key, .. } => {
                    self.sidebar_state.toggle_group(key);
                }
                SidebarItem::Conversation { group_key, .. }
                | SidebarItem::EphemeralSession { group_key, .. } => {
                    self.sidebar_state.toggle_group(group_key);
                }
                SidebarItem::SectionControl { key, kind, action } => {
                    use crate::ui::sidebar::{ControlAction, SectionKind};
                    let map = match kind {
                        SectionKind::Conversations => &mut self.sidebar_state.visible_conversations,
                        SectionKind::Groups => &mut self.sidebar_state.visible_groups,
                    };
                    match action {
                        ControlAction::ShowMore(hidden) => {
                            // total = current visible + hidden
                            let current = SidebarState::visible_count(map, key);
                            SidebarState::show_more(map, key, current + hidden);
                        }
                        ControlAction::ShowAll(total) => {
                            SidebarState::show_all(map, key, *total);
                        }
                        ControlAction::ShowFewer => {
                            SidebarState::show_fewer(map, key);
                        }
                        ControlAction::Collapse => {
                            SidebarState::collapse_to_default(map, key);
                        }
                    }
                }
                SidebarItem::OtherHeader { .. } => {
                    self.sidebar_state.toggle_other_collapsed();
                }
                SidebarItem::ProjectHeader { project_key, .. } => {
                    self.sidebar_state.toggle_project(project_key);
                }
                SidebarItem::WorkspaceSectionHeader | SidebarItem::AddWorkspace => {}
            }
        }
    }

    pub fn update_selected_conversation(&mut self) {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        if let Some(item) = items.get(selected) {
            match item {
                SidebarItem::Conversation { group_key, index } => {
                    // Find the conversation
                    for group in &self.groups {
                        if &group.key() == group_key {
                            if let Some(conv) = group.conversations().get(*index) {
                                self.selected_conversation = Some(conv.clone());
                                return;
                            }
                        }
                    }
                }
                SidebarItem::GroupHeader { .. }
                | SidebarItem::EphemeralSession { .. }
                | SidebarItem::SectionControl { .. }
                | SidebarItem::OtherHeader { .. }
                | SidebarItem::ProjectHeader { .. }
                | SidebarItem::WorkspaceSectionHeader
                | SidebarItem::AddWorkspace => {
                    // No conversation selected for headers, ephemeral sessions, or section controls
                }
            }
        }
        self.selected_conversation = None;
    }

    /// Move the sidebar cursor to the item representing the currently active session.
    ///
    /// Called on exit_insert_mode() so the sidebar highlights the conversation
    /// or ephemeral session you were just viewing in the terminal.
    pub fn select_sidebar_for_active_session(&mut self) {
        let active_id = match self.active_session_id.as_ref() {
            Some(id) => id.clone(),
            None => return,
        };

        let items = self.sidebar_items();

        // Check ephemeral sessions first
        if self.ephemeral_sessions.contains_key(&active_id) {
            for (i, item) in items.iter().enumerate() {
                if let SidebarItem::EphemeralSession { session_id, .. } = item {
                    if *session_id == active_id {
                        self.sidebar_state.list_state.select(Some(i));
                        self.update_selected_conversation();
                        return;
                    }
                }
            }
        }

        // Check conversations via session_to_claude_id mapping
        if let Some(Some(claude_id)) = self.session_to_claude_id.get(&active_id) {
            for (i, item) in items.iter().enumerate() {
                if let SidebarItem::Conversation { group_key, index } = item {
                    // Look up the conversation to compare session_id
                    for group in &self.groups {
                        if &group.key() == group_key {
                            if let Some(conv) = group.conversations().get(*index) {
                                if conv.session_id == *claude_id {
                                    self.sidebar_state.list_state.select(Some(i));
                                    self.update_selected_conversation();
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Navigate to a specific conversation's sidebar position
    pub fn navigate_to_index(&mut self, index: usize) {
        self.sidebar_state.list_state.select(Some(index));
        self.update_selected_conversation();
    }

    /// Activate the sidebar filter input and enter insert mode
    pub fn activate_sidebar_filter(&mut self) {
        self.sidebar_state.activate_filter();
        self.input_mode = InputMode::Insert;
        self.escape_seq_state = EscapeSequenceState::None;
    }

    /// Deactivate the sidebar filter input (keep text visible) and return to normal mode
    pub fn deactivate_sidebar_filter(&mut self) {
        self.sidebar_state.deactivate_filter();
        self.input_mode = InputMode::Normal;
        self.escape_seq_state = EscapeSequenceState::None;
    }

    /// Clear the sidebar filter entirely and return to normal mode
    pub fn clear_sidebar_filter(&mut self) {
        self.sidebar_state.clear_filter();
        self.input_mode = InputMode::Normal;
        self.escape_seq_state = EscapeSequenceState::None;
        self.sidebar_state.list_state.select(Some(1));
        self.update_selected_conversation();
    }

    /// Whether the sidebar filter input is currently active (accepting keystrokes)
    pub fn is_sidebar_filter_active(&self) -> bool {
        self.sidebar_state.filter_active
    }
}
