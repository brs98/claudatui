//! Session manager for PTY sessions.
//!
//! Owns all PTY sessions and their terminal state directly in the TUI process.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtyPair, PtySize};

use super::types::{screen_state_from_vt100, SessionId, SessionState};

/// Number of scrollback lines to retain in terminal history per session.
pub const SCROLLBACK_LINES: usize = 10000;

/// A managed PTY session.
pub struct ManagedSession {
    /// Unique session ID (internal to this process).
    pub session_id: SessionId,
    /// Working directory.
    #[allow(dead_code)]
    working_dir: String,
    /// Claude conversation ID if resuming.
    #[allow(dead_code)]
    claude_session_id: Option<String>,
    /// PTY pair.
    pair: PtyPair,
    /// Writer to send input to PTY.
    writer: Box<dyn Write + Send>,
    /// Receiver for PTY output.
    output_rx: Receiver<Vec<u8>>,
    /// Reader thread handle.
    _reader_thread: thread::JoinHandle<()>,
    /// Flag for whether PTY is still alive.
    alive: Arc<AtomicBool>,
    /// VT100 parser for terminal emulation.
    pub vt_parser: vt100::Parser,
    /// Terminal dimensions.
    pub rows: u16,
    pub cols: u16,
    /// Current scroll offset (0 = live/bottom).
    pub scroll_offset: usize,
    /// Whether scroll is locked.
    pub scroll_locked: bool,
}

impl ManagedSession {
    /// Create a new managed session by spawning a PTY.
    pub fn spawn(
        session_id: SessionId,
        working_dir: &Path,
        rows: u16,
        cols: u16,
        claude_session_id: Option<&str>,
    ) -> Result<Self> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        let mut cmd = CommandBuilder::new("claude");
        cmd.cwd(working_dir);

        // Add --resume flag if session_id provided
        if let Some(sid) = claude_session_id {
            cmd.arg("--resume");
            cmd.arg(sid);
        }

        // Set environment variables for better terminal experience
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let _child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn claude command")?;

        let writer = pair.master.take_writer()?;
        let mut reader = pair.master.try_clone_reader()?;

        // Spawn a thread to read PTY output
        let (output_tx, output_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = Arc::clone(&alive);

        let reader_thread = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        if output_tx.send(buf[..n].to_vec()).is_err() {
                            break; // Channel closed
                        }
                    }
                    Err(_) => break,
                }
            }
            // Mark as not alive when reader thread exits
            alive_clone.store(false, Ordering::SeqCst);
        });

        Ok(Self {
            session_id,
            working_dir: working_dir.to_string_lossy().to_string(),
            claude_session_id: claude_session_id.map(|s| s.to_string()),
            pair,
            writer,
            output_rx,
            _reader_thread: reader_thread,
            alive,
            vt_parser: vt100::Parser::new(rows, cols, SCROLLBACK_LINES),
            rows,
            cols,
            scroll_offset: 0,
            scroll_locked: false,
        })
    }

    /// Check if the PTY is still alive.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Process pending PTY output.
    /// Returns true if any output was processed.
    pub fn process_output(&mut self) -> bool {
        let mut had_output = false;
        while let Ok(data) = self.output_rx.try_recv() {
            self.vt_parser.process(&data);
            had_output = true;
        }

        // Auto-scroll to bottom when new output arrives, unless scroll is locked
        if had_output && !self.scroll_locked {
            self.scroll_offset = 0;
        }

        // ALWAYS re-apply scroll position after processing output.
        // vt100's process() resets scrollback internally (standard terminal auto-scroll behavior),
        // so we must ensure the vt100 parser's scrollback matches our tracked position
        // before any subsequent state reads.
        self.vt_parser.set_scrollback(self.scroll_offset);

        had_output
    }

    /// Write input to the PTY.
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Resize the terminal and PTY.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.pair
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to resize PTY")?;

        self.vt_parser = vt100::Parser::new(rows, cols, SCROLLBACK_LINES);
        self.rows = rows;
        self.cols = cols;
        Ok(())
    }

    /// Get full session state for rendering.
    pub fn state(&self) -> SessionState {
        SessionState {
            session_id: self.session_id.clone(),
            is_alive: self.is_alive(),
            rows: self.rows,
            cols: self.cols,
            screen: screen_state_from_vt100(&self.vt_parser, self.scroll_offset),
            scroll_offset: self.scroll_offset,
            scroll_locked: self.scroll_locked,
            scrollback_len: self.vt_parser.screen().scrollback(),
        }
    }

    /// Scroll up by the specified number of lines.
    pub fn scroll_up(&mut self, lines: usize) {
        let desired_offset = self.scroll_offset.saturating_add(lines);
        self.vt_parser.set_scrollback(desired_offset);
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
}

/// Manages all PTY sessions.
pub struct SessionManager {
    /// All managed sessions.
    sessions: HashMap<SessionId, ManagedSession>,
    /// Counter for generating session IDs.
    next_id: u64,
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            next_id: 0,
        }
    }

    /// Create a new session.
    ///
    /// Returns the session ID on success.
    pub fn create_session(
        &mut self,
        working_dir: &Path,
        claude_session_id: Option<&str>,
        rows: u16,
        cols: u16,
    ) -> Result<SessionId> {
        // Generate a unique session ID
        let session_id = format!("session-{}", self.next_id);
        self.next_id += 1;

        let session = ManagedSession::spawn(
            session_id.clone(),
            working_dir,
            rows,
            cols,
            claude_session_id,
        )?;

        self.sessions.insert(session_id.clone(), session);
        Ok(session_id)
    }

    /// Close a session.
    ///
    /// Returns true if the session existed and was closed.
    pub fn close_session(&mut self, session_id: &str) -> bool {
        self.sessions.remove(session_id).is_some()
    }

    /// Get a mutable session by ID.
    pub fn get_session_mut(&mut self, session_id: &str) -> Option<&mut ManagedSession> {
        self.sessions.get_mut(session_id)
    }

    /// Get the session state for rendering.
    pub fn get_session_state(&self, session_id: &str) -> Option<SessionState> {
        self.sessions.get(session_id).map(|s| s.state())
    }

    /// Process output for all sessions.
    pub fn process_all_output(&mut self) {
        for session in self.sessions.values_mut() {
            session.process_output();
        }
    }

    /// Clean up dead sessions.
    /// Returns the IDs of sessions that were removed.
    pub fn cleanup_dead(&mut self) -> Vec<SessionId> {
        let dead: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| !s.is_alive())
            .map(|(id, _)| id.clone())
            .collect();

        for id in &dead {
            self.sessions.remove(id);
        }

        dead
    }

    /// Get all session IDs (for iteration).
    pub fn session_ids(&self) -> Vec<SessionId> {
        self.sessions.keys().cloned().collect()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
