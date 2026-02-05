//! PTY handling for spawning and managing Claude Code processes.
//!
//! Note: This module is primarily used by the daemon. The TUI uses DaemonClient instead.

#![allow(dead_code)]

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtyPair, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

/// Handles PTY spawning and I/O for Claude Code
pub struct PtyHandler {
    pair: PtyPair,
    writer: Box<dyn Write + Send>,
    output_rx: Receiver<Vec<u8>>,
    _reader_thread: thread::JoinHandle<()>,
    /// Flag set to false when the reader thread exits (child process terminated)
    alive: Arc<AtomicBool>,
}

impl PtyHandler {
    /// Spawn a new PTY running Claude Code in the specified directory
    ///
    /// If `session_id` is provided, runs `claude --resume <session_id>` to resume
    /// an existing conversation. Otherwise starts a new conversation.
    pub fn spawn(
        working_dir: &Path,
        rows: u16,
        cols: u16,
        session_id: Option<&str>,
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
        if let Some(sid) = session_id {
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
            pair,
            writer,
            output_rx,
            _reader_thread: reader_thread,
            alive,
        })
    }

    /// Resize the PTY
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.pair
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to resize PTY")?;
        Ok(())
    }

    /// Write data to the PTY (keyboard input)
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Try to receive output from the PTY (non-blocking)
    pub fn try_recv_output(&self) -> Option<Vec<u8>> {
        self.output_rx.try_recv().ok()
    }

    /// Check if the PTY child process is still running
    ///
    /// Returns false if the reader thread has exited (EOF on PTY),
    /// which indicates the child process has terminated.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}
