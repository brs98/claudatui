//! Mosaic view: renders all active PTY sessions in a dynamic grid.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::session::SessionState;
use crate::ui::terminal_pane::TerminalPane;

/// Compute grid sub-rects for the mosaic layout.
///
/// Rules:
/// - 0 sessions: returns empty vec
/// - 1 session: full area
/// - 2 sessions: 2 side-by-side
/// - 3 sessions: 2 on top, 1 full-width bottom
/// - 4 sessions: 2x2 grid
/// - 5+ sessions: 3 columns, rows as needed; last row stretches if partial
pub fn compute_mosaic_rects(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return vec![];
    }
    if count == 1 {
        return vec![area];
    }

    let cols = if count <= 4 { 2 } else { 3 };
    let rows = count.div_ceil(cols);

    let row_height = area.height / rows as u16;

    let mut rects = Vec::with_capacity(count);
    let mut idx = 0;

    for row in 0..rows {
        let items_in_row = if row == rows - 1 { count - idx } else { cols };

        let y = area.y + row as u16 * row_height;
        let h = if row == rows - 1 {
            // Last row gets remaining height
            area.height - row as u16 * row_height
        } else {
            row_height
        };

        let row_col_width = area.width / items_in_row as u16;

        for col in 0..items_in_row {
            let x = area.x + col as u16 * row_col_width;
            let w = if col == items_in_row - 1 {
                // Last column in row gets remaining width
                area.width - col as u16 * row_col_width
            } else {
                row_col_width
            };

            rects.push(Rect::new(x, y, w, h));
            idx += 1;
        }
    }

    rects
}

/// Returns the (row, col) grid position and column count for a given index.
pub fn grid_position(count: usize, index: usize) -> (usize, usize, usize) {
    if count == 0 {
        return (0, 0, 1);
    }
    let cols = if count <= 4 { 2 } else { 3 };
    let row = index / cols;
    let col = index % cols;
    (row, col, cols)
}

/// Mosaic view widget that renders all active PTY sessions in a grid.
pub struct MosaicView<'a> {
    /// (session_id, display_name, session_state)
    sessions: &'a [(String, String, SessionState)],
    /// Index of the currently selected pane
    selected: usize,
}

impl<'a> MosaicView<'a> {
    pub fn new(sessions: &'a [(String, String, SessionState)], selected: usize) -> Self {
        Self { sessions, selected }
    }
}

impl<'a> Widget for MosaicView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.sessions.is_empty() {
            // Centered placeholder
            let text = "No active sessions";
            let x = area.x + (area.width.saturating_sub(text.len() as u16)) / 2;
            let y = area.y + area.height / 2;
            if y < area.y + area.height && x < area.x + area.width {
                buf.set_string(x, y, text, Style::default().fg(Color::DarkGray));
            }
            return;
        }

        let rects = compute_mosaic_rects(area, self.sessions.len());

        for (i, ((_sid, name, state), rect)) in self.sessions.iter().zip(rects.iter()).enumerate() {
            let focused = i == self.selected;
            let pane =
                TerminalPane::new(Some(state), focused, false, None).with_title(name.clone());
            pane.render(*rect, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_sessions_returns_empty() {
        let area = Rect::new(0, 0, 100, 50);
        assert!(compute_mosaic_rects(area, 0).is_empty());
    }

    #[test]
    fn one_session_returns_full_area() {
        let area = Rect::new(0, 0, 100, 50);
        let rects = compute_mosaic_rects(area, 1);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0], area);
    }

    #[test]
    fn two_sessions_side_by_side() {
        let area = Rect::new(0, 0, 100, 50);
        let rects = compute_mosaic_rects(area, 2);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].width, 50);
        assert_eq!(rects[1].width, 50);
        assert_eq!(rects[0].height, 50);
    }

    #[test]
    fn three_sessions_two_top_one_bottom() {
        let area = Rect::new(0, 0, 100, 50);
        let rects = compute_mosaic_rects(area, 3);
        assert_eq!(rects.len(), 3);
        // Top row: 2 panes
        assert_eq!(rects[0].width, 50);
        assert_eq!(rects[1].width, 50);
        // Bottom row: 1 full-width pane
        assert_eq!(rects[2].width, 100);
    }

    #[test]
    fn four_sessions_two_by_two() {
        let area = Rect::new(0, 0, 100, 50);
        let rects = compute_mosaic_rects(area, 4);
        assert_eq!(rects.len(), 4);
        assert_eq!(rects[0].width, 50);
        assert_eq!(rects[1].width, 50);
        assert_eq!(rects[2].width, 50);
        assert_eq!(rects[3].width, 50);
    }

    #[test]
    fn five_sessions_three_columns() {
        let area = Rect::new(0, 0, 99, 50);
        let rects = compute_mosaic_rects(area, 5);
        assert_eq!(rects.len(), 5);
        // First row: 3 cols
        assert_eq!(rects[0].width, 33);
        assert_eq!(rects[1].width, 33);
        assert_eq!(rects[2].width, 33);
        // Second row: 2 items stretching
        assert!(rects[3].width >= 49);
        assert!(rects[4].width >= 49);
    }

    #[test]
    fn grid_position_returns_correct_coords() {
        assert_eq!(grid_position(4, 0), (0, 0, 2));
        assert_eq!(grid_position(4, 1), (0, 1, 2));
        assert_eq!(grid_position(4, 2), (1, 0, 2));
        assert_eq!(grid_position(4, 3), (1, 1, 2));
    }
}
