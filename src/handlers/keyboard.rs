use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{self, App, ChordState, EscapeSequenceState, Focus};
use crate::input::which_key::{LeaderAction, LeaderKeyResult};
use crate::input::InputMode;
use crate::ui::sidebar::FilterKeyResult;

use super::modal::forward_key_to_modal;
use crate::event_loop::{HotReloadStatus, KeyAction};

/// Result of processing a key through the jk/kj escape sequence state machine
pub(crate) enum EscapeSeqResult {
    /// Key was buffered as first of potential escape sequence — do nothing yet
    Buffered,
    /// jk or kj detected — exit insert mode
    Escaped,
    /// Previous buffered key expired by a new j/k — flush old key, new key is now buffered
    FlushBuffered(KeyEvent),
    /// Previous buffered key invalidated by non-j/k — flush old key, process new key normally
    FlushAndProcess(KeyEvent, KeyEvent),
    /// Not a trigger key (not j or k) and no pending state — pass through directly
    PassThrough,
}

/// Process a key through the jk/kj escape sequence state machine.
///
/// Only `j` and `k` with no modifiers are trigger keys.
/// When a trigger key is pressed:
/// - If no pending state: buffer it (Buffered)
/// - If pending with the complementary key: escape detected (Escaped)
/// - If pending with the same key (jj/kk): flush old, buffer new (FlushBuffered)
///
/// When a non-trigger key is pressed:
/// - If pending: flush buffered + process current (FlushAndProcess)
/// - If no pending: pass through directly (PassThrough)
pub(crate) fn try_escape_sequence(app: &mut App, key: KeyEvent) -> EscapeSeqResult {
    let trigger_char = match key.code {
        KeyCode::Char(c @ ('j' | 'k')) if key.modifiers == KeyModifiers::NONE => Some(c),
        _ => None,
    };

    match std::mem::replace(&mut app.escape_seq_state, EscapeSequenceState::None) {
        EscapeSequenceState::None => {
            if let Some(c) = trigger_char {
                // Buffer this key as potential first of escape sequence
                app.escape_seq_state = EscapeSequenceState::Pending {
                    first_key: c,
                    first_key_event: key,
                    started_at: Instant::now(),
                };
                EscapeSeqResult::Buffered
            } else {
                EscapeSeqResult::PassThrough
            }
        }
        EscapeSequenceState::Pending {
            first_key,
            first_key_event,
            started_at,
        } => {
            let elapsed = started_at.elapsed().as_millis() as u64;

            if elapsed > app::ESCAPE_SEQ_TIMEOUT_MS {
                // Timed out — flush old key, process new key from scratch
                if let Some(c) = trigger_char {
                    // New trigger key: buffer it as new pending
                    app.escape_seq_state = EscapeSequenceState::Pending {
                        first_key: c,
                        first_key_event: key,
                        started_at: Instant::now(),
                    };
                    EscapeSeqResult::FlushBuffered(first_key_event)
                } else {
                    EscapeSeqResult::FlushAndProcess(first_key_event, key)
                }
            } else if let Some(c) = trigger_char {
                if c != first_key {
                    // Complementary key (j→k or k→j) within timeout — escape!
                    EscapeSeqResult::Escaped
                } else {
                    // Same key (jj or kk) — flush old, buffer new
                    app.escape_seq_state = EscapeSequenceState::Pending {
                        first_key: c,
                        first_key_event: key,
                        started_at: Instant::now(),
                    };
                    EscapeSeqResult::FlushBuffered(first_key_event)
                }
            } else {
                // Non-trigger key while pending — flush old, process new normally
                EscapeSeqResult::FlushAndProcess(first_key_event, key)
            }
        }
    }
}

pub(crate) fn handle_key_event(
    app: &mut App,
    key: KeyEvent,
    hot_reload_status: &mut HotReloadStatus,
) -> Result<KeyAction> {
    // Clear build failed status on any key
    if matches!(hot_reload_status, HotReloadStatus::BuildFailed(_)) {
        *hot_reload_status = HotReloadStatus::None;
    }

    // 0. If there's an active text selection, clear it on any keypress
    // (Selection auto-copies on mouse release, so no special handling needed)
    if app.text_selection.is_some() {
        app.clear_selection();
    }

    // 0.5. True global keybindings — must work even in Insert mode
    if let (KeyCode::Char('q'), KeyModifiers::CONTROL) = (key.code, key.modifiers) {
        return Ok(KeyAction::Quit);
    }

    // 1. Insert mode - handle filter, modal, or terminal passthrough with jk/kj escape detection
    if matches!(app.input_mode, InputMode::Insert) {
        if app.is_sidebar_filter_active() {
            return handle_filter_insert_with_escape_seq(app, key);
        } else if app.is_modal_open() {
            return handle_modal_insert_with_escape_seq(app, key);
        } else if matches!(app.focus, Focus::Terminal(_)) {
            return handle_terminal_insert_with_escape_seq(app, key);
        }
        // Fallback: if in insert mode but not in filter, modal, or terminal, exit insert mode
        app.exit_insert_mode();
    }

    // 2. Leader mode (works in both sidebar and terminal focus)
    if let InputMode::Leader(ref _state) = app.input_mode {
        return handle_leader_key(app, key);
    }

    // 3. Normal-mode keybindings (Ctrl+Q handled above in true globals)
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) if app.focus == Focus::Sidebar => {
            return Ok(KeyAction::Quit);
        }
        // Hot reload: Ctrl+Shift+B (B for Build)
        (KeyCode::Char('B'), KeyModifiers::CONTROL | KeyModifiers::SHIFT) => {
            return Ok(KeyAction::HotReload);
        }
        // Cycle between active projects (works from any pane)
        (KeyCode::Char('.'), KeyModifiers::ALT) => {
            let _ = app.cycle_and_switch_to_active(true);
            return Ok(KeyAction::Continue);
        }
        (KeyCode::Char(','), KeyModifiers::ALT) => {
            let _ = app.cycle_and_switch_to_active(false);
            return Ok(KeyAction::Continue);
        }
        // Layout keybindings
        (KeyCode::Char('>'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)
        | (KeyCode::Char('.'), KeyModifiers::CONTROL) => {
            // Increase sidebar width
            app.resize_sidebar(5);
            return Ok(KeyAction::Continue);
        }
        (KeyCode::Char('<'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)
        | (KeyCode::Char(','), KeyModifiers::CONTROL) => {
            // Decrease sidebar width
            app.resize_sidebar(-5);
            return Ok(KeyAction::Continue);
        }
        (KeyCode::Char('|'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)
        | (KeyCode::Char('\\'), KeyModifiers::ALT) => {
            // Toggle sidebar position (left/right)
            // Use Ctrl+Shift+\ or Alt+\ to avoid conflict with SIGQUIT
            app.toggle_sidebar_position();
            return Ok(KeyAction::Continue);
        }
        (KeyCode::Char('b'), KeyModifiers::ALT) => {
            // Toggle sidebar minimized (Alt+B to avoid conflict with Ctrl+B in terminals)
            app.toggle_sidebar_minimized();
            return Ok(KeyAction::Continue);
        }
        _ => {}
    }

    // 4. Normal mode - always sidebar (Normal mode = Sidebar focus)
    handle_sidebar_key_normal(app, key)
}

/// Handle key input in leader mode (works in both sidebar and terminal)
pub(crate) fn handle_leader_key(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
    // Escape or Space cancels leader mode
    if key.code == KeyCode::Esc || key.code == KeyCode::Char(' ') {
        app.exit_leader_mode();
        return Ok(KeyAction::Continue);
    }

    // Get the key character
    let KeyCode::Char(c) = key.code else {
        // Non-character keys cancel leader mode
        app.exit_leader_mode();
        return Ok(KeyAction::Continue);
    };

    // Get the current path and pending escape state from leader state
    let (path, pending_escape) = if let InputMode::Leader(ref state) = app.input_mode {
        (state.path.clone(), state.pending_escape)
    } else {
        // Shouldn't happen, but handle gracefully
        return Ok(KeyAction::Continue);
    };

    // Check if this key completes a jk/kj escape sequence
    if let Some((first_key, started_at)) = pending_escape {
        let is_complement = matches!((first_key, c), ('j', 'k') | ('k', 'j'));
        if is_complement
            && started_at.elapsed().as_millis() as u64 <= crate::app::ESCAPE_SEQ_TIMEOUT_MS
        {
            app.exit_leader_mode();
            return Ok(KeyAction::Continue);
        }
        // Not a valid escape sequence — clear pending and fall through to process_key
        if let InputMode::Leader(ref mut state) = app.input_mode {
            state.pending_escape = None;
        }
    }

    // Process the key through the which-key config
    match app.which_key_config.process_key(&path, c) {
        LeaderKeyResult::Execute(action) => {
            app.exit_leader_mode();
            execute_leader_action(app, action)?;
        }
        LeaderKeyResult::Submenu => {
            // Navigate into submenu by adding key to path
            if let InputMode::Leader(ref mut state) = app.input_mode {
                state.pending_escape = None;
                state.push(c);
            }
        }
        LeaderKeyResult::Cancel => {
            // If j or k, buffer as potential escape sequence start
            if c == 'j' || c == 'k' {
                if let InputMode::Leader(ref mut state) = app.input_mode {
                    state.pending_escape = Some((c, Instant::now()));
                }
            } else {
                // Other invalid keys cancel leader mode immediately
                app.exit_leader_mode();
            }
        }
    }

    Ok(KeyAction::Continue)
}

/// Execute a leader action
pub(crate) fn execute_leader_action(app: &mut App, action: LeaderAction) -> Result<()> {
    match action {
        LeaderAction::SearchOpen => {
            app.open_search_modal();
        }
        LeaderAction::NewProject => {
            app.open_new_project_modal();
        }
        LeaderAction::CloseSession => {
            app.close_selected_session();
        }
        LeaderAction::Archive => {
            app.archive_selected_conversation()?;
        }
        LeaderAction::Unarchive => {
            app.unarchive_selected_conversation()?;
        }
        LeaderAction::CycleArchiveFilter => {
            app.cycle_archive_filter();
        }
        LeaderAction::Refresh => {
            app.manual_refresh()?;
        }
        LeaderAction::YankPath => {
            app.copy_selected_path_to_clipboard();
        }
        LeaderAction::ToggleDangerous => {
            app.toggle_dangerous_mode();
        }
        LeaderAction::AddConversation => {
            app.new_conversation_in_selected_group()?;
        }
        LeaderAction::CreateWorktree => {
            app.open_worktree_modal();
        }
        LeaderAction::WorktreeSearch => {
            app.open_worktree_search_modal();
        }
        LeaderAction::ManageWorkspaces => {
            app.open_workspace_modal();
        }
    }
    Ok(())
}

/// Handle terminal insert mode with jk/kj escape sequence detection
pub(crate) fn handle_terminal_insert_with_escape_seq(
    app: &mut App,
    key: KeyEvent,
) -> Result<KeyAction> {
    match try_escape_sequence(app, key) {
        EscapeSeqResult::Buffered => {
            // Key buffered, waiting for second key or timeout
        }
        EscapeSeqResult::Escaped => {
            app.exit_insert_mode();
            app.set_focus(Focus::Sidebar);
        }
        EscapeSeqResult::FlushBuffered(buffered) => {
            // Send the old buffered key to PTY
            let bytes = key_to_bytes(buffered);
            if !bytes.is_empty() {
                app.write_to_pty(&bytes)?;
            }
        }
        EscapeSeqResult::FlushAndProcess(buffered, current) => {
            // Send the old buffered key, then process the current key normally
            let bytes = key_to_bytes(buffered);
            if !bytes.is_empty() {
                app.write_to_pty(&bytes)?;
            }
            let bytes = key_to_bytes(current);
            if !bytes.is_empty() {
                app.write_to_pty(&bytes)?;
            }
        }
        EscapeSeqResult::PassThrough => {
            // Not a trigger key — pass directly to PTY
            let bytes = key_to_bytes(key);
            if !bytes.is_empty() {
                app.write_to_pty(&bytes)?;
            }
        }
    }
    Ok(KeyAction::Continue)
}

/// Handle modal insert mode with jk/kj escape sequence detection
pub(crate) fn handle_modal_insert_with_escape_seq(
    app: &mut App,
    key: KeyEvent,
) -> Result<KeyAction> {
    // Escape handling: WorktreeSearch Phase 2 goes back to Phase 1 instead of closing
    if key.code == KeyCode::Esc {
        if let crate::app::ModalState::WorktreeSearch(ref state) = app.modal_state {
            if state.phase == crate::ui::modal::worktree_search::WorktreeSearchPhase::BranchInput {
                // Forward Esc to the modal so it can transition back to Phase 1
                forward_key_to_modal(app, key)?;
                return Ok(KeyAction::Continue);
            }
        }
        app.close_modal();
        return Ok(KeyAction::Continue);
    }

    match try_escape_sequence(app, key) {
        EscapeSeqResult::Buffered => {
            // Key buffered, waiting for second key or timeout
        }
        EscapeSeqResult::Escaped => {
            // Close modal and exit insert mode
            app.close_modal();
        }
        EscapeSeqResult::FlushBuffered(buffered) => {
            // Send old buffered key to modal
            forward_key_to_modal(app, buffered)?;
        }
        EscapeSeqResult::FlushAndProcess(buffered, current) => {
            // Send old buffered key, then current key to modal
            forward_key_to_modal(app, buffered)?;
            forward_key_to_modal(app, current)?;
        }
        EscapeSeqResult::PassThrough => {
            // Not a trigger key — send directly to modal
            forward_key_to_modal(app, key)?;
        }
    }
    Ok(KeyAction::Continue)
}

/// Handle sidebar filter insert mode with jk/kj escape sequence detection
pub(crate) fn handle_filter_insert_with_escape_seq(
    app: &mut App,
    key: KeyEvent,
) -> Result<KeyAction> {
    // Escape always clears the filter
    if key.code == KeyCode::Esc {
        app.clear_sidebar_filter();
        return Ok(KeyAction::Continue);
    }

    match try_escape_sequence(app, key) {
        EscapeSeqResult::Buffered => {
            // Key buffered, waiting for second key or timeout
        }
        EscapeSeqResult::Escaped => {
            // jk/kj detected — clear filter and return to normal
            app.clear_sidebar_filter();
        }
        EscapeSeqResult::FlushBuffered(buffered) => {
            forward_key_to_filter(app, buffered);
        }
        EscapeSeqResult::FlushAndProcess(buffered, current) => {
            forward_key_to_filter(app, buffered);
            forward_key_to_filter(app, current);
        }
        EscapeSeqResult::PassThrough => {
            forward_key_to_filter(app, key);
        }
    }
    Ok(KeyAction::Continue)
}

/// Forward a key event to the sidebar filter input
pub(crate) fn forward_key_to_filter(app: &mut App, key: KeyEvent) {
    match app.sidebar_state.handle_filter_key(key) {
        FilterKeyResult::Continue => {}
        FilterKeyResult::QueryChanged => {
            // Reset selection to top and update
            app.sidebar_state.list_state.select(Some(1));
            app.update_selected_conversation();
        }
        FilterKeyResult::Deactivated => {
            // Enter pressed — keep filter text, exit insert mode
            app.deactivate_sidebar_filter();
        }
    }
}

/// Flush a buffered escape sequence key to the appropriate target (filter, PTY, or modal)
pub(crate) fn flush_buffered_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if matches!(app.input_mode, InputMode::Insert) {
        if app.is_sidebar_filter_active() {
            forward_key_to_filter(app, key);
        } else if app.is_modal_open() {
            forward_key_to_modal(app, key)?;
        } else if matches!(app.focus, Focus::Terminal(_)) {
            let bytes = key_to_bytes(key);
            if !bytes.is_empty() {
                app.write_to_pty(&bytes)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn handle_sidebar_key_normal(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
    // Handle chord sequences first
    match &app.chord_state {
        ChordState::DeletePending { .. } => {
            if key.code == KeyCode::Char('d') {
                // Second 'd' pressed - close selected session
                app.chord_state = ChordState::None;
                app.close_selected_session();
                return Ok(KeyAction::Continue);
            } else {
                // Any other key cancels the chord
                app.chord_state = ChordState::None;
                // Fall through to handle the key normally
            }
        }
        ChordState::CountPending { count, .. } => {
            let count = *count;
            match key.code {
                // Accumulate digits (0-9)
                KeyCode::Char(c @ '0'..='9') => {
                    let digit = (c as u32) - ('0' as u32);
                    // Cap at 9999 to prevent overflow
                    let new_count = (count * 10 + digit).min(9999);
                    app.chord_state = ChordState::CountPending {
                        count: new_count,
                        started_at: Instant::now(),
                    };
                    return Ok(KeyAction::Continue);
                }
                // Execute motion with count
                KeyCode::Char('j') | KeyCode::Down => {
                    app.chord_state = ChordState::None;
                    app.navigate_down_by(count as usize);
                    return Ok(KeyAction::Continue);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    app.chord_state = ChordState::None;
                    app.navigate_up_by(count as usize);
                    return Ok(KeyAction::Continue);
                }
                // Escape cancels
                KeyCode::Esc => {
                    app.chord_state = ChordState::None;
                    return Ok(KeyAction::Continue);
                }
                // Any other key cancels count and is processed normally
                _ => {
                    app.chord_state = ChordState::None;
                    // Fall through to handle the key normally
                }
            }
        }
        ChordState::None => {}
    }

    match key.code {
        // Navigation
        KeyCode::Char('j') | KeyCode::Down => app.navigate_down(),
        KeyCode::Char('k') | KeyCode::Up => app.navigate_up(),
        KeyCode::Char('g') => app.jump_to_first(),
        KeyCode::Char('G') => app.jump_to_last(),

        // Numbers 1-9 start count prefix (for 5j, 10k style navigation)
        KeyCode::Char(c @ '1'..='9') => {
            let digit = (c as u32) - ('0' as u32);
            app.chord_state = ChordState::CountPending {
                count: digit,
                started_at: Instant::now(),
            };
        }

        // Space enters leader mode (which-key menu)
        KeyCode::Char(' ') => {
            app.enter_leader_mode();
        }

        // Enter is context-aware: toggle group on header, open conversation otherwise
        KeyCode::Enter => {
            handle_sidebar_enter(app)?;
        }

        // 'a' adds new conversation in selected group
        KeyCode::Char('a') => {
            app.new_conversation_in_selected_group()?;
        }

        // Tab toggles hide inactive
        KeyCode::Tab => {
            app.sidebar_state.toggle_hide_inactive();
            // Clamp selection to stay within bounds after items change
            let items = app.sidebar_items();
            let max = items.len().saturating_sub(1);
            let selected = app.sidebar_state.list_state.selected().unwrap_or(0);
            if selected > max {
                app.sidebar_state.list_state.select(Some(max));
            }
            app.update_selected_conversation();
        }

        // Direct actions (kept for convenience, also available via leader)
        KeyCode::Char('r') => app.manual_refresh()?,
        KeyCode::Char('y') => app.copy_selected_path_to_clipboard(),
        KeyCode::Char('d') => {
            // Start delete chord sequence
            app.chord_state = ChordState::DeletePending {
                started_at: Instant::now(),
            };
        }

        KeyCode::Esc => {
            // Cancel any pending chord
            app.chord_state = ChordState::None;
            if app.sidebar_state.has_filter() {
                // Clear persistent filter first
                app.clear_sidebar_filter();
            } else {
                app.clear_preview();
            }
        }

        // Enter insert mode and focus terminal
        KeyCode::Char('l') => {
            app.enter_insert_mode();
        }

        // Quick access actions (also available via leader)
        KeyCode::Char('D') => app.toggle_dangerous_mode(),
        KeyCode::Char('n') => app.open_new_project_modal(),
        KeyCode::Char('/') => app.open_search_modal(),
        KeyCode::Char('A') => app.cycle_archive_filter(),
        KeyCode::Char('x') => {
            let _ = app.archive_selected_conversation();
        }
        KeyCode::Char('u') => {
            let _ = app.unarchive_selected_conversation();
        }

        // Worktree: create from selected group (w) or search all projects (W)
        KeyCode::Char('w') => app.open_worktree_modal(),
        KeyCode::Char('W') => app.open_worktree_search_modal(),

        // Inline sidebar filter
        KeyCode::Char('f') => app.activate_sidebar_filter(),

        // Preview: show session in terminal pane without leaving sidebar
        KeyCode::Char('p') => {
            let _ = app.preview_selected();
        }

        _ => {}
    }
    Ok(KeyAction::Continue)
}

/// Handle Enter key in sidebar - context-aware behavior
fn handle_sidebar_enter(app: &mut App) -> Result<()> {
    use crate::ui::sidebar::SidebarItem;

    let items = app.sidebar_items();
    let selected = app.sidebar_state.list_state.selected().unwrap_or(0);

    match items.get(selected) {
        Some(
            SidebarItem::GroupHeader { .. }
            | SidebarItem::OtherHeader { .. }
            | SidebarItem::ProjectHeader { .. },
        ) => {
            // Toggle expand/collapse on group/project headers
            app.toggle_current_group();
        }
        Some(SidebarItem::WorkspaceSectionHeader) => {
            // Non-interactive item — no-op
        }
        _ => {
            // Open conversation/ephemeral session, add workspace, or handle other items
            app.open_selected()?;
        }
    }
    Ok(())
}

pub(crate) fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
    match (key.code, key.modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) => vec![c as u8],
        (KeyCode::Char(c), KeyModifiers::SHIFT) => vec![c.to_ascii_uppercase() as u8],
        (KeyCode::Char(c), KeyModifiers::CONTROL) => {
            // Control characters: Ctrl+A = 0x01, Ctrl+B = 0x02, etc.
            let ctrl = (c.to_ascii_lowercase() as u8)
                .wrapping_sub(b'a')
                .wrapping_add(1);
            vec![ctrl]
        }
        (KeyCode::Enter, _) => vec![b'\r'],
        (KeyCode::Backspace, _) => vec![0x7f],
        (KeyCode::Tab, _) => vec![b'\t'],
        (KeyCode::BackTab, _) => vec![0x1b, b'[', b'Z'],
        (KeyCode::Esc, _) => vec![0x1b],
        (KeyCode::Up, _) => vec![0x1b, b'[', b'A'],
        (KeyCode::Down, _) => vec![0x1b, b'[', b'B'],
        (KeyCode::Right, _) => vec![0x1b, b'[', b'C'],
        (KeyCode::Left, _) => vec![0x1b, b'[', b'D'],
        (KeyCode::Home, _) => vec![0x1b, b'[', b'H'],
        (KeyCode::End, _) => vec![0x1b, b'[', b'F'],
        (KeyCode::PageUp, _) => vec![0x1b, b'[', b'5', b'~'],
        (KeyCode::PageDown, _) => vec![0x1b, b'[', b'6', b'~'],
        (KeyCode::Delete, _) => vec![0x1b, b'[', b'3', b'~'],
        _ => vec![],
    }
}
