mod app;
mod claude;
mod event;
mod pty;
mod ui;

use std::io;
use std::io::IsTerminal;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{poll, read, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers},
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

use app::{App, Focus};
use ui::layout::create_layout_with_help;
use ui::sidebar::Sidebar;
use ui::terminal_pane::TerminalPane;

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

    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        // Process PTY output
        app.process_pty_output();

        // Draw UI
        terminal.draw(|f| draw_ui(f, app))?;

        // Handle events with timeout for PTY updates
        if poll(Duration::from_millis(50))? {
            let event = read()?;

            match event {
                Event::Key(key) => {
                    if handle_key_event(app, key)? {
                        break;
                    }
                }
                Event::Resize(w, h) => {
                    app.resize(w, h)?;
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn handle_key_event(app: &mut App, key: KeyEvent) -> Result<bool> {
    // Global keybindings
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(true),
        (KeyCode::Char('q'), KeyModifiers::NONE) if app.focus == Focus::Sidebar => {
            return Ok(true);
        }
        (KeyCode::Tab, KeyModifiers::NONE) => {
            app.toggle_focus();
            return Ok(false);
        }
        _ => {}
    }

    // Focus-specific keybindings
    match app.focus {
        Focus::Sidebar => handle_sidebar_key(app, key),
        Focus::Terminal => handle_terminal_key(app, key),
    }
}

fn handle_sidebar_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.navigate_down(),
        KeyCode::Char('k') | KeyCode::Up => app.navigate_up(),
        KeyCode::Char('g') => app.jump_to_first(),
        KeyCode::Char('G') => app.jump_to_last(),
        KeyCode::Char(' ') => app.toggle_current_group(),
        KeyCode::Enter => {
            app.open_selected()?;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_terminal_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    // Convert key event to bytes and send to PTY
    let bytes = key_to_bytes(key);
    if !bytes.is_empty() {
        app.write_to_pty(&bytes)?;
    }
    Ok(false)
}

fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
    match (key.code, key.modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) => vec![c as u8],
        (KeyCode::Char(c), KeyModifiers::SHIFT) => vec![c.to_ascii_uppercase() as u8],
        (KeyCode::Char(c), KeyModifiers::CONTROL) => {
            // Control characters: Ctrl+A = 0x01, Ctrl+B = 0x02, etc.
            let ctrl = (c.to_ascii_lowercase() as u8).wrapping_sub(b'a').wrapping_add(1);
            vec![ctrl]
        }
        (KeyCode::Enter, _) => vec![b'\r'],
        (KeyCode::Backspace, _) => vec![0x7f],
        (KeyCode::Tab, _) => vec![b'\t'],
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

fn draw_ui(f: &mut Frame, app: &mut App) {
    let (sidebar_area, terminal_area, help_area) = create_layout_with_help(f.area());

    // Draw sidebar
    let sidebar = Sidebar::new(&app.groups, app.focus == Focus::Sidebar);
    f.render_stateful_widget(sidebar, sidebar_area, &mut app.sidebar_state);

    // Draw terminal pane
    let parser_ref = if app.pty.is_some() {
        Some(&app.vt_parser)
    } else {
        None
    };
    let terminal_pane = TerminalPane::new(parser_ref, app.focus == Focus::Terminal);
    f.render_widget(terminal_pane, terminal_area);

    // Draw help bar
    draw_help_bar(f, help_area, app);
}

fn draw_help_bar(f: &mut Frame, area: Rect, app: &App) {
    let help_text = match app.focus {
        Focus::Sidebar => {
            vec![
                Span::styled(" j/k ", Style::default().fg(Color::Cyan)),
                Span::raw("navigate "),
                Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
                Span::raw("open "),
                Span::styled(" Space ", Style::default().fg(Color::Cyan)),
                Span::raw("toggle "),
                Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
                Span::raw("focus "),
                Span::styled(" q ", Style::default().fg(Color::Cyan)),
                Span::raw("quit"),
            ]
        }
        Focus::Terminal => {
            vec![
                Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
                Span::raw("sidebar "),
                Span::styled(" Ctrl+C ", Style::default().fg(Color::Cyan)),
                Span::raw("quit"),
            ]
        }
    };

    let help = Paragraph::new(Line::from(help_text))
        .style(Style::default().bg(Color::DarkGray));
    f.render_widget(help, area);
}
