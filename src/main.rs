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
        poll, read, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
        EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
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

use app::{App, ChordState, Focus, ModalState};
use ui::layout::create_layout_with_help;
use ui::modal::NewProjectModal;
use ui::sidebar::Sidebar;
use ui::terminal_pane::TerminalPane;

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
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )
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
        DisableMouseCapture,
        DisableBracketedPaste
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

        // Process all PTY output (direct, no daemon)
        app.session_manager.process_all_output();

        // Update session state cache for rendering
        app.update_session_state();

        // Check all sessions for dead PTYs and clean up
        app.check_all_session_status();

        // Check for sessions-index.json changes and reload if needed
        app.check_sessions_updates();

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
                Event::Paste(text) => {
                    handle_paste_event(app, &text)?;
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

    // Modal handling takes precedence over everything else
    if app.is_modal_open() {
        return handle_modal_key(app, key);
    }

    // Global keybindings
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::CONTROL) => return Ok(KeyAction::Quit),
        (KeyCode::Char('q'), KeyModifiers::NONE) if app.focus == Focus::Sidebar => {
            return Ok(KeyAction::Quit);
        }
        // Hot reload: Ctrl+Shift+B (B for Build)
        (KeyCode::Char('B'), KeyModifiers::CONTROL | KeyModifiers::SHIFT) => {
            return Ok(KeyAction::HotReload);
        }
        (KeyCode::Left, KeyModifiers::CONTROL) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
            app.set_focus(Focus::Sidebar);
            return Ok(KeyAction::Continue);
        }
        (KeyCode::Right, KeyModifiers::CONTROL) | (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
            app.set_focus(Focus::Terminal);
            return Ok(KeyAction::Continue);
        }
        // Cycle between active projects (works from any pane)
        (KeyCode::Char('.'), KeyModifiers::CONTROL) => {
            let _ = app.cycle_and_switch_to_active(true);
            return Ok(KeyAction::Continue);
        }
        (KeyCode::Char(','), KeyModifiers::CONTROL) => {
            let _ = app.cycle_and_switch_to_active(false);
            return Ok(KeyAction::Continue);
        }
        _ => {}
    }

    // Focus-specific keybindings
    match app.focus {
        Focus::Sidebar => handle_sidebar_key(app, key),
        Focus::Terminal => handle_terminal_key(app, key),
    }
}

fn handle_modal_key(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
    // Escape always closes the modal
    if key.code == KeyCode::Esc {
        app.close_modal();
        return Ok(KeyAction::Continue);
    }

    // Handle modal-specific input
    match &mut app.modal_state {
        ModalState::None => {
            // Shouldn't happen, but handle gracefully
            Ok(KeyAction::Continue)
        }
        ModalState::NewProject(ref mut state) => {
            // Handle key and check if a path was confirmed
            if let Some(path) = state.handle_key(key) {
                // Path was confirmed - start session
                app.confirm_new_project(path)?;
            }
            Ok(KeyAction::Continue)
        }
    }
}

fn handle_sidebar_key(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
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
        KeyCode::Char('j') | KeyCode::Down => app.navigate_down(),
        KeyCode::Char('k') | KeyCode::Up => app.navigate_up(),
        KeyCode::Char('g') | KeyCode::Char('0') => app.jump_to_first(),
        KeyCode::Char('G') => app.jump_to_last(),
        KeyCode::Char(' ') => app.toggle_current_group(),
        KeyCode::Char('r') => app.manual_refresh()?,
        KeyCode::Char('a') => app.sidebar_state.toggle_hide_inactive(),
        KeyCode::Char('d') => {
            // Start delete chord sequence
            app.chord_state = ChordState::DeletePending {
                started_at: std::time::Instant::now(),
            };
        }
        KeyCode::Char('y') => {
            // Yank (copy) selected project path to clipboard
            app.copy_selected_path_to_clipboard();
        }
        // Start count chord sequence (digits 1-9)
        KeyCode::Char(c @ '1'..='9') => {
            let digit = c.to_digit(10).unwrap();
            app.chord_state = ChordState::CountPending {
                count: digit,
                started_at: std::time::Instant::now(),
            };
        }
        KeyCode::Esc => {
            // Cancel any pending chord
            app.chord_state = ChordState::None;
        }
        KeyCode::Char(']') => {
            // Cycle forward to next active project and switch to active session
            let _ = app.cycle_and_switch_to_active(true);
        }
        KeyCode::Char('[') => {
            // Cycle backward to previous active project and switch to active session
            let _ = app.cycle_and_switch_to_active(false);
        }
        KeyCode::Char('D') => {
            // Toggle dangerous mode (skip permissions for new sessions)
            app.toggle_dangerous_mode();
        }
        KeyCode::Char('n') => {
            // Open new project modal
            app.open_new_project_modal();
        }
        KeyCode::Enter => {
            app.open_selected()?;
        }
        _ => {}
    }
    Ok(KeyAction::Continue)
}

fn handle_terminal_key(app: &mut App, key: KeyEvent) -> Result<KeyAction> {
    // Get page size from session state, or use default
    let page_size = app.get_page_size();
    let is_scroll_locked = app.is_scroll_locked();

    match (key.code, key.modifiers) {
        (KeyCode::PageUp, _) => {
            app.scroll_up(page_size);
            return Ok(KeyAction::Continue);
        }
        (KeyCode::PageDown, _) => {
            app.scroll_down(page_size);
            return Ok(KeyAction::Continue);
        }
        (KeyCode::Char('G'), KeyModifiers::SHIFT) if is_scroll_locked => {
            app.scroll_to_bottom();
            return Ok(KeyAction::Continue);
        }
        (KeyCode::Esc, _) if is_scroll_locked => {
            app.scroll_to_bottom();
            return Ok(KeyAction::Continue);
        }
        _ => {}
    }

    // Convert key event to bytes and send to PTY
    let bytes = key_to_bytes(key);
    if !bytes.is_empty() {
        app.write_to_pty(&bytes)?;
    }
    Ok(KeyAction::Continue)
}

fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    // Only handle mouse scroll when terminal is focused
    if app.focus != Focus::Terminal {
        return;
    }

    const SCROLL_LINES: usize = 3;

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            app.scroll_up(SCROLL_LINES);
        }
        MouseEventKind::ScrollDown => {
            app.scroll_down(SCROLL_LINES);
        }
        _ => {}
    }
}

fn handle_paste_event(app: &mut App, text: &str) -> Result<()> {
    // Only send paste to terminal when focused
    if app.focus == Focus::Terminal {
        // Send bracketed paste sequence to PTY so the child process knows it's a paste
        // This allows readline and other tools to handle multi-line pastes correctly
        app.write_to_pty(b"\x1b[200~")?;
        app.write_to_pty(text.as_bytes())?;
        app.write_to_pty(b"\x1b[201~")?;
    }
    Ok(())
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
    let (sidebar_area, terminal_area, help_area) = create_layout_with_help(f.area());

    // Collect running session IDs for sidebar display
    let running_sessions = app.running_session_ids();

    // Draw sidebar with running session indicators and ephemeral sessions
    let sidebar = Sidebar::new(
        &app.groups,
        app.focus == Focus::Sidebar,
        &running_sessions,
        &app.ephemeral_sessions,
        app.sidebar_state.hide_inactive,
    );
    f.render_stateful_widget(sidebar, sidebar_area, &mut app.sidebar_state);

    // Draw terminal pane with session state from daemon
    let session_state = app.get_session_state();
    let terminal_pane = TerminalPane::new(session_state, app.focus == Focus::Terminal);
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

    // Draw modal last (on top of everything) if one is open
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
    }
}

fn draw_help_bar(f: &mut Frame, area: Rect, app: &App) {
    // Check for dangerous mode (highest priority warning)
    if app.dangerous_mode {
        let msg = Paragraph::new(Line::from(vec![
            Span::styled(
                " ⚠ DANGEROUS MODE ",
                Style::default().fg(Color::Black).bg(Color::Red),
            ),
            Span::raw(" New sessions will skip permission prompts. Press "),
            Span::styled("D", Style::default().fg(Color::Cyan)),
            Span::raw(" to disable."),
        ]))
        .style(Style::default().bg(Color::DarkGray));
        f.render_widget(msg, area);
        return;
    }

    // Check for pending chord sequence first (highest priority)
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

    let help_text = match app.focus {
        Focus::Sidebar => {
            vec![
                Span::styled(" j/k ", Style::default().fg(Color::Cyan)),
                Span::raw("nav "),
                Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
                Span::raw("open "),
                Span::styled(" n ", Style::default().fg(Color::Cyan)),
                Span::raw("new "),
                Span::styled(" dd ", Style::default().fg(Color::Cyan)),
                Span::raw("close "),
                Span::styled(" y ", Style::default().fg(Color::Cyan)),
                Span::raw("yank "),
                Span::styled(" Space ", Style::default().fg(Color::Cyan)),
                Span::raw("toggle "),
                Span::styled(" a ", Style::default().fg(Color::Cyan)),
                Span::raw("active "),
                Span::styled(" C-q ", Style::default().fg(Color::Cyan)),
                Span::raw("quit"),
            ]
        }
        Focus::Terminal => {
            vec![
                Span::styled(" C-h ", Style::default().fg(Color::Cyan)),
                Span::raw("sidebar "),
                Span::styled(" C-./C-, ", Style::default().fg(Color::Cyan)),
                Span::raw("cycle "),
                Span::styled(" PgUp/Dn ", Style::default().fg(Color::Cyan)),
                Span::raw("scroll "),
                Span::styled(" C-q ", Style::default().fg(Color::Cyan)),
                Span::raw("quit"),
            ]
        }
    };

    let help = Paragraph::new(Line::from(help_text)).style(Style::default().bg(Color::DarkGray));
    f.render_widget(help, area);
}
