//! User-facing actions on App (archive, clipboard, modals, search, etc.).

use std::path::Path;

use anyhow::Result;

use super::*;

impl App {
    /// Archive the currently selected conversation
    /// Only works for conversations that are Idle (closed/not running)
    pub fn archive_selected_conversation(&mut self) -> Result<bool> {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                // Find the conversation
                let mut conv_info: Option<(String, bool)> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            conv_info = Some((conv.session_id.clone(), conv.is_archived));
                            break;
                        }
                    }
                }

                if let Some((session_id, is_archived)) = conv_info {
                    // Check if already archived
                    if is_archived {
                        self.toast_warning("Already archived");
                        return Ok(false);
                    }

                    // Check if the conversation is running (only trust live PTY, not stale JSONL status)
                    if self.is_conversation_running(&session_id) {
                        self.toast_warning("Cannot archive active conversation");
                        return Ok(false);
                    }

                    // Archive the conversation
                    self.archive_manager.archive(&session_id, false);

                    // Update the conversation's is_archived flag in groups
                    for group in &mut self.groups {
                        if &group.key() == group_key {
                            for conv in group.conversations_mut() {
                                if conv.session_id == session_id {
                                    conv.is_archived = true;
                                    conv.archived_at = Some(chrono::Utc::now());
                                    break;
                                }
                            }
                            break;
                        }
                    }

                    // Save archive state
                    if let Err(e) = self.archive_manager.save() {
                        self.toast_error(format!("Failed to save archive: {}", e));
                        return Err(e);
                    }

                    self.toast_success("Conversation archived");
                    return Ok(true);
                }
            }
            Some(SidebarItem::EphemeralSession { .. }) => {
                self.toast_warning("Cannot archive unsaved conversation");
            }
            _ => {
                self.toast_warning("Select a conversation to archive");
            }
        }

        Ok(false)
    }

    /// Unarchive the currently selected conversation
    pub fn unarchive_selected_conversation(&mut self) -> Result<bool> {
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::Conversation { group_key, index }) => {
                // Find the conversation
                let mut conv_info: Option<(String, bool)> = None;
                for group in &self.groups {
                    if &group.key() == group_key {
                        if let Some(conv) = group.conversations().get(*index) {
                            conv_info = Some((conv.session_id.clone(), conv.is_archived));
                            break;
                        }
                    }
                }

                if let Some((session_id, is_archived)) = conv_info {
                    // Check if not archived
                    if !is_archived {
                        self.toast_warning("Not archived");
                        return Ok(false);
                    }

                    // Unarchive the conversation
                    self.archive_manager.unarchive(&session_id);

                    // Update the conversation's is_archived flag in groups
                    for group in &mut self.groups {
                        if &group.key() == group_key {
                            for conv in group.conversations_mut() {
                                if conv.session_id == session_id {
                                    conv.is_archived = false;
                                    conv.archived_at = None;
                                    break;
                                }
                            }
                            break;
                        }
                    }

                    // Save archive state
                    if let Err(e) = self.archive_manager.save() {
                        self.toast_error(format!("Failed to save archive: {}", e));
                        return Err(e);
                    }

                    self.toast_success("Conversation unarchived");
                    return Ok(true);
                }
            }
            _ => {
                self.toast_warning("Select a conversation to unarchive");
            }
        }

        Ok(false)
    }

    /// Cycle the archive filter mode (Active -> Archived -> All -> Active)
    pub fn cycle_archive_filter(&mut self) {
        self.sidebar_state.cycle_archive_filter();
        let mode = match self.sidebar_state.archive_filter {
            crate::ui::sidebar::ArchiveFilter::Active => "active",
            crate::ui::sidebar::ArchiveFilter::Archived => "archived",
            crate::ui::sidebar::ArchiveFilter::All => "all",
        };
        self.toast_info(format!("Showing {} conversations", mode));
    }

    /// Check for conversations that should be auto-archived
    /// Archives Idle conversations older than the configured threshold
    pub fn check_auto_archive(&mut self) {
        // Collect sessions that need to be archived
        let mut to_archive: Vec<String> = Vec::new();

        for group in &self.groups {
            for conv in group.conversations() {
                // Skip if already archived or if running
                if conv.is_archived {
                    continue;
                }
                if self.running_session_ids().contains(&conv.session_id) {
                    continue;
                }

                // Check if conversation should be auto-archived based on age
                if self.archive_manager.should_auto_archive(conv.timestamp) {
                    to_archive.push(conv.session_id.clone());
                }
            }
        }

        // Archive the conversations
        for session_id in &to_archive {
            self.archive_manager.archive(session_id, true);

            // Update the conversation's is_archived flag in groups
            for group in &mut self.groups {
                for conv in group.conversations_mut() {
                    if &conv.session_id == session_id {
                        conv.is_archived = true;
                        conv.archived_at = Some(chrono::Utc::now());
                        break;
                    }
                }
            }
        }

        // Save if we archived anything
        if !to_archive.is_empty() {
            if let Err(e) = self.archive_manager.save() {
                // Log error but don't show toast for auto-archive
                eprintln!("Failed to save auto-archive state: {}", e);
            }
        }
    }

    // =========================================================================
    // Clipboard Methods
    // =========================================================================

    /// Copy the selected item's project path to the clipboard
    pub fn copy_selected_path_to_clipboard(&mut self) {
        let path = match self.get_selected_path() {
            Some(p) => p.clone(),
            None => {
                self.toast_error("No path selected");
                return;
            }
        };

        // Copy to clipboard using arboard
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let path_str = path.to_string_lossy().into_owned();
            if clipboard.set_text(&path_str).is_ok() {
                self.toast_success("Copied to clipboard");
            } else {
                self.toast_error("Failed to copy");
            }
        } else {
            self.toast_error("Clipboard unavailable");
        }
    }

    /// Clear the text selection
    pub fn clear_selection(&mut self) {
        self.text_selection = None;
    }

    /// Copy the current text selection to the system clipboard
    pub fn copy_selection_to_clipboard(&mut self) {
        let text = match (&self.text_selection, &self.session_state_cache) {
            (Some(sel), Some(state)) => extract_selected_text(&state.screen, sel),
            _ => return,
        };

        if text.is_empty() {
            return;
        }

        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if clipboard.set_text(&text).is_ok() {
                let lines = text.lines().count();
                let chars = text.len();
                self.toast_success(format!("Copied selection ({lines} lines, {chars} chars)"));
            } else {
                self.toast_error("Failed to copy selection");
            }
        } else {
            self.toast_error("Clipboard unavailable");
        }
    }

    /// Get the project path of the currently selected sidebar item
    fn get_selected_path(&self) -> Option<&PathBuf> {
        // First try selected conversation
        if let Some(ref conv) = self.selected_conversation {
            return Some(&conv.project_path);
        }

        // Also support group headers and ephemeral sessions (which represent a project)
        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        match items.get(selected) {
            Some(SidebarItem::GroupHeader { key, .. }) => {
                for group in &self.groups {
                    if &group.key() == key {
                        // Get path from first conversation in group
                        if let Some(conv) = group.conversations().first() {
                            return Some(&conv.project_path);
                        }
                    }
                }
                None
            }
            Some(SidebarItem::EphemeralSession { session_id, .. }) => self
                .ephemeral_sessions
                .get(session_id)
                .map(|e| &e.project_path),
            _ => None,
        }
    }

    // =========================================================================
    // Dangerous Mode
    // =========================================================================

    /// Toggle dangerous mode on/off
    pub fn toggle_dangerous_mode(&mut self) {
        self.dangerous_mode = !self.dangerous_mode;
        self.dangerous_mode_toggled_at = Some(std::time::Instant::now());
        self.config.dangerous_mode = self.dangerous_mode;
        let _ = self.config.save();
        if self.dangerous_mode {
            self.toast_warning("Dangerous mode enabled");
        } else {
            self.toast_info("Normal mode restored");
        }
    }

    /// Check if dangerous mode was recently toggled (for temporary message display)
    /// Returns Some(true) if entering dangerous, Some(false) if exiting, None if not recent
    pub fn recent_dangerous_mode_toggle(&self, within_ms: u64) -> Option<bool> {
        self.dangerous_mode_toggled_at.and_then(|t| {
            let elapsed = t.elapsed().as_millis() as u64;
            if elapsed < within_ms {
                Some(self.dangerous_mode)
            } else {
                None
            }
        })
    }

    /// Check if clipboard copy happened recently (for UI feedback)
    pub fn recent_clipboard_copy(&self, within_ms: u64) -> Option<&str> {
        match &self.clipboard_status {
            ClipboardStatus::None => None,
            ClipboardStatus::Copied { path, at } => {
                let elapsed = at.elapsed().as_millis() as u64;
                if elapsed < within_ms {
                    Some(path.as_str())
                } else {
                    None
                }
            }
        }
    }

    // =========================================================================
    // Refresh Methods
    // =========================================================================

    /// Manual refresh triggered by user (e.g., pressing 'r').
    /// Performs a full re-sort of groups by most recent activity.
    pub fn manual_refresh(&mut self) -> Result<()> {
        self.load_conversations_full()?;
        self.cleanup_persisted_ephemeral_sessions();
        self.last_refresh = Some(std::time::Instant::now());
        self.last_refresh_was_auto = false;
        Ok(())
    }

    /// Check if a refresh happened recently (within the given duration)
    /// Returns Some((is_auto, elapsed)) if refresh was recent, None otherwise
    pub fn recent_refresh(&self, within_ms: u64) -> Option<(bool, u64)> {
        self.last_refresh.and_then(|t| {
            let elapsed = t.elapsed().as_millis() as u64;
            if elapsed < within_ms {
                Some((self.last_refresh_was_auto, elapsed))
            } else {
                None
            }
        })
    }

    // =========================================================================
    // Modal Methods
    // =========================================================================

    /// Open the new project modal dialog
    pub fn open_new_project_modal(&mut self) {
        self.modal_state = ModalState::NewProject(Box::default());
        self.input_mode = InputMode::Insert;
    }

    /// Open the search modal dialog
    pub fn open_search_modal(&mut self) {
        self.modal_state = ModalState::Search(Box::default());
        self.input_mode = InputMode::Insert;
    }

    /// Close any open modal dialog
    pub fn close_modal(&mut self) {
        self.modal_state = ModalState::None;
        self.input_mode = InputMode::Normal;
        self.escape_seq_state = EscapeSequenceState::None;
    }

    /// Perform a search with the current query in the search modal
    pub fn perform_search(&mut self) {
        if let ModalState::Search(ref mut state) = self.modal_state {
            let query = state.search_query();
            let results = self.search_engine.search(&query, &self.groups);
            state.set_results(results);
        }
    }

    /// Navigate to a conversation by session_id (from search results)
    pub fn navigate_to_conversation(&mut self, session_id: &str) -> Result<bool> {
        // Close search modal
        self.modal_state = ModalState::None;

        // Find the conversation in groups
        let mut target: Option<(String, usize, Conversation)> = None;
        for group in &self.groups {
            for (conv_idx, conv) in group.conversations().iter().enumerate() {
                if conv.session_id == session_id {
                    target = Some((group.key(), conv_idx, conv.clone()));
                    break;
                }
            }
            if target.is_some() {
                break;
            }
        }

        let Some((group_key, conv_idx, conv)) = target else {
            self.toast_error("Conversation not found");
            return Ok(false);
        };

        // Ensure the conversation is visible in the sidebar:
        // 1. Uncollapse the group
        self.sidebar_state.collapsed_groups.remove(&group_key);
        // 2. Expand conversation list to show all so this conversation is visible
        let conv_total = self
            .groups
            .iter()
            .find(|g| g.key() == group_key)
            .map(|g| g.conversations().len())
            .unwrap_or(0);
        SidebarState::show_all(
            &mut self.sidebar_state.visible_conversations,
            &group_key,
            conv_total,
        );
        // 3. For every project, ensure all groups are visible so this group isn't hidden
        for group in &self.groups {
            let pk = group.project_key();
            let group_total = self.groups.iter().filter(|g| g.project_key() == pk).count();
            SidebarState::show_all(&mut self.sidebar_state.visible_groups, &pk, group_total);
        }

        // Best-effort: select the conversation in the sidebar
        let items = self.sidebar_items();
        if let Some(item_idx) = items.iter().position(|item| {
            matches!(item, SidebarItem::Conversation { group_key: gk, index }
                if gk == &group_key && *index == conv_idx)
        }) {
            self.sidebar_state.list_state.select(Some(item_idx));
        }
        self.update_selected_conversation();

        // Open the conversation directly (don't rely on open_selected which
        // depends on the sidebar item being visible/selected)
        let project_path = conv.project_path.clone();
        if !project_path.exists() {
            self.selected_conversation = Some(conv);
            return Ok(true);
        }

        let existing_session = self
            .session_to_claude_id
            .iter()
            .find(|(_, v)| **v == Some(session_id.to_string()))
            .map(|(k, _)| k.clone());

        if let Some(sid) = existing_session {
            self.active_session_id = Some(sid);
        } else {
            self.start_session(&project_path, Some(session_id))?;
        }
        self.selected_conversation = Some(conv);
        self.focus = Focus::Terminal(TerminalPaneId::Primary);
        self.enter_insert_mode();
        Ok(true)
    }

    /// Check if a modal is currently open
    pub fn is_modal_open(&self) -> bool {
        !matches!(self.modal_state, ModalState::None)
    }

    /// Confirm the new project modal selection and start a session
    pub fn confirm_new_project(&mut self, path: &Path) -> Result<()> {
        // Close the modal first
        self.modal_state = ModalState::None;

        // Start a new session in the selected directory
        self.start_session(path, None)?;
        self.selected_conversation = None;
        self.focus = Focus::Terminal(TerminalPaneId::Primary);
        self.enter_insert_mode();

        Ok(())
    }

    /// Open the worktree creation modal for the currently selected group.
    pub fn open_worktree_modal(&mut self) {
        use crate::claude::worktree::detect_repo_info;

        let items = self.sidebar_items();
        let selected = self.sidebar_state.list_state.selected().unwrap_or(0);

        // Determine which group the cursor is in
        let group_key = Self::get_group_key_for_index(&items, selected);
        let Some(key) = group_key else {
            self.toast_error("Select a project group first");
            return;
        };

        // Find a project path from the group
        let project_path = self
            .groups
            .iter()
            .find(|g| g.key() == key)
            .and_then(ConversationGroup::project_path);

        let Some(path) = project_path else {
            self.toast_error("No project path for this group");
            return;
        };

        // Detect the repo type
        let Some(repo_info) = detect_repo_info(&path) else {
            self.toast_error("Not a git repository");
            return;
        };

        let display_name = repo_info.display_name();
        let state = WorktreeModalState::new(key, display_name);
        self.modal_state = ModalState::Worktree(Box::new(state));
        self.input_mode = InputMode::Insert;
    }

    /// Confirm worktree creation from the modal.
    pub fn confirm_worktree(&mut self, branch_name: &str) -> Result<()> {
        use crate::claude::worktree::{create_worktree, detect_repo_info};

        // Get the group key from the modal state
        let group_key = match &self.modal_state {
            ModalState::Worktree(state) => state.group_key.clone(),
            _ => return Ok(()),
        };

        // Find the project path again
        let project_path = self
            .groups
            .iter()
            .find(|g| g.key() == group_key)
            .and_then(ConversationGroup::project_path);

        let Some(path) = project_path else {
            if let ModalState::Worktree(ref mut state) = self.modal_state {
                state.error_message = Some("Group no longer exists".to_string());
            }
            return Ok(());
        };

        let Some(repo_info) = detect_repo_info(&path) else {
            if let ModalState::Worktree(ref mut state) = self.modal_state {
                state.error_message = Some("Not a git repository".to_string());
            }
            return Ok(());
        };

        match create_worktree(&repo_info, branch_name) {
            Ok(worktree_path) => {
                // Close modal
                self.modal_state = ModalState::None;

                self.toast_success(format!("Worktree '{}' created", branch_name));

                // Start a new conversation in the worktree
                self.start_session(&worktree_path, None)?;
                self.selected_conversation = None;
                self.focus = Focus::Terminal(TerminalPaneId::Primary);
                self.enter_insert_mode();
            }
            Err(e) => {
                // Keep modal open with error
                if let ModalState::Worktree(ref mut state) = self.modal_state {
                    state.error_message = Some(format!("{}", e));
                }
            }
        }
        Ok(())
    }

    /// Collect unique git projects from sidebar groups (deduplicated by repo path).
    fn collect_worktree_projects(&self) -> Vec<crate::ui::modal::WorktreeProject> {
        use crate::claude::worktree::detect_repo_info;
        use crate::ui::modal::WorktreeProject;
        use std::collections::HashSet;

        let mut seen = HashSet::new();
        let mut projects = Vec::new();

        for group in &self.groups {
            let Some(project_path) = group.project_path() else {
                continue;
            };
            let Some(repo_info) = detect_repo_info(&project_path) else {
                continue;
            };
            let repo_path = repo_info.repo_path().to_path_buf();
            if seen.insert(repo_path.clone()) {
                projects.push(WorktreeProject {
                    display_name: repo_info.display_name(),
                    project_path,
                    repo_path,
                });
            }
        }

        projects
    }

    /// Open the worktree search modal (project picker + branch input).
    pub fn open_worktree_search_modal(&mut self) {
        let projects = self.collect_worktree_projects();
        if projects.is_empty() {
            self.toast_error("No git repositories found");
            return;
        }
        let state = WorktreeSearchModalState::new(projects);
        self.modal_state = ModalState::WorktreeSearch(Box::new(state));
        self.input_mode = InputMode::Insert;
    }

    /// Confirm worktree creation from the worktree search modal.
    pub fn confirm_worktree_search(
        &mut self,
        project_path: &Path,
        branch_name: &str,
    ) -> Result<()> {
        use crate::claude::worktree::{create_worktree, detect_repo_info};

        let Some(repo_info) = detect_repo_info(project_path) else {
            if let ModalState::WorktreeSearch(ref mut state) = self.modal_state {
                state.error_message = Some("Not a git repository".to_string());
            }
            return Ok(());
        };

        match create_worktree(&repo_info, branch_name) {
            Ok(worktree_path) => {
                self.modal_state = ModalState::None;
                self.toast_success(format!("Worktree '{}' created", branch_name));
                self.start_session(&worktree_path, None)?;
                self.selected_conversation = None;
                self.focus = Focus::Terminal(TerminalPaneId::Primary);
                self.enter_insert_mode();
            }
            Err(e) => {
                if let ModalState::WorktreeSearch(ref mut state) = self.modal_state {
                    state.error_message = Some(format!("{}", e));
                }
            }
        }
        Ok(())
    }

    // =========================================================================
    // Profile Methods
    // =========================================================================

    /// Switch the active profile. `None` means "All" mode (no filtering).
    pub fn switch_profile(&mut self, idx: Option<usize>) {
        match idx {
            Some(i) => {
                if i >= self.config.profiles.len() {
                    self.toast_error("Invalid profile index");
                    return;
                }
                self.active_profile = Some(i);
                let name = &self.config.profiles[i].name;
                self.toast_success(format!("Profile: {}", name));
            }
            None => {
                self.active_profile = None;
                self.toast_success("Switched to All");
            }
        }
    }

    /// Open the profile management modal.
    pub fn open_profile_modal(&mut self) {
        let profile_names: Vec<String> = self.config.profiles.iter().map(|p| p.name.clone()).collect();
        let state = crate::ui::modal::ProfileModalState::new(profile_names, self.active_profile);
        self.modal_state = ModalState::Profile(Box::new(state));
        self.input_mode = InputMode::Insert;
    }

    /// Create a new profile from the modal.
    pub fn create_profile(&mut self, name: &str) {
        self.config.profiles.push(crate::config::ProfileEntry {
            name: name.to_string(),
            workspaces: Vec::new(),
        });
        self.save_config_silent();
        self.which_key_config
            .rebuild_with_profiles(&self.config.profiles);

        // Update modal state to reflect the change
        if let ModalState::Profile(ref mut state) = self.modal_state {
            state.profiles.push(name.to_string());
            state.selected = state.profiles.len() - 1;
            state.list_state.select(Some(state.selected));
            state.mode = crate::ui::modal::profile::ProfileModalMode::List;
            state.input_purpose = None;
            state.input_buffer.clear();
            state.cursor_pos = 0;
            state.error_message = None;
        }

        self.toast_success(format!("Profile created: {}", name));
    }

    /// Rename an existing profile from the modal.
    pub fn rename_profile(&mut self, index: usize, new_name: &str) {
        if index >= self.config.profiles.len() {
            return;
        }
        self.config.profiles[index].name = new_name.to_string();
        self.save_config_silent();
        self.which_key_config
            .rebuild_with_profiles(&self.config.profiles);

        // Update modal state
        if let ModalState::Profile(ref mut state) = self.modal_state {
            if index < state.profiles.len() {
                state.profiles[index] = new_name.to_string();
            }
            state.mode = crate::ui::modal::profile::ProfileModalMode::List;
            state.input_purpose = None;
            state.input_buffer.clear();
            state.cursor_pos = 0;
            state.error_message = None;
        }

        self.toast_success(format!("Profile renamed to: {}", new_name));
    }

    /// Delete a profile from the modal.
    pub fn delete_profile(&mut self, index: usize) {
        if index >= self.config.profiles.len() {
            return;
        }
        let removed_name = self.config.profiles.remove(index).name;
        self.save_config_silent();

        // Handle active profile index shift
        if let Some(active) = self.active_profile {
            if index == active {
                // Deleted the active profile — switch to "All" mode
                self.active_profile = None;
            } else if index < active {
                // Deleted before active — shift index down
                self.active_profile = Some(active - 1);
            }
            // index > active — no change needed
        }

        // Rebuild which-key (may clear profile entries if list is now empty)
        if self.config.has_profiles() {
            self.which_key_config
                .rebuild_with_profiles(&self.config.profiles);
        } else {
            // No profiles left — reset to default commands
            self.which_key_config = crate::input::which_key::WhichKeyConfig::new();
        }

        // Update modal state
        if let ModalState::Profile(ref mut state) = self.modal_state {
            if index < state.profiles.len() {
                state.profiles.remove(index);
            }
            // Update active marker
            state.active_profile = self.active_profile;
            // Clamp selection
            if state.profiles.is_empty() {
                state.selected = 0;
                state.list_state.select(None);
            } else {
                state.selected = state.selected.min(state.profiles.len() - 1);
                state.list_state.select(Some(state.selected));
            }
        }

        self.toast_success(format!("Profile deleted: {}", removed_name));
    }

    /// Activate a profile from the modal and close it.
    pub fn activate_profile_from_modal(&mut self, index: usize) {
        self.close_modal();
        self.switch_profile(Some(index));
    }

    // =========================================================================
    // Workspace Methods
    // =========================================================================

    /// Get a mutable reference to the active workspace list.
    /// If a profile is active, returns that profile's workspaces.
    /// If no profiles are defined, returns legacy workspaces.
    fn active_workspace_list(&self) -> &[String] {
        if let Some(idx) = self.active_profile {
            if let Some(profile) = self.config.profiles.get(idx) {
                return &profile.workspaces;
            }
        }
        &self.config.workspaces
    }

    /// Open the workspace management modal.
    pub fn open_workspace_modal(&mut self) {
        let current_workspaces = self.active_workspace_list().to_vec();
        let state = WorkspaceModalState::new(current_workspaces);
        self.modal_state = ModalState::Workspace(Box::new(state));
        self.input_mode = InputMode::Insert;
    }

    /// Add a workspace directory to the active profile (or legacy workspaces).
    pub fn add_workspace(&mut self, path: &str) {
        let workspaces = if let Some(idx) = self.active_profile {
            &mut self.config.profiles[idx].workspaces
        } else {
            &mut self.config.workspaces
        };

        if workspaces.contains(&path.to_string()) {
            self.toast_warning("Already a workspace");
            return;
        }
        workspaces.push(path.to_string());
        self.save_config_silent();

        // Rebuild which-key menu if profiles changed
        if self.active_profile.is_some() {
            self.which_key_config
                .rebuild_with_profiles(&self.config.profiles);
        }

        // Update the modal state to reflect the change
        if let ModalState::Workspace(ref mut state) = self.modal_state {
            state.workspaces.push(path.to_string());
            // Update current list selection
            if !state.workspaces.is_empty() {
                state
                    .list_state_current
                    .select(Some(state.workspaces.len() - 1));
                state.selected_current = state.workspaces.len() - 1;
            }
        }

        self.toast_success(format!("Workspace added: {}", path));
    }

    /// Remove a workspace directory from the active profile (or legacy workspaces) by index.
    pub fn remove_workspace(&mut self, index: usize) {
        let workspaces = if let Some(idx) = self.active_profile {
            &mut self.config.profiles[idx].workspaces
        } else {
            &mut self.config.workspaces
        };

        if index >= workspaces.len() {
            return;
        }
        let removed = workspaces.remove(index);
        self.save_config_silent();

        // Rebuild which-key menu if profiles changed
        if self.active_profile.is_some() {
            self.which_key_config
                .rebuild_with_profiles(&self.config.profiles);
        }

        // Update the modal state to reflect the change
        if let ModalState::Workspace(ref mut state) = self.modal_state {
            if index < state.workspaces.len() {
                state.workspaces.remove(index);
            }
            // Clamp current list selection
            if state.workspaces.is_empty() {
                state.list_state_current.select(None);
                state.selected_current = 0;
                state.focus = crate::ui::modal::workspace::WorkspaceModalFocus::AvailableList;
            } else {
                state.selected_current = state.selected_current.min(state.workspaces.len() - 1);
                state
                    .list_state_current
                    .select(Some(state.selected_current));
            }
        }

        self.toast_success(format!("Workspace removed: {}", removed));
    }

    // =========================================================================
    // Toast Helpers
    // =========================================================================

    /// Show an info toast
    pub fn toast_info(&mut self, message: impl Into<String>) {
        self.toast_manager.push(message, ToastType::Info);
    }

    /// Show a success toast
    pub fn toast_success(&mut self, message: impl Into<String>) {
        self.toast_manager.push(message, ToastType::Success);
    }

    /// Show a warning toast
    pub fn toast_warning(&mut self, message: impl Into<String>) {
        self.toast_manager.push(message, ToastType::Warning);
    }

    /// Show an error toast
    pub fn toast_error(&mut self, message: impl Into<String>) {
        self.toast_manager.push(message, ToastType::Error);
    }

    // =========================================================================
    // Layout Methods
    // =========================================================================

    /// Adjust sidebar width by delta percentage (positive = wider, negative = narrower)
    /// Clamps to valid range (10-50%) and resizes all sessions
    pub fn resize_sidebar(&mut self, delta: i8) {
        let current = self.config.layout.sidebar_width_pct as i16;
        let new_width = (current + delta as i16).clamp(10, 50) as u8;

        if new_width != self.config.layout.sidebar_width_pct {
            self.config.layout.sidebar_width_pct = new_width;
            self.save_config_silent();
            self.resize_sessions_to_layout();
            self.toast_info(format!("Sidebar: {}%", new_width));
        }
    }

    /// Toggle sidebar position between left and right
    pub fn toggle_sidebar_position(&mut self) {
        self.config.layout.sidebar_position = self.config.layout.sidebar_position.toggle();
        self.save_config_silent();
        let pos = match self.config.layout.sidebar_position {
            SidebarPosition::Left => "left",
            SidebarPosition::Right => "right",
        };
        self.toast_info(format!("Sidebar: {}", pos));
    }

    /// Toggle sidebar minimized state
    pub fn toggle_sidebar_minimized(&mut self) {
        self.config.layout.sidebar_minimized = !self.config.layout.sidebar_minimized;
        self.save_config_silent();
        self.resize_sessions_to_layout();
        if self.config.layout.sidebar_minimized {
            self.toast_info("Sidebar minimized");
        } else {
            self.toast_info("Sidebar restored");
        }
    }

    /// Save config silently (don't show error toast, just log)
    fn save_config_silent(&self) {
        if let Err(e) = self.config.save() {
            eprintln!("Failed to save config: {}", e);
        }
    }
}
