//! Session lifecycle methods on App.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;

use super::*;

impl App {
    /// Open the selected item in the terminal pane
    ///
    /// - For conversations: resumes the conversation with `claude --resume <session_id>`
    ///   If the session is already running, just switches to it.
    /// - For group headers: starts a new conversation in the group's project directory
    /// - For ephemeral sessions: switches to the already-running session
    pub fn open_selected(&mut self) -> Result<()> {
        self.clear_preview();
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                // Resume existing conversation with session_id
                let mut target: Option<(PathBuf, String, Conversation)> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            target = Some((
                                conv.project_path.clone(),
                                conv.session_id.clone(),
                                conv.clone(),
                            ));
                            break;
                        }
                    }
                }

                if let Some((path, claude_session_id, conv)) = target {
                    // Check if the working directory still exists
                    if !path.exists() {
                        // Directory was deleted (e.g., git worktree removed)
                        // Just select the conversation but don't start a session
                        self.selected_conversation = Some(conv);
                        return Ok(());
                    }

                    // Check if we already have a daemon session for this Claude session
                    let existing_session = self
                        .session_to_claude_id
                        .iter()
                        .find(|(_, v)| v.as_deref() == Some(claude_session_id.as_str()))
                        .map(|(k, _)| k.clone());

                    if let Some(session_id) = existing_session {
                        self.active_session_id = Some(session_id);
                    } else {
                        // Start new session with --resume
                        self.start_session(&path, Some(&claude_session_id))?;
                    }
                    self.selected_conversation = Some(conv);
                    self.focus = Focus::Terminal(TerminalPaneId::Primary);
                    self.enter_insert_mode();
                }
            }
            Some(SidebarItem::GroupHeader { key, .. }) => {
                // Start new conversation in the group's project directory
                let mut project_path: Option<PathBuf> = None;
                for group in &self.groups {
                    if &group.key() == key {
                        project_path = group.project_path();
                        break;
                    }
                }

                if let Some(path) = project_path {
                    self.start_session(&path, None)?;
                    self.selected_conversation = None;
                    self.focus = Focus::Terminal(TerminalPaneId::Primary);
                    self.enter_insert_mode();
                }
            }
            Some(SidebarItem::EphemeralSession { session_id, .. }) => {
                // Switch to the already-running ephemeral session
                self.active_session_id = Some(session_id.clone());
                self.selected_conversation = None;
                self.focus = Focus::Terminal(TerminalPaneId::Primary);
                self.enter_insert_mode();
            }
            Some(SidebarItem::SectionControl { key, kind, action }) => {
                // Handle section controls (show more/all/fewer/collapse)
                use crate::ui::sidebar::{ControlAction, SectionKind};
                let map = match kind {
                    SectionKind::Conversations => &mut self.sidebar_state.visible_conversations,
                    SectionKind::Groups => &mut self.sidebar_state.visible_groups,
                };
                match action {
                    ControlAction::ShowMore(hidden) => {
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
            Some(SidebarItem::OtherHeader { .. }) => {
                self.sidebar_state.toggle_other_collapsed();
            }
            Some(SidebarItem::ProjectHeader { project_key, .. }) => {
                self.sidebar_state.toggle_project(project_key);
            }
            Some(SidebarItem::AddWorkspace) => {
                self.open_workspace_modal();
            }
            Some(SidebarItem::WorkspaceSectionHeader) | None => {}
        }

        Ok(())
    }

    /// Create a new conversation in whichever group the selected sidebar item belongs to.
    ///
    /// Unlike `open_selected()`, this always starts a fresh conversation regardless
    /// of which item type is selected (GroupHeader, Conversation, EphemeralSession, etc.).
    pub fn new_conversation_in_selected_group(&mut self) -> Result<()> {
        self.clear_preview();
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        // Extract the group key from whatever item is selected
        let group_key = match items.get(selected) {
            Some(SidebarItem::GroupHeader { key, .. }) => Some(key.clone()),
            Some(SidebarItem::Conversation { group_key, .. }) => Some(group_key.clone()),
            Some(SidebarItem::EphemeralSession { group_key, .. }) => Some(group_key.clone()),
            Some(SidebarItem::SectionControl {
                key,
                kind: crate::ui::sidebar::SectionKind::Conversations,
                ..
            }) => Some(key.clone()),
            Some(SidebarItem::ProjectHeader { project_key, .. }) => {
                // Find the first group whose project_path matches this project_key
                self.groups
                    .iter()
                    .find(|g| {
                        g.project_path()
                            .map(|p| p.to_string_lossy().to_string())
                            .as_deref()
                            == Some(project_key.as_str())
                    })
                    .map(ConversationGroup::key)
            }
            _ => None,
        };

        if let Some(key) = group_key {
            // Find the group's project path
            let project_path = self
                .groups
                .iter()
                .find(|g| g.key() == key)
                .and_then(ConversationGroup::project_path);

            if let Some(path) = project_path {
                self.selected_conversation = None;
                self.start_session(&path, None)?;

                // Position sidebar cursor on the newly created ephemeral session
                if let Some(ref new_sid) = self.active_session_id {
                    let new_items = self.sidebar_items();
                    if let Some(idx) = new_items.iter().position(|item| {
                        matches!(item, SidebarItem::EphemeralSession { session_id, .. } if session_id == new_sid)
                    }) {
                        self.sidebar_state.list_state.select(Some(idx));
                    }
                }

                self.focus = Focus::Terminal(TerminalPaneId::Primary);
                self.enter_insert_mode();
            }
        }

        Ok(())
    }

    /// Preview the selected sidebar item in the terminal pane without leaving the sidebar.
    ///
    /// Shows the session output in the terminal pane but keeps focus on the sidebar
    /// and stays in normal mode. Pressing `p` again on the same conversation is a
    /// no-op; use `clear_preview()` (Escape) to exit preview.
    pub fn preview_selected(&mut self) -> Result<()> {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                let mut target: Option<(PathBuf, String)> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            target = Some((conv.project_path.clone(), conv.session_id.clone()));
                            break;
                        }
                    }
                }

                if let Some((path, claude_session_id)) = target {
                    // Toggle: if already previewing this session, clear preview
                    let existing_session = self
                        .session_to_claude_id
                        .iter()
                        .find(|(_, v)| v.as_deref() == Some(claude_session_id.as_str()))
                        .map(|(k, _)| k.clone());

                    if let Some(ref sid) = existing_session {
                        if self.preview_session_id.as_ref() == Some(sid) {
                            return Ok(());
                        }
                    }

                    // Check if directory still exists
                    if !path.exists() {
                        return Ok(());
                    }

                    if let Some(session_id) = existing_session {
                        self.preview_session_id = Some(session_id);
                    } else {
                        // Need to spawn a session — save/restore active_session_id
                        let prev_active = self.active_session_id.clone();
                        self.start_session(&path, Some(&claude_session_id))?;
                        let new_session_id = self.active_session_id.clone();
                        self.active_session_id = prev_active;
                        self.preview_session_id = new_session_id;
                    }
                }
            }
            Some(SidebarItem::EphemeralSession { session_id, .. }) => {
                // Already previewing this session — no-op
                if self.preview_session_id.as_ref() == Some(session_id) {
                    return Ok(());
                }
                self.preview_session_id = Some(session_id.clone());
            }
            _ => {}
        }

        Ok(())
    }

    /// Clear the preview session
    pub fn clear_preview(&mut self) {
        self.preview_session_id = None;
    }

    /// Start a new session (or resume one) in the given directory.
    ///
    /// If `claude_session_id` is provided, resumes an existing conversation.
    /// Otherwise starts a new conversation.
    pub(crate) fn start_session(
        &mut self,
        working_dir: &std::path::Path,
        claude_session_id: Option<&str>,
    ) -> Result<()> {
        let (rows, cols) = self.calculate_terminal_dimensions();

        let result = self.session_manager.create_session(
            working_dir,
            claude_session_id,
            rows,
            cols,
            self.dangerous_mode,
        );

        match result {
            Ok(session_id) => {
                // Track the mapping from session ID to Claude session
                self.session_to_claude_id.insert(
                    session_id.clone(),
                    claude_session_id.map(ToString::to_string),
                );

                // Track ephemeral sessions (new sessions without a saved conversation file)
                if claude_session_id.is_none() {
                    self.ephemeral_sessions.insert(
                        session_id.clone(),
                        EphemeralSession {
                            project_path: working_dir.to_path_buf(),
                            created_at: chrono::Utc::now().timestamp_millis(),
                        },
                    );
                }

                self.active_session_id = Some(session_id);

                self.toast_success("Session started");
                Ok(())
            }
            Err(e) => {
                self.toast_error(format!("Failed to start session: {}", e));
                Err(e)
            }
        }
    }

    /// Get the cached session state for rendering
    pub fn get_session_state(&self) -> Option<&SessionState> {
        self.session_state_cache.as_ref()
    }

    /// Check if a specific conversation has an active PTY session.
    pub fn is_conversation_running(&self, session_id: &str) -> bool {
        self.session_to_claude_id
            .values()
            .any(|claude_id| claude_id.as_deref() == Some(session_id))
    }

    /// Get set of running session IDs for sidebar display
    pub fn running_session_ids(&self) -> HashSet<String> {
        // Return Claude session IDs for sessions that are running
        self.session_to_claude_id
            .iter()
            .filter_map(|(_, claude_id)| claude_id.clone())
            .collect()
    }

    /// Update session state cache (call this in the main loop after processing output)
    ///
    /// Prefers preview_session_id when set (preview mode shows the previewed session),
    /// otherwise falls back to active_session_id.
    pub fn update_session_state(&mut self) {
        let display_session = self
            .preview_session_id
            .as_ref()
            .or(self.active_session_id.as_ref());

        // Clear selection if the displayed session changed
        let old_session_id = self
            .session_state_cache
            .as_ref()
            .map(|s| s.session_id.clone());
        let new_session_id = display_session.cloned();
        if old_session_id != new_session_id {
            self.text_selection = None;
        }

        if let Some(session_id) = display_session {
            self.session_state_cache = self.session_manager.get_session_state(session_id);
        } else {
            self.session_state_cache = None;
        }
    }

    /// Get the session ID currently displayed (preview takes priority over active).
    /// Mirrors the logic in `update_session_state`.
    fn display_session_id(&self) -> Option<String> {
        self.preview_session_id
            .clone()
            .or(self.active_session_id.clone())
    }

    /// Scroll up by the specified number of lines in the displayed session
    pub fn scroll_up(&mut self, lines: usize) {
        self.text_selection = None;
        if let Some(ref session_id) = self.display_session_id() {
            if let Some(session) = self.session_manager.get_session_mut(session_id) {
                session.scroll_up(lines);
            }
        }
    }

    /// Scroll down by the specified number of lines in the displayed session
    pub fn scroll_down(&mut self, lines: usize) {
        self.text_selection = None;
        if let Some(ref session_id) = self.display_session_id() {
            if let Some(session) = self.session_manager.get_session_mut(session_id) {
                session.scroll_down(lines);
            }
        }
    }

    /// Jump to the bottom (live view) for the displayed session
    pub fn scroll_to_bottom(&mut self) {
        if let Some(ref session_id) = self.display_session_id() {
            if let Some(session) = self.session_manager.get_session_mut(session_id) {
                session.scroll_to_bottom();
            }
        }
    }

    /// Check if active session is scroll locked
    pub fn is_scroll_locked(&self) -> bool {
        self.session_state_cache
            .as_ref()
            .map(|s| s.scroll_locked)
            .unwrap_or(false)
    }

    /// Write input to active session's PTY
    pub fn write_to_pty(&mut self, data: &[u8]) -> Result<()> {
        // Clone needed: immutable borrow of session_id + mutable borrow of session_manager
        if let Some(ref session_id) = self.active_session_id.clone() {
            if let Some(session) = self.session_manager.get_session_mut(session_id) {
                session.write(data)?;
            }
        }
        Ok(())
    }

    /// Resize all running sessions
    pub fn resize(&mut self, width: u16, height: u16) -> Result<()> {
        self.text_selection = None;
        self.term_size = (width, height);

        let (rows, cols) = self.calculate_terminal_dimensions();
        self.session_manager.resize_all(rows, cols);

        Ok(())
    }

    /// Calculate terminal pane dimensions based on current config and term size
    pub(crate) fn calculate_terminal_dimensions(&self) -> (u16, u16) {
        let sidebar_pct = if self.config.layout.sidebar_minimized {
            3 // Minimized sidebar takes ~3 columns
        } else {
            self.config.layout.sidebar_width_pct as u16
        };

        let terminal_pct = 100u16.saturating_sub(sidebar_pct);
        let cols = (self.term_size.0 * terminal_pct / 100).saturating_sub(2);
        let rows = self.term_size.1.saturating_sub(3); // Account for borders and help bar

        (rows, cols)
    }

    /// Check all sessions for dead PTYs and clean up
    pub fn check_all_session_status(&mut self) {
        // Clean up dead sessions from the session manager
        let dead_sessions = self.session_manager.cleanup_dead();

        // Before destroying ephemeral data, give unmatched ephemerals one last
        // chance to match a persisted conversation. Without this, the race
        // between cleanup and session-index reload causes the conversation to
        // disappear when `hide_inactive` is on.
        let has_unmatched_dead_ephemerals = dead_sessions.iter().any(|sid| {
            self.ephemeral_sessions.contains_key(sid)
                && self
                    .session_to_claude_id
                    .get(sid)
                    .map(Option::is_none)
                    .unwrap_or(false)
        });

        if has_unmatched_dead_ephemerals {
            let _ = self.load_conversations_preserve_order();
            self.cleanup_persisted_ephemeral_sessions();
        }

        for session_id in dead_sessions {
            self.session_to_claude_id.remove(&session_id);

            // Remove from ephemeral sessions if present
            self.ephemeral_sessions.remove(&session_id);

            // Clear preview if the dead session was being previewed
            if self.preview_session_id.as_ref() == Some(&session_id) {
                self.preview_session_id = None;
            }

            // Clear active_session_id if it was the dead one
            if self.active_session_id.as_ref() == Some(&session_id) {
                self.active_session_id = None;
                self.session_state_cache = None;
                self.toast_info("Session ended");
                // Return focus to sidebar when viewed session closes
                if matches!(self.focus, Focus::Terminal(_)) {
                    self.focus = Focus::Sidebar;
                }
            }
        }
    }

    /// Check for sessions-index.json changes and reload conversations if needed.
    /// Uses preserve-order loading to maintain stable group positions during auto-refresh.
    pub fn check_sessions_updates(&mut self) {
        if let Some(ref watcher) = self.sessions_watcher {
            // Drain all pending notifications
            let mut should_reload = false;
            while watcher.try_recv().is_some() {
                should_reload = true;
            }

            if should_reload && self.load_conversations_preserve_order().is_ok() {
                self.cleanup_persisted_ephemeral_sessions();
                self.last_refresh = Some(Instant::now());
                self.last_refresh_was_auto = true;
            }
        }
    }

    /// Remove ephemeral sessions that have been persisted to disk
    /// and update session mappings to point to the discovered conversations.
    ///
    /// When a new conversation is started, it appears in `ephemeral_sessions` until
    /// Claude writes the .jsonl file. Once the file exists and is discovered during
    /// `load_conversations()`, we need to:
    /// 1. Update `session_to_claude_id` to map daemon_id -> actual Claude session ID
    /// 2. Update `selected_conversation` if this is the active session
    /// 3. Remove the ephemeral entry to avoid duplicate sidebar entries
    ///
    /// The matching algorithm ensures correctness by:
    /// - Only matching conversations created AFTER the ephemeral session was started
    /// - Not matching conversations already claimed by another daemon session
    pub(crate) fn cleanup_persisted_ephemeral_sessions(&mut self) {
        // Build list of all conversations
        let all_convs: Vec<&Conversation> = self
            .groups
            .iter()
            .flat_map(ConversationGroup::conversations)
            .collect();

        // Track which Claude session IDs are already claimed by a daemon
        let mut claimed_ids: HashSet<String> = self
            .session_to_claude_id
            .values()
            .filter_map(Clone::clone)
            .collect();

        // Collect ephemeral sessions to process (avoid borrow issues)
        let ephemeral_to_process: Vec<(String, EphemeralSession)> = self
            .ephemeral_sessions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // For each ephemeral session, find a matching conversation
        for (daemon_id, ephemeral) in ephemeral_to_process {
            // Check if this ephemeral session's Claude ID is still None
            let needs_update = self
                .session_to_claude_id
                .get(&daemon_id)
                .map(Option::is_none)
                .unwrap_or(false);

            if !needs_update {
                continue;
            }

            // Find matching conversation:
            // 1. Same project path
            // 2. Created AFTER this ephemeral session was started
            // 3. Not already claimed by another daemon session
            let matching_conv = all_convs
                .iter()
                .filter(|c| c.project_path == ephemeral.project_path)
                .filter(|c| c.timestamp > ephemeral.created_at)
                .filter(|c| !claimed_ids.contains(&c.session_id))
                .min_by_key(|c| c.timestamp); // Oldest matching = most likely match

            if let Some(conv) = matching_conv {
                let conv_session_id = conv.session_id.clone();

                // Update the daemon → Claude ID mapping
                self.session_to_claude_id
                    .insert(daemon_id.clone(), Some(conv_session_id.clone()));

                // Mark this conversation as claimed so no other ephemeral can match it
                claimed_ids.insert(conv_session_id);

                // If this is the active session, update selected_conversation
                if self.active_session_id.as_ref() == Some(&daemon_id) {
                    self.selected_conversation = Some((*conv).clone());
                }

                // Remove from ephemeral_sessions
                self.ephemeral_sessions.remove(&daemon_id);
            }
        }
    }

    /// Close a session by its ID, cleaning up all associated state
    pub fn close_session(&mut self, session_id: &str) {
        self.session_manager.close_session(session_id);
        self.session_to_claude_id.remove(session_id);
        self.ephemeral_sessions.remove(session_id);

        // Clear preview if we're closing the previewed session
        if self.preview_session_id.as_ref() == Some(&session_id.to_string()) {
            self.preview_session_id = None;
        }

        if self.active_session_id.as_ref() == Some(&session_id.to_string()) {
            self.active_session_id = None;
            self.session_state_cache = None;
            if matches!(self.focus, Focus::Terminal(_)) {
                self.focus = Focus::Sidebar;
            }
        }

        self.toast_info("Session closed");
    }

    /// Close the currently selected session in the sidebar
    ///
    /// For conversations: finds the running daemon session and closes it
    /// For ephemeral sessions: closes the session directly
    /// For other items (headers, show more): no-op
    pub fn close_selected_session(&mut self) {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                // Find the conversation's Claude session ID
                let mut claude_session_id: Option<String> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            claude_session_id = Some(conv.session_id.clone());
                            break;
                        }
                    }
                }

                if let Some(cid) = claude_session_id {
                    // Find the daemon session ID that maps to this Claude session
                    let daemon_session_id = self
                        .session_to_claude_id
                        .iter()
                        .find(|(_, v)| **v == Some(cid.clone()))
                        .map(|(k, _)| k.clone());

                    if let Some(dsid) = daemon_session_id {
                        self.close_session(&dsid);
                    }
                }
            }
            Some(SidebarItem::EphemeralSession { session_id, .. }) => {
                // Close the ephemeral session directly
                let session_id = session_id.clone();
                self.close_session(&session_id);
            }
            Some(
                SidebarItem::GroupHeader { .. }
                | SidebarItem::OtherHeader { .. }
                | SidebarItem::SectionControl { .. }
                | SidebarItem::ProjectHeader { .. }
                | SidebarItem::WorkspaceSectionHeader
                | SidebarItem::AddWorkspace,
            )
            | None => {
                // No-op for non-session items
            }
        }
    }

    /// Get ordered list of active PTY session IDs (sorted for stable display).
    pub fn active_pty_session_ids_ordered(&self) -> Vec<String> {
        let mut ids = self.session_manager.session_ids();
        ids.sort();
        ids
    }

    /// Update the mosaic state cache from active PTY sessions.
    /// No-ops when not in mosaic mode.
    pub fn update_mosaic_state_cache(&mut self) {
        if self.split_mode != SplitMode::Mosaic {
            return;
        }

        // Save currently selected session ID before rebuilding
        let selected_sid = self
            .mosaic_state_cache
            .get(self.mosaic_selected)
            .map(|(sid, _, _)| sid.clone());

        let ids = self.active_pty_session_ids_ordered();
        self.mosaic_state_cache = ids
            .iter()
            .filter_map(|sid| {
                let state = self.session_manager.get_session_state(sid)?;
                let name = self.session_display_name(sid);
                Some((sid.clone(), name, state))
            })
            .collect();

        // Restore selection by session ID, or clamp if session is gone
        if !self.mosaic_state_cache.is_empty() {
            if let Some(ref sid) = selected_sid {
                if let Some(new_idx) = self
                    .mosaic_state_cache
                    .iter()
                    .position(|(cache_sid, _, _)| cache_sid == sid)
                {
                    self.mosaic_selected = new_idx;
                } else {
                    self.mosaic_selected =
                        self.mosaic_selected.min(self.mosaic_state_cache.len() - 1);
                }
            } else {
                self.mosaic_selected = self.mosaic_selected.min(self.mosaic_state_cache.len() - 1);
            }
        } else {
            self.mosaic_selected = 0;
        }
    }

    /// Derive a human-readable label for a session (used in mosaic pane titles).
    pub fn session_display_name(&self, session_id: &str) -> String {
        // Check ephemeral sessions first
        if let Some(eph) = self.ephemeral_sessions.get(session_id) {
            if let Some(name) = eph.project_path.file_name() {
                return name.to_string_lossy().to_string();
            }
        }

        // Check session_to_claude_id -> find conversation in groups
        if let Some(Some(claude_id)) = self.session_to_claude_id.get(session_id) {
            for group in &self.groups {
                for conv in group.conversations() {
                    if conv.session_id == *claude_id {
                        if let Some(name) = conv.project_path.file_name() {
                            return name.to_string_lossy().to_string();
                        }
                    }
                }
            }
        }

        // Fallback to session ID
        session_id.to_string()
    }

    /// Toggle mosaic view on/off.
    pub fn toggle_mosaic_view(&mut self) {
        if self.split_mode == SplitMode::Mosaic {
            self.split_mode = SplitMode::None;
            self.focus = Focus::Sidebar;
            self.mosaic_state_cache.clear();
        } else {
            self.split_mode = SplitMode::Mosaic;
            self.focus = Focus::Mosaic;
            self.mosaic_selected = 0;
        }
    }

    /// Resize all sessions to match current layout config
    pub(crate) fn resize_sessions_to_layout(&mut self) {
        let (rows, cols) = self.calculate_terminal_dimensions();
        self.session_manager.resize_all(rows, cols);
    }
}
