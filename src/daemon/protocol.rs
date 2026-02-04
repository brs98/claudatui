//! IPC protocol for communication between claudatui TUI and daemon.
//!
//! The daemon owns PTY processes and terminal state, allowing the TUI
//! to restart without losing sessions.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Unique identifier for a session managed by the daemon.
pub type SessionId = String;

/// Request messages sent from TUI to daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Ping to check daemon is alive.
    Ping,

    /// Gracefully shutdown the daemon.
    Shutdown,

    /// Create a new session.
    CreateSession(CreateSessionRequest),

    /// Close a session.
    CloseSession { session_id: SessionId },

    /// List all sessions.
    ListSessions,

    /// Write input to a session's PTY.
    WriteToSession {
        session_id: SessionId,
        data: Vec<u8>,
    },

    /// Get the current state of a session.
    GetSessionState { session_id: SessionId },

    /// Resize a session's PTY and terminal.
    ResizeSession {
        session_id: SessionId,
        rows: u16,
        cols: u16,
    },
}

/// Request to create a new session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    /// Working directory for the session.
    pub working_dir: String,
    /// Optional session ID to resume an existing Claude conversation.
    /// If None, starts a new conversation.
    pub resume_session_id: Option<String>,
    /// Terminal dimensions.
    pub rows: u16,
    pub cols: u16,
}

/// Response messages sent from daemon to TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    /// Ping response.
    Pong,

    /// Shutdown acknowledged.
    ShuttingDown,

    /// Session created successfully.
    SessionCreated { session_id: SessionId },

    /// Session closed successfully.
    SessionClosed { session_id: SessionId },

    /// List of all sessions.
    SessionList { sessions: Vec<SessionInfo> },

    /// Data written to session.
    WriteAck { session_id: SessionId },

    /// Session state snapshot.
    SessionState(SessionState),

    /// Session resized.
    Resized { session_id: SessionId },

    /// Error occurred.
    Error { message: String },
}

/// Summary info about a session (for listing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique session ID.
    pub session_id: SessionId,
    /// Working directory.
    pub working_dir: String,
    /// Claude conversation ID if resuming, or None if new.
    pub claude_session_id: Option<String>,
    /// Whether the PTY is still alive.
    pub is_alive: bool,
    /// Terminal dimensions.
    pub rows: u16,
    pub cols: u16,
}

/// Full session state for rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Session ID.
    pub session_id: SessionId,
    /// Whether the PTY is alive.
    pub is_alive: bool,
    /// Terminal dimensions.
    pub rows: u16,
    pub cols: u16,
    /// Screen contents as rows of cells.
    pub screen: ScreenState,
    /// Current scroll offset (0 = live/bottom).
    pub scroll_offset: usize,
    /// Whether scroll is locked (user scrolled up).
    pub scroll_locked: bool,
    /// Total scrollback lines available.
    pub scrollback_len: usize,
}

/// Terminal screen state for rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenState {
    /// Rows of the screen, including scrollback.
    /// Each row is a list of cells.
    pub rows: Vec<ScreenRow>,
    /// Cursor position (row, col).
    pub cursor: (u16, u16),
    /// Whether cursor is visible.
    pub cursor_visible: bool,
}

/// A row of cells on the screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenRow {
    /// The cells in this row.
    pub cells: Vec<ScreenCell>,
}

/// A single cell on the terminal screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenCell {
    /// The character(s) in this cell.
    pub contents: String,
    /// Foreground color.
    pub fg: TermColor,
    /// Background color.
    pub bg: TermColor,
    /// Text attributes.
    pub attrs: CellAttrs,
}

/// Terminal color representation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct TermColor {
    /// Color type and value.
    pub kind: ColorKind,
}

/// Color kinds supported.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum ColorKind {
    /// Default terminal color.
    #[default]
    Default,
    /// 256-color palette index.
    Indexed(u8),
    /// 24-bit RGB color.
    Rgb(u8, u8, u8),
}

/// Cell text attributes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct CellAttrs {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl TermColor {
    /// Convert from vt100 color.
    pub fn from_vt100(color: vt100::Color) -> Self {
        let kind = match color {
            vt100::Color::Default => ColorKind::Default,
            vt100::Color::Idx(idx) => ColorKind::Indexed(idx),
            vt100::Color::Rgb(r, g, b) => ColorKind::Rgb(r, g, b),
        };
        Self { kind }
    }

    /// Convert to ratatui color for rendering.
    pub fn to_ratatui(&self) -> ratatui::style::Color {
        match self.kind {
            ColorKind::Default => ratatui::style::Color::Reset,
            ColorKind::Indexed(idx) => ratatui::style::Color::Indexed(idx),
            ColorKind::Rgb(r, g, b) => ratatui::style::Color::Rgb(r, g, b),
        }
    }
}

impl CellAttrs {
    /// Convert from vt100 cell.
    pub fn from_vt100_cell(cell: &vt100::Cell) -> Self {
        Self {
            bold: cell.bold(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        }
    }
}

/// Extract screen state from a vt100 parser.
pub fn screen_state_from_vt100(parser: &vt100::Parser, _scroll_offset: usize) -> ScreenState {
    let screen = parser.screen();
    let (rows, cols) = screen.size();

    let mut screen_rows = Vec::with_capacity(rows as usize);

    // Get the visible rows based on scroll offset
    for row_idx in 0..rows {
        let mut cells = Vec::with_capacity(cols as usize);
        for col_idx in 0..cols {
            let cell = screen.cell(row_idx, col_idx).unwrap();
            cells.push(ScreenCell {
                contents: cell.contents().to_string(),
                fg: TermColor::from_vt100(cell.fgcolor()),
                bg: TermColor::from_vt100(cell.bgcolor()),
                attrs: CellAttrs::from_vt100_cell(cell),
            });
        }
        screen_rows.push(ScreenRow { cells });
    }

    let cursor = screen.cursor_position();

    ScreenState {
        rows: screen_rows,
        cursor,
        cursor_visible: !screen.hide_cursor(),
    }
}

/// Length-prefixed message framing for the IPC protocol.
pub mod framing {
    use std::io::{Read, Write};

    use super::*;

    /// Write a message with length prefix.
    pub fn write_message<W: Write, T: Serialize>(writer: &mut W, msg: &T) -> std::io::Result<()> {
        let data = serde_json::to_vec(msg).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        let len = data.len() as u32;
        writer.write_all(&len.to_le_bytes())?;
        writer.write_all(&data)?;
        writer.flush()?;
        Ok(())
    }

    /// Read a length-prefixed message.
    pub fn read_message<R: Read, T: for<'de> Deserialize<'de>>(reader: &mut R) -> std::io::Result<T> {
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        // Sanity check - messages shouldn't be huge
        if len > 100 * 1024 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Message too large: {} bytes", len),
            ));
        }

        let mut data = vec![0u8; len];
        reader.read_exact(&mut data)?;

        serde_json::from_slice(&data).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })
    }
}
