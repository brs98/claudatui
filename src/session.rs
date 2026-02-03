//! Running session management for parallel Claude Code sessions.
//!
//! Each `RunningSession` encapsulates all state needed to maintain
//! an active Claude Code PTY session, including terminal emulation.

use std::path::PathBuf;

use crate::pty::PtyHandler;

/// Number of scrollback lines to retain in terminal history per session
pub const SCROLLBACK_LINES: usize = 10000;

/// A running Claude Code session with its own PTY and terminal state.
///
/// This struct bundles together:
/// - The PTY handler for the subprocess
/// - The VT100 terminal parser for rendering
/// - Scroll state (offset and lock)
///
/// Each session maintains independent terminal state, allowing users
/// to switch between sessions without losing output or scroll position.
pub struct RunningSession {
    /// The session_id from Claude's session tracking
    #[allow(dead_code)]
    pub session_id: String,
    /// The working directory where this session was started
    #[allow(dead_code)]
    pub working_dir: PathBuf,
    /// PTY handler for subprocess I/O
    pub pty: PtyHandler,
    /// VT100 parser for terminal emulation
    pub vt_parser: vt100::Parser,
    /// Current scroll offset (0 = live/bottom, positive = lines scrolled up)
    pub scroll_offset: usize,
    /// Whether scroll is locked (user has scrolled up)
    pub scroll_locked: bool,
}

impl RunningSession {
    /// Create a new running session with the given PTY.
    ///
    /// Initializes the VT100 parser with the specified dimensions
    /// and resets scroll state to live view.
    pub fn new(session_id: String, working_dir: PathBuf, pty: PtyHandler, rows: u16, cols: u16) -> Self {
        Self {
            session_id,
            working_dir,
            pty,
            vt_parser: vt100::Parser::new(rows, cols, SCROLLBACK_LINES),
            scroll_offset: 0,
            scroll_locked: false,
        }
    }

    /// Process any pending PTY output into the terminal parser.
    ///
    /// Returns true if any output was processed.
    pub fn process_output(&mut self) -> bool {
        let mut had_output = false;
        while let Some(data) = self.pty.try_recv_output() {
            self.vt_parser.process(&data);
            had_output = true;
        }

        // Auto-scroll to bottom when new output arrives, unless scroll is locked
        if had_output && !self.scroll_locked {
            self.scroll_offset = 0;
            self.vt_parser.set_scrollback(0);
        }

        had_output
    }

    /// Scroll up by the specified number of lines.
    pub fn scroll_up(&mut self, lines: usize) {
        let desired_offset = self.scroll_offset.saturating_add(lines);
        self.vt_parser.set_scrollback(desired_offset);
        // Read back actual offset (clamped by parser)
        self.scroll_offset = self.vt_parser.screen().scrollback();
        if self.scroll_offset > 0 {
            self.scroll_locked = true;
        }
    }

    /// Scroll down by the specified number of lines.
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.vt_parser.set_scrollback(self.scroll_offset);
        if self.scroll_offset == 0 {
            self.scroll_locked = false;
        }
    }

    /// Jump to the bottom (live view).
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.scroll_locked = false;
        self.vt_parser.set_scrollback(0);
    }

    /// Get the current scrollback length.
    #[allow(dead_code)]
    pub fn scrollback_len(&self) -> usize {
        self.vt_parser.screen().scrollback()
    }

    /// Check if the PTY is still alive.
    pub fn is_alive(&self) -> bool {
        self.pty.is_alive()
    }

    /// Resize the terminal and PTY.
    pub fn resize(&mut self, rows: u16, cols: u16) -> anyhow::Result<()> {
        self.vt_parser = vt100::Parser::new(rows, cols, SCROLLBACK_LINES);
        self.pty.resize(rows, cols)?;
        Ok(())
    }
}
