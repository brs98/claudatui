//! Terminal state types for rendering PTY output.
//!
//! These types represent terminal screen state that can be rendered by the TUI.

use serde::{Deserialize, Serialize};

/// Unique identifier for a session.
pub type SessionId = String;

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
            let Some(cell) = screen.cell(row_idx, col_idx) else {
                continue;
            };
            cells.push(ScreenCell {
                contents: cell.contents().clone(),
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
