//! Session manager for the daemon.
//!
//! Owns all PTY sessions and their terminal state.
//!
//! Note: This module is used by the daemon binary, not the TUI.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtyPair, PtySize};

use super::protocol::{
    screen_state_from_vt100, CreateSessionRequest, SessionId, SessionInfo, SessionState,
};

/// Number of scrollback lines to retain in terminal history per session.
pub const SCROLLBACK_LINES: usize = 10000;

/// A managed PTY session in the daemon.
pub struct ManagedSession {
    /// Unique session ID.
    pub session_id: SessionId,
    /// Working directory.
    pub working_dir: String,
    /// Claude conversation ID if resuming.
    pub claude_session_id: Option<String>,
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
        while let Some(data) = self.output_rx.try_recv().ok() {
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

    /// Get session info for listing.
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            session_id: self.session_id.clone(),
            working_dir: self.working_dir.clone(),
            claude_session_id: self.claude_session_id.clone(),
            is_alive: self.is_alive(),
            rows: self.rows,
            cols: self.cols,
        }
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

/// Manages all sessions in the daemon.
pub struct SessionManager {
    /// All managed sessions.
    sessions: HashMap<SessionId, ManagedSession>,
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Create a new session.
    pub fn create_session(&mut self, req: CreateSessionRequest) -> Result<SessionId> {
        // Generate a unique session ID for the daemon
        let session_id = uuid::Uuid::new_v4().to_string();

        let session = ManagedSession::spawn(
            session_id.clone(),
            Path::new(&req.working_dir),
            req.rows,
            req.cols,
            req.resume_session_id.as_deref(),
        )?;

        self.sessions.insert(session_id.clone(), session);
        Ok(session_id)
    }

    /// Close a session.
    pub fn close_session(&mut self, session_id: &str) -> bool {
        self.sessions.remove(session_id).is_some()
    }

    /// List all sessions.
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions.values().map(|s| s.info()).collect()
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: &str) -> Option<&ManagedSession> {
        self.sessions.get(session_id)
    }

    /// Get a mutable session by ID.
    pub fn get_session_mut(&mut self, session_id: &str) -> Option<&mut ManagedSession> {
        self.sessions.get_mut(session_id)
    }

    /// Process output for all sessions.
    pub fn process_all(&mut self) {
        for session in self.sessions.values_mut() {
            session.process_output();
        }
    }

    /// Clean up dead sessions.
    /// Returns the IDs of sessions that were removed.
    pub fn cleanup_dead_sessions(&mut self) -> Vec<SessionId> {
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
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
