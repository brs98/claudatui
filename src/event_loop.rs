use std::io;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{poll, read, Event};
use ratatui::{
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal,
};

use crate::app::{App, ChordState, Focus};
use crate::handlers::keyboard::{flush_buffered_key, handle_key_event};
use crate::handlers::mouse::handle_mouse_event;
use crate::input::InputMode;
use crate::ui::layout::create_layout_with_help_config;
use crate::ui::modal::{
    NewProjectModal, SearchModal, WorkspaceModal, WorktreeModal, WorktreeSearchModal,
};
use crate::ui::sidebar::{Sidebar, SidebarContext};
use crate::ui::terminal_pane::TerminalPane;
use crate::ui::toast_widget::{ToastPosition, ToastWidget};
use crate::ui::WhichKeyWidget;

/// Action to take after run_app completes
pub enum HotReloadAction {
    Quit,
    Exec(String),
}

/// Hot reload status for display
pub(crate) enum HotReloadStatus {
    None,
    Building,
    BuildFailed(String),
}

/// Action returned from key handling
pub(crate) enum KeyAction {
    Continue,
    Quit,
    HotReload,
}

pub fn run_app(
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

        // Poll JSONL status for running sessions (~1s throttle)
        app.poll_running_session_statuses();

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

fn draw_ui(f: &mut Frame, app: &mut App, hot_reload_status: &HotReloadStatus) {
    let (sidebar_area, terminal_area, help_area) =
        create_layout_with_help_config(f.area(), &app.config.layout);

    // Collect running session IDs for sidebar display
    let running_sessions = app.running_session_ids();

    // Clone filter state to avoid overlapping borrows with render_stateful_widget
    let filter_query = app.sidebar_state.filter_query.clone();

    // Build sidebar context with shared parameters
    let sidebar_ctx = SidebarContext {
        groups: &app.groups,
        running_sessions: &running_sessions,
        ephemeral_sessions: &app.ephemeral_sessions,
        hide_inactive: app.sidebar_state.hide_inactive,
        archive_filter: app.sidebar_state.archive_filter,
        filter_query: &filter_query,
        filter_active: app.sidebar_state.filter_active,
        filter_cursor_pos: app.sidebar_state.filter_cursor_pos,
        workspaces: &app.config.workspaces,
    };

    // Draw sidebar with running session indicators and ephemeral sessions
    let sidebar = Sidebar::new(&sidebar_ctx, app.focus == Focus::Sidebar);
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
        crate::app::ModalState::None => {}
        crate::app::ModalState::NewProject(ref mut state) => {
            let area = NewProjectModal::calculate_area(f.area());
            let modal = NewProjectModal::new(state);
            f.render_widget(modal, area);
        }
        crate::app::ModalState::Search(ref mut state) => {
            let area = SearchModal::calculate_area(f.area());
            let modal = SearchModal::new(state);
            f.render_widget(modal, area);
        }
        crate::app::ModalState::Worktree(ref state) => {
            let area = WorktreeModal::calculate_area(f.area());
            let modal = WorktreeModal::new(state);
            f.render_widget(modal, area);
        }
        crate::app::ModalState::WorktreeSearch(ref mut state) => {
            let area = WorktreeSearchModal::calculate_area(f.area());
            let modal = WorktreeSearchModal::new(state);
            f.render_widget(modal, area);
        }
        crate::app::ModalState::Workspace(ref mut state) => {
            let area = WorkspaceModal::calculate_area(f.area());
            let modal = WorkspaceModal::new(state);
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

/// Perform a hot reload by building the project and returning the new binary path.
pub(crate) fn perform_hot_reload() -> Result<String> {
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

    Ok(new_binary.to_string_lossy().into_owned())
}
