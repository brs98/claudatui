//! Independent type definitions used by App.

use std::path::PathBuf;
use std::time::Instant;

/// Clipboard status for feedback display
#[derive(Debug, Clone)]
pub enum ClipboardStatus {
    /// No recent clipboard action
    None,
    /// A path was copied at the given instant
    Copied {
        /// The path that was copied
        path: String,
        /// When the copy happened
        at: Instant,
    },
}

/// Archive status for feedback display
#[derive(Debug, Clone)]
pub enum ArchiveStatus {
    /// No recent archive action
    None,
    /// A session was archived
    Archived {
        /// The session that was archived
        session_id: String,
        /// When the archive happened
        at: Instant,
    },
    /// A session was unarchived
    Unarchived {
        /// The session that was unarchived
        session_id: String,
        /// When the unarchive happened
        at: Instant,
    },
}

/// A position within the terminal content grid (row/col in screen coordinates)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalPosition {
    /// Row index (0-based from top of terminal content)
    pub row: usize,
    /// Column index (0-based from left)
    pub col: usize,
}

/// Text selection in the terminal pane (anchor = mouse down, cursor = current drag position)
#[derive(Debug, Clone)]
pub struct TextSelection {
    /// Position where the mouse was pressed down
    pub anchor: TerminalPosition,
    /// Current drag position
    pub cursor: TerminalPosition,
}

impl TextSelection {
    /// Return (start, end) sorted top-left to bottom-right
    pub fn ordered(&self) -> (TerminalPosition, TerminalPosition) {
        if self.anchor.row < self.cursor.row
            || (self.anchor.row == self.cursor.row && self.anchor.col <= self.cursor.col)
        {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    /// Check if a cell is within the selection (standard terminal stream selection)
    pub fn contains(&self, row: usize, col: usize) -> bool {
        let (start, end) = self.ordered();
        if start.row == end.row {
            // Single line: col must be in [start.col, end.col]
            row == start.row && col >= start.col && col <= end.col
        } else if row == start.row {
            // First line: from start.col to end of line
            col >= start.col
        } else if row == end.row {
            // Last line: from start of line to end.col
            col <= end.col
        } else {
            // Middle lines: fully selected
            row > start.row && row < end.row
        }
    }

    /// True when anchor and cursor are the same position (no real selection)
    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }
}

/// Split mode configuration for dual-pane terminal layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplitMode {
    /// Single pane (default)
    #[default]
    None,
    /// Side-by-side (left/right)
    Horizontal,
    /// Stacked (top/bottom)
    Vertical,
}

impl SplitMode {
    /// Cycle to the next split mode (None -> Horizontal -> Vertical -> None)
    pub fn cycle(&self) -> Self {
        match self {
            SplitMode::None => SplitMode::Horizontal,
            SplitMode::Horizontal => SplitMode::Vertical,
            SplitMode::Vertical => SplitMode::None,
        }
    }
}

/// Identifies which terminal pane is active in split mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalPaneId {
    /// Left or top pane
    #[default]
    Primary,
    /// Right or bottom pane (only in split mode)
    Secondary,
}

impl TerminalPaneId {
    /// Convert to array index (0 for Primary, 1 for Secondary)
    pub fn index(&self) -> usize {
        match self {
            TerminalPaneId::Primary => 0,
            TerminalPaneId::Secondary => 1,
        }
    }

    /// Toggle to the other pane
    pub fn toggle(&self) -> Self {
        match self {
            TerminalPaneId::Primary => TerminalPaneId::Secondary,
            TerminalPaneId::Secondary => TerminalPaneId::Primary,
        }
    }
}

/// Configuration for each terminal pane
#[derive(Debug, Clone, Default)]
pub struct PaneConfig {
    /// Session ID currently displayed in this pane, if any
    pub session_id: Option<String>,
}

/// Tracks an ephemeral session (new conversation not yet persisted to disk)
#[derive(Clone, Debug)]
pub struct EphemeralSession {
    pub project_path: PathBuf,
    pub created_at: i64, // Unix timestamp when session was created
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_returns_anchor_first_when_anchor_is_before_cursor() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 1, col: 5 },
            cursor: TerminalPosition { row: 3, col: 10 },
        };
        let (start, end) = sel.ordered();
        assert_eq!(start, TerminalPosition { row: 1, col: 5 });
        assert_eq!(end, TerminalPosition { row: 3, col: 10 });
    }

    #[test]
    fn ordered_swaps_when_cursor_is_before_anchor() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 5, col: 10 },
            cursor: TerminalPosition { row: 2, col: 3 },
        };
        let (start, end) = sel.ordered();
        assert_eq!(start, TerminalPosition { row: 2, col: 3 });
        assert_eq!(end, TerminalPosition { row: 5, col: 10 });
    }

    #[test]
    fn ordered_swaps_when_same_row_and_cursor_col_before_anchor_col() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 3, col: 15 },
            cursor: TerminalPosition { row: 3, col: 2 },
        };
        let (start, end) = sel.ordered();
        assert_eq!(start, TerminalPosition { row: 3, col: 2 });
        assert_eq!(end, TerminalPosition { row: 3, col: 15 });
    }

    #[test]
    fn ordered_preserves_order_when_same_position() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 4, col: 7 },
            cursor: TerminalPosition { row: 4, col: 7 },
        };
        let (start, end) = sel.ordered();
        assert_eq!(start, TerminalPosition { row: 4, col: 7 });
        assert_eq!(end, TerminalPosition { row: 4, col: 7 });
    }

    #[test]
    fn contains_single_line_includes_cells_within_range() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 2, col: 5 },
            cursor: TerminalPosition { row: 2, col: 10 },
        };
        assert!(sel.contains(2, 5));
        assert!(sel.contains(2, 7));
        assert!(sel.contains(2, 10));
        assert!(!sel.contains(2, 4));
        assert!(!sel.contains(2, 11));
        assert!(!sel.contains(1, 7));
        assert!(!sel.contains(3, 7));
    }

    #[test]
    fn contains_multiline_first_row_from_start_col_to_end() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 1, col: 5 },
            cursor: TerminalPosition { row: 3, col: 10 },
        };
        // First row: col >= 5
        assert!(sel.contains(1, 5));
        assert!(sel.contains(1, 100));
        assert!(!sel.contains(1, 4));
    }

    #[test]
    fn contains_multiline_middle_rows_fully_selected() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 1, col: 5 },
            cursor: TerminalPosition { row: 4, col: 10 },
        };
        // Middle rows (2 and 3): fully selected
        assert!(sel.contains(2, 0));
        assert!(sel.contains(2, 999));
        assert!(sel.contains(3, 0));
        assert!(sel.contains(3, 999));
    }

    #[test]
    fn contains_multiline_last_row_from_start_to_end_col() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 1, col: 5 },
            cursor: TerminalPosition { row: 3, col: 10 },
        };
        // Last row: col <= 10
        assert!(sel.contains(3, 0));
        assert!(sel.contains(3, 10));
        assert!(!sel.contains(3, 11));
    }

    #[test]
    fn contains_excludes_rows_outside_selection() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 2, col: 5 },
            cursor: TerminalPosition { row: 4, col: 10 },
        };
        assert!(!sel.contains(0, 5));
        assert!(!sel.contains(1, 5));
        assert!(!sel.contains(5, 5));
    }

    #[test]
    fn is_empty_returns_true_when_anchor_equals_cursor() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 3, col: 7 },
            cursor: TerminalPosition { row: 3, col: 7 },
        };
        assert!(sel.is_empty());
    }

    #[test]
    fn is_empty_returns_false_when_positions_differ_by_col() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 3, col: 7 },
            cursor: TerminalPosition { row: 3, col: 8 },
        };
        assert!(!sel.is_empty());
    }

    #[test]
    fn is_empty_returns_false_when_positions_differ_by_row() {
        let sel = TextSelection {
            anchor: TerminalPosition { row: 3, col: 7 },
            cursor: TerminalPosition { row: 4, col: 7 },
        };
        assert!(!sel.is_empty());
    }
}
