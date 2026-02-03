use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize, PtyPair};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

/// Handles PTY spawning and I/O for Claude Code
pub struct PtyHandler {
    pair: PtyPair,
    writer: Box<dyn Write + Send>,
    output_rx: Receiver<Vec<u8>>,
    _reader_thread: thread::JoinHandle<()>,
}

impl PtyHandler {
    /// Spawn a new PTY running Claude Code in the specified directory
    pub fn spawn(working_dir: &Path, rows: u16, cols: u16) -> Result<Self> {
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
        });

        Ok(Self {
            pair,
            writer,
            output_rx,
            _reader_thread: reader_thread,
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
}
