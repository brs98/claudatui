use std::ffi::CString;
use std::io;
use std::io::IsTerminal;

use anyhow::{Context, Result};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use claudatui::app::App;
use claudatui::event_loop::{run_app, HotReloadAction};

fn main() -> Result<()> {
    // Handle --version / -V before any terminal setup
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("claudatui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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
            let c_path =
                CString::new(path.as_bytes()).context("Invalid path for hot reload binary")?;
            let args: [CString; 1] = [c_path.clone()];
            // execv never returns on success
            match nix::unistd::execv(&c_path, &args) {
                Ok(infallible) => match infallible {},
                Err(e) => anyhow::bail!("Failed to exec new binary: {}", e),
            }
        }
        Ok(HotReloadAction::Quit) => Ok(()),
        Err(e) => Err(e),
    }
}
