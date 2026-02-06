use claudatui::app;
use claudatui::ui;

use std::ffi::CString;
use std::io;
use std::io::IsTerminal;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{
        poll, read, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal,
};

use app::{
    App, ChordState, EscapeSequenceState, Focus, ModalState, TerminalPosition, TextSelection,
};
use claudatui::input::which_key::{LeaderAction, LeaderKeyResult};
use claudatui::input::InputMode;
use ui::layout::create_layout_with_help_config;
use ui::modal::{NewProjectModal, SearchKeyResult, SearchModal, WorktreeModal};
use ui::sidebar::{FilterKeyResult, Sidebar};
use ui::terminal_pane::TerminalPane;
use ui::toast_widget::{ToastPosition, ToastWidget};
use ui::WhichKeyWidget;

/// Hot reload status for display
enum HotReloadStatus {
    None,
    Building,
    BuildFailed(String),
}

fn main() -> Result<()> {
    // Check if we're in a proper terminal
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("claudatui must be run in an interactive terminal");
    }

    // Setup terminal
    enable_raw_mode().context("Failed to enable raw mode - are you in a terminal?")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("Failed to setup terminal")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("Failed to create terminal")?;

    // Create app
    let mut app = App::new().context("Failed to initialize application")?;

    // Get initial terminal size
    let size = terminal.size().context("Failed to get terminal size")?;
    app.term_size = (size.width, size.height);

    // Run app
    let result = run_app(&mut terminal, &mut app);

    // Restore terminal (always try to restore even on error)
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();

    // If we got a hot reload request, exec the new binary
    match result {
        Ok(HotReloadAction::Exec(path)) => {
            // Re-exec the new binary
            let c_path = CString::new(path.as_bytes()).expect("Invalid path");
            let args: [CString; 1] = [c_path.clone()];
            // execv never returns on success
            #[allow(unreachable_code)]
            match nix::unistd::execv(&c_path, &args) {
                Ok(infallible) => match infallible {},
                Err(e) => anyhow::bail!("Failed to exec new binary: {}", e),
            }
        }
        Ok(HotReloadAction::Quit) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Action to take after run_app completes
enum HotReloadAction {
    Quit,
    Exec(String),
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<HotReloadAction> {
    let mut hot_reload_status = HotReloadStatus::None;

    loop {
        // Check if chord has timed out
        app.check_chord_timeout();

        // Check if leader mode has timed out
        app.check_leader_timeout();

        // Check if escape sequence has timed out and flush buffered key
        if let Some(expired_key) = app.check_escape_seq_timeout() {
            flush_buffered_key(app, expired_key)?;
        }

        // Process all PTY output (direct, no daemon)
        app.session_manager.process_all_output();

        // Update session state cache for rendering
        app.update_session_state();

        // Check all sessions for dead PTYs and clean up
        app.check_all_session_status();

        // Check for sessions-index.json changes and reload if needed
        app.check_sessions_updates();

        // Update toast manager (remove expired)
        app.toast_manager.update();

        // Draw UI
        terminal.draw(|f| draw_ui(f, app, &hot_reload_status))?;

        // Handle events with timeout for PTY updates
        if poll(Duration::from_millis(50))? {
            let event = read()?;

            match event {
                Event::Key(key) => {
                    match handle_key_event(app, key, &mut hot_reload_status)? {
                        KeyAction::Continue => {}
                        KeyAction::Quit => return Ok(HotReloadAction::Quit),
                        KeyAction::HotReload => {
                            // Trigger hot reload
                            hot_reload_status = HotReloadStatus::Building;

                            // Draw the "Building..." status immediately
                            terminal.draw(|f| draw_ui(f, app, &hot_reload_status))?;

                            // Run cargo build
                            match perform_hot_reload() {
                                Ok(binary_path) => return Ok(HotReloadAction::Exec(binary_path)),
                                Err(e) => {
                                    hot_reload_status = HotReloadStatus::BuildFailed(e.to_string());
                                }
                            }
                        }
                    }
                }
                Event::Mouse(mouse) => {
                    handle_mouse_event(app, mouse);
                }
                Event::Resize(w, h) => {
                    app.resize(w, h)?;
                }
                _ => {}
            }
        }

        if app.should_quit {
            return Ok(HotReloadAction::Quit);
        }
    }
}

/// Action returned from key handling
enum KeyAction {
    Continue,
    Quit,
    HotReload,
}

fn handle_key_event(
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
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::CONTROL) => return Ok(KeyAction::Quit),
        (KeyCode::Left, KeyModifiers::CONTROL) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
            app.exit_insert_mode();
            return Ok(KeyAction::Continue);
        }
        (KeyCode::Right, KeyModifiers::CONTROL) | (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
            app.enter_insert_mode();
            return Ok(KeyAction::Continue);
        }
        _ => {}
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
fn handle_leader_key(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
    // Escape or Space cancels leader mode
    if key.code == KeyCode::Esc || key.code == KeyCode::Char(' ') {
        app.exit_leader_mode();
        return Ok(KeyAction::Continue);
    }

    // Get the key character
    let c = match key.code {
        KeyCode::Char(c) => c,
        _ => {
            // Non-character keys cancel leader mode
            app.exit_leader_mode();
            return Ok(KeyAction::Continue);
        }
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
                    state.pending_escape = Some((c, std::time::Instant::now()));
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
fn execute_leader_action(app: &mut App, action: LeaderAction) -> Result<()> {
    match action {
        LeaderAction::BookmarkJump(slot) => {
            app.jump_to_bookmark(slot)?;
        }
        LeaderAction::BookmarkSet(slot) => {
            app.bookmark_current(slot)?;
        }
        LeaderAction::BookmarkDelete(slot) => {
            app.remove_bookmark(slot)?;
        }
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
            // Open selected group and start new conversation
            app.open_selected()?;
        }
        LeaderAction::CreateWorktree => {
            app.open_worktree_modal();
        }
    }
    Ok(())
}

/// Result of processing a key through the jk/kj escape sequence state machine
enum EscapeSeqResult {
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
fn try_escape_sequence(app: &mut App, key: KeyEvent) -> EscapeSeqResult {
    let is_trigger = matches!(key.code, KeyCode::Char('j') | KeyCode::Char('k'))
        && key.modifiers == KeyModifiers::NONE;

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
                    started_at: std::time::Instant::now(),
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
                if is_trigger {
                    // New trigger key: buffer it as new pending
                    app.escape_seq_state = EscapeSequenceState::Pending {
                        first_key: trigger_char.unwrap(),
                        first_key_event: key,
                        started_at: std::time::Instant::now(),
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
                        started_at: std::time::Instant::now(),
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

/// Handle terminal insert mode with jk/kj escape sequence detection
fn handle_terminal_insert_with_escape_seq(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
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
fn handle_modal_insert_with_escape_seq(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
    // Escape always closes the modal (keep this fast path)
    if key.code == KeyCode::Esc {
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
fn handle_filter_insert_with_escape_seq(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
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
fn forward_key_to_filter(app: &mut App, key: KeyEvent) {
    match app.sidebar_state.handle_filter_key(key) {
        FilterKeyResult::Continue => {}
        FilterKeyResult::QueryChanged => {
            // Reset selection to top and update
            app.sidebar_state.list_state.select(Some(0));
            app.update_selected_conversation();
        }
        FilterKeyResult::Deactivated => {
            // Enter pressed — keep filter text, exit insert mode
            app.deactivate_sidebar_filter();
        }
    }
}

/// Forward a key event to the currently open modal
fn forward_key_to_modal(app: &mut App, key: KeyEvent) -> Result<()> {
    match &mut app.modal_state {
        ModalState::None => {}
        ModalState::NewProject(ref mut state) => {
            if let Some(path) = state.handle_key(key) {
                app.confirm_new_project(path)?;
            }
        }
        ModalState::Search(ref mut state) => match state.handle_key(key) {
            SearchKeyResult::Continue => {}
            SearchKeyResult::QueryChanged => {
                app.perform_search();
            }
            SearchKeyResult::Selected(session_id) => {
                app.navigate_to_conversation(&session_id)?;
            }
        },
        ModalState::Worktree(ref mut state) => {
            if let Some(branch_name) = state.handle_key(key) {
                app.confirm_worktree(branch_name)?;
            }
        }
    }
    Ok(())
}

/// Flush a buffered escape sequence key to the appropriate target (filter, PTY, or modal)
fn flush_buffered_key(app: &mut App, key: KeyEvent) -> Result<()> {
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

fn handle_sidebar_key_normal(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
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
                    let digit = c.to_digit(10).unwrap();
                    // Cap at 9999 to prevent overflow
                    let new_count = (count * 10 + digit).min(9999);
                    app.chord_state = ChordState::CountPending {
                        count: new_count,
                        started_at: std::time::Instant::now(),
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
            let digit = c.to_digit(10).unwrap();
            app.chord_state = ChordState::CountPending {
                count: digit,
                started_at: std::time::Instant::now(),
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
        }

        // Direct actions (kept for convenience, also available via leader)
        KeyCode::Char('r') => app.manual_refresh()?,
        KeyCode::Char('y') => app.copy_selected_path_to_clipboard(),
        KeyCode::Char('d') => {
            // Start delete chord sequence
            app.chord_state = ChordState::DeletePending {
                started_at: std::time::Instant::now(),
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

        // Worktree: create a new git worktree in selected group
        KeyCode::Char('w') => app.open_worktree_modal(),

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
    use claudatui::ui::sidebar::SidebarItem;

    let items = app.sidebar_items();
    let selected = app.sidebar_state.list_state.selected().unwrap_or(0);

    match items.get(selected) {
        Some(SidebarItem::GroupHeader { .. }) => {
            // Toggle expand/collapse on group headers
            app.toggle_current_group();
        }
        Some(SidebarItem::BookmarkHeader) | Some(SidebarItem::BookmarkSeparator) => {
            // Non-interactive items — no-op
        }
        _ => {
            // Open conversation/ephemeral session, bookmark entry, or handle other items
            app.open_selected()?;
        }
    }
    Ok(())
}

/// Map absolute screen coordinates to terminal content coordinates.
/// Returns None if the position is outside the terminal inner area.
fn screen_to_terminal_pos(app: &App, col: u16, row: u16) -> Option<TerminalPosition> {
    let inner = app.terminal_inner_area?;
    if col < inner.x
        || col >= inner.x + inner.width
        || row < inner.y
        || row >= inner.y + inner.height
    {
        return None;
    }
    let t_col = (col - inner.x) as usize;
    let t_row = (row - inner.y) as usize;

    // Clamp to actual screen state bounds
    if let Some(ref state) = app.session_state_cache {
        let max_row = state.screen.rows.len().saturating_sub(1);
        let max_col = state
            .screen
            .rows
            .first()
            .map_or(0, |r| r.cells.len().saturating_sub(1));
        Some(TerminalPosition {
            row: t_row.min(max_row),
            col: t_col.min(max_col),
        })
    } else {
        Some(TerminalPosition {
            row: t_row,
            col: t_col,
        })
    }
}

fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    const SCROLL_LINES: usize = 3;

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Start a new selection if click is inside terminal pane
            // Do NOT change focus or input mode — mouse selection is overlay-only
            if let Some(pos) = screen_to_terminal_pos(app, mouse.column, mouse.row) {
                app.text_selection = Some(TextSelection {
                    anchor: pos,
                    cursor: pos,
                });
            } else {
                // Click outside terminal — clear selection
                app.clear_selection();
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if app.text_selection.is_some() {
                if let Some(inner) = app.terminal_inner_area {
                    // Compute new cursor position before mutating selection
                    let clamped_col = mouse.column.max(inner.x).min(inner.x + inner.width - 1);
                    let clamped_row = mouse.row.max(inner.y).min(inner.y + inner.height - 1);
                    let new_pos = screen_to_terminal_pos(app, clamped_col, clamped_row);

                    if let (Some(ref mut sel), Some(pos)) = (&mut app.text_selection, new_pos) {
                        sel.cursor = pos;
                    }

                    // Auto-scroll when dragging above or below terminal area
                    if mouse.row < inner.y {
                        if let Some(ref session_id) = app.active_session_id.clone() {
                            if let Some(session) = app.session_manager.get_session_mut(session_id) {
                                session.scroll_up(1);
                            }
                        }
                    } else if mouse.row >= inner.y + inner.height {
                        if let Some(ref session_id) = app.active_session_id.clone() {
                            if let Some(session) = app.session_manager.get_session_mut(session_id) {
                                session.scroll_down(1);
                            }
                        }
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // On release: if there's a real selection, copy to clipboard and clear immediately
            if let Some(ref sel) = app.text_selection {
                if !sel.is_empty() {
                    app.copy_selection_to_clipboard();
                }
            }
            app.clear_selection();
        }
        MouseEventKind::ScrollUp => {
            app.clear_selection();
            app.scroll_up(SCROLL_LINES);
        }
        MouseEventKind::ScrollDown => {
            app.clear_selection();
            app.scroll_down(SCROLL_LINES);
        }
        _ => {}
    }
}

fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
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

/// Perform a hot reload by building the project and returning the new binary path.
fn perform_hot_reload() -> Result<String> {
    // Get the current executable's directory to determine the project root
    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe
        .parent()
        .context("No parent directory for executable")?;

    // The project root should be a few levels up from target/release or target/debug
    let project_root = if exe_dir.ends_with("target/release") || exe_dir.ends_with("target/debug") {
        exe_dir.parent().and_then(|p| p.parent())
    } else {
        // Might be running from project root directly
        None
    };

    // Build the project
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("--release");

    if let Some(root) = project_root {
        cmd.current_dir(root);
    }

    let output = cmd.output().context("Failed to run cargo build")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Build failed:\n{}", stderr);
    }

    // Return the path to the new binary
    let new_binary = if let Some(root) = project_root {
        root.join("target/release/claudatui")
    } else {
        exe_dir.join("claudatui")
    };

    Ok(new_binary.to_string_lossy().to_string())
}

fn draw_ui(f: &mut Frame, app: &mut App, hot_reload_status: &HotReloadStatus) {
    let (sidebar_area, terminal_area, help_area) =
        create_layout_with_help_config(f.area(), &app.config.layout);

    // Collect running session IDs for sidebar display
    let running_sessions = app.running_session_ids();

    // Clone filter state to avoid overlapping borrows with render_stateful_widget
    let filter_query = app.sidebar_state.filter_query.clone();
    let filter_active = app.sidebar_state.filter_active;
    let filter_cursor_pos = app.sidebar_state.filter_cursor_pos;

    // Draw sidebar with running session indicators and ephemeral sessions
    let sidebar = Sidebar::new(
        &app.groups,
        app.focus == Focus::Sidebar,
        &running_sessions,
        &app.ephemeral_sessions,
        app.sidebar_state.hide_inactive,
        app.sidebar_state.archive_filter,
        &app.bookmark_manager,
        &filter_query,
        filter_active,
        filter_cursor_pos,
    );
    f.render_stateful_widget(sidebar, sidebar_area, &mut app.sidebar_state);

    // Cache terminal inner area for mouse coordinate mapping (area minus 1px border)
    let terminal_inner = Rect {
        x: terminal_area.x + 1,
        y: terminal_area.y + 1,
        width: terminal_area.width.saturating_sub(2),
        height: terminal_area.height.saturating_sub(2),
    };
    app.terminal_inner_area = Some(terminal_inner);

    // Draw terminal pane with session state from daemon
    let session_state = app.get_session_state();
    let is_preview = app.preview_session_id.is_some() && app.focus == Focus::Sidebar;
    let selection = app.text_selection.as_ref();
    let terminal_pane = TerminalPane::new(
        session_state,
        matches!(app.focus, Focus::Terminal(_)),
        is_preview,
        selection,
    );
    f.render_widget(terminal_pane, terminal_area);

    // Draw help bar or hot reload status
    match hot_reload_status {
        HotReloadStatus::None => draw_help_bar(f, help_area, app),
        HotReloadStatus::Building => {
            let msg = Paragraph::new(Line::from(vec![
                Span::styled(
                    " BUILDING ",
                    Style::default().fg(Color::Black).bg(Color::Yellow),
                ),
                Span::raw(" Running cargo build --release..."),
            ]))
            .style(Style::default().bg(Color::DarkGray));
            f.render_widget(msg, help_area);
        }
        HotReloadStatus::BuildFailed(err) => {
            let msg = Paragraph::new(Line::from(vec![
                Span::styled(
                    " BUILD FAILED ",
                    Style::default().fg(Color::White).bg(Color::Red),
                ),
                Span::raw(format!(" {} (press any key to dismiss)", err)),
            ]))
            .style(Style::default().bg(Color::DarkGray));
            f.render_widget(msg, help_area);
        }
    }

    // Draw toasts (overlay on top of everything except modals)
    let toasts: Vec<_> = app.toast_manager.visible_toasts();
    if !toasts.is_empty() {
        ToastWidget::new(&toasts)
            .position(ToastPosition::BottomRight)
            .render(f, f.area());
    }

    // Draw which-key popup when in Leader mode
    if let InputMode::Leader(ref state) = app.input_mode {
        let which_key_area = WhichKeyWidget::calculate_area(f.area());
        let which_key = WhichKeyWidget::new(&app.which_key_config, &state.path);
        f.render_widget(which_key, which_key_area);
    }

    // Draw modal last (highest z-index)
    draw_modal(f, app);
}

fn draw_modal(f: &mut Frame, app: &mut App) {
    match &mut app.modal_state {
        ModalState::None => {}
        ModalState::NewProject(ref mut state) => {
            let area = NewProjectModal::calculate_area(f.area());
            let modal = NewProjectModal::new(state);
            f.render_widget(modal, area);
        }
        ModalState::Search(ref mut state) => {
            let area = SearchModal::calculate_area(f.area());
            let modal = SearchModal::new(state);
            f.render_widget(modal, area);
        }
        ModalState::Worktree(ref state) => {
            let area = WorktreeModal::calculate_area(f.area());
            let modal = WorktreeModal::new(state);
            f.render_widget(modal, area);
        }
    }
}

fn draw_help_bar(f: &mut Frame, area: Rect, app: &App) {
    // Check for recent dangerous mode toggle (2 second temporary message)
    if let Some(entering_dangerous) = app.recent_dangerous_mode_toggle(2000) {
        if entering_dangerous {
            let msg = Paragraph::new(Line::from(vec![
                Span::styled(
                    " DANGEROUS MODE ENABLED ",
                    Style::default().fg(Color::Black).bg(Color::Red),
                ),
                Span::raw(" New sessions will skip permission prompts."),
            ]))
            .style(Style::default().bg(Color::DarkGray));
            f.render_widget(msg, area);
            return;
        } else {
            let msg = Paragraph::new(Line::from(vec![
                Span::styled(
                    " NORMAL MODE ",
                    Style::default().fg(Color::Black).bg(Color::Green),
                ),
                Span::raw(" Dangerous mode disabled."),
            ]))
            .style(Style::default().bg(Color::DarkGray));
            f.render_widget(msg, area);
            return;
        }
    }

    // Check for pending chord sequence (highest priority after temporary messages)
    if let Some(pending) = app.chord_state.pending_display() {
        let hint = match &app.chord_state {
            ChordState::DeletePending { .. } => {
                format!(" {} (press d again to delete, Esc to cancel)", pending)
            }
            ChordState::CountPending { .. } => {
                format!(" {} (j/k to move, Esc to cancel)", pending)
            }
            ChordState::None => String::new(),
        };
        let msg = Paragraph::new(Line::from(vec![
            Span::styled(
                " PENDING ",
                Style::default().fg(Color::Black).bg(Color::Yellow),
            ),
            Span::raw(hint),
        ]))
        .style(Style::default().bg(Color::DarkGray));
        f.render_widget(msg, area);
        return;
    }

    // Check for recent clipboard copy (visible for 2 seconds)
    if let Some(path) = app.recent_clipboard_copy(2000) {
        let display_path = if path.len() > 50 {
            format!("...{}", &path[path.len() - 47..])
        } else {
            path.to_string()
        };
        let msg = Paragraph::new(Line::from(vec![
            Span::styled(
                " COPIED ",
                Style::default().fg(Color::Black).bg(Color::Green),
            ),
            Span::raw(format!(" {}", display_path)),
        ]))
        .style(Style::default().bg(Color::DarkGray));
        f.render_widget(msg, area);
        return;
    }

    // Check for recent manual refresh to show feedback (visible for 2 seconds)
    // Auto-refreshes are silent to reduce notification noise
    if let Some((is_auto, _elapsed)) = app.recent_refresh(2000) {
        if !is_auto {
            let msg = Paragraph::new(Line::from(vec![
                Span::styled(
                    " REFRESHED ",
                    Style::default().fg(Color::Black).bg(Color::Green),
                ),
                Span::raw(" Sessions list updated"),
            ]))
            .style(Style::default().bg(Color::DarkGray));
            f.render_widget(msg, area);
            return;
        }
    }

    // Build mode indicator
    let mode_indicator = build_mode_indicator(app);
    let dangerous_indicator = build_dangerous_indicator(app);

    // Filter-specific help bar hints
    if app.sidebar_state.filter_active {
        let help = Paragraph::new(Line::from(vec![
            build_mode_indicator(app),
            Span::styled(" Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("cancel "),
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("keep filter "),
            Span::styled(" jk ", Style::default().fg(Color::Cyan)),
            Span::raw("cancel"),
        ]))
        .style(Style::default().bg(Color::DarkGray));
        f.render_widget(help, area);
        return;
    }

    let help_text = match app.focus {
        Focus::Sidebar => {
            let mut spans = vec![mode_indicator];
            if let Some(danger) = dangerous_indicator {
                spans.push(danger);
            }
            if app.sidebar_state.has_filter() {
                // Persistent filter active — show filter-relevant hints
                spans.extend(vec![
                    Span::styled(" f ", Style::default().fg(Color::Cyan)),
                    Span::raw("edit filter "),
                    Span::styled(" Esc ", Style::default().fg(Color::Cyan)),
                    Span::raw("clear filter "),
                    Span::styled(" j/k ", Style::default().fg(Color::Cyan)),
                    Span::raw("nav "),
                    Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
                    Span::raw("open "),
                    Span::styled(" C-q ", Style::default().fg(Color::Cyan)),
                    Span::raw("quit"),
                ]);
            } else {
                spans.extend(vec![
                    Span::styled(" j/k ", Style::default().fg(Color::Cyan)),
                    Span::raw("nav "),
                    Span::styled(" l ", Style::default().fg(Color::Cyan)),
                    Span::raw("terminal "),
                    Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
                    Span::raw("open "),
                    Span::styled(" SPC ", Style::default().fg(Color::Cyan)),
                    Span::raw("leader "),
                    Span::styled(" / ", Style::default().fg(Color::Cyan)),
                    Span::raw("search "),
                    Span::styled(" f ", Style::default().fg(Color::Cyan)),
                    Span::raw("filter "),
                    Span::styled(" dd ", Style::default().fg(Color::Cyan)),
                    Span::raw("close "),
                    Span::styled(" C-q ", Style::default().fg(Color::Cyan)),
                    Span::raw("quit"),
                ]);
            }
            spans
        }
        Focus::Terminal(_) => {
            // Terminal focus = Insert mode (Normal+Terminal is not reachable)
            let mut spans = vec![mode_indicator];
            if let Some(danger) = dangerous_indicator {
                spans.push(danger);
            }
            spans.extend(vec![
                Span::styled(" jk ", Style::default().fg(Color::Cyan)),
                Span::raw("sidebar "),
                Span::styled(" C-h ", Style::default().fg(Color::Cyan)),
                Span::raw("sidebar "),
                Span::styled(" C-q ", Style::default().fg(Color::Cyan)),
                Span::raw("quit"),
            ]);
            spans
        }
    };

    let help = Paragraph::new(Line::from(help_text)).style(Style::default().bg(Color::DarkGray));
    f.render_widget(help, area);
}

/// Build the mode indicator span for the help bar
fn build_mode_indicator(app: &App) -> Span<'static> {
    match &app.input_mode {
        InputMode::Normal => Span::styled(
            " -- NORMAL -- ",
            Style::default().fg(Color::Black).bg(Color::Blue),
        ),
        InputMode::Insert => Span::styled(
            " -- INSERT -- ",
            Style::default().fg(Color::Black).bg(Color::Green),
        ),
        InputMode::Leader(_) => Span::styled(
            " -- LEADER -- ",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ),
    }
}

/// Build the dangerous mode indicator span, if active
fn build_dangerous_indicator(app: &App) -> Option<Span<'static>> {
    if app.dangerous_mode {
        Some(Span::styled(
            " -- DANGEROUS -- ",
            Style::default().fg(Color::Black).bg(Color::Red),
        ))
    } else {
        None
    }
}
