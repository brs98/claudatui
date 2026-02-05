use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};

use crate::app::TextSelection;
use crate::session::{CellAttrs, ScreenState, SessionState, TermColor};

/// Terminal pane widget for displaying PTY output from daemon.
pub struct TerminalPane<'a> {
    session_state: Option<&'a SessionState>,
    focused: bool,
    preview: bool,
    selection: Option<&'a TextSelection>,
}

impl<'a> TerminalPane<'a> {
    pub fn new(
        session_state: Option<&'a SessionState>,
        focused: bool,
        preview: bool,
        selection: Option<&'a TextSelection>,
    ) -> Self {
        Self {
            session_state,
            focused,
            preview,
            selection,
        }
    }
}

impl<'a> Widget for TerminalPane<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // Get scroll offset from session if available
        let scroll_offset = self.session_state.map(|s| s.scroll_offset).unwrap_or(0);

        // Show scroll/preview indicators in title
        let title = if self.preview {
            if scroll_offset > 0 {
                format!(" Claude Code [PREVIEW] [SCROLLED: -{}] ", scroll_offset)
            } else {
                " Claude Code [PREVIEW] ".to_string()
            }
        } else if scroll_offset > 0 {
            format!(" Claude Code [SCROLLED: -{}] ", scroll_offset)
        } else {
            " Claude Code ".to_string()
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner_area = block.inner(area);
        block.render(area, buf);

        match self.session_state {
            Some(state) => {
                render_screen_state(&state.screen, inner_area, buf, state.scroll_offset, self.selection);
            }
            None => {
                // Show placeholder when no PTY is active
                let placeholder = "Press Enter to start Claude Code in selected directory";
                let x =
                    inner_area.x + (inner_area.width.saturating_sub(placeholder.len() as u16)) / 2;
                let y = inner_area.y + inner_area.height / 2;
                if y < inner_area.y + inner_area.height && x < inner_area.x + inner_area.width {
                    buf.set_string(x, y, placeholder, Style::default().fg(Color::DarkGray));
                }
            }
        }
    }
}

fn render_screen_state(
    screen: &ScreenState,
    area: Rect,
    buf: &mut Buffer,
    scroll_offset: usize,
    selection: Option<&TextSelection>,
) {
    let has_selection = selection.is_some();

    for (row_idx, screen_row) in screen.rows.iter().enumerate() {
        if row_idx as u16 >= area.height {
            break;
        }
        let y = area.y + row_idx as u16;

        for (col_idx, cell) in screen_row.cells.iter().enumerate() {
            if col_idx as u16 >= area.width {
                break;
            }
            let x = area.x + col_idx as u16;

            let is_selected = selection.map_or(false, |sel| sel.contains(row_idx, col_idx));

            if !cell.contents.is_empty() {
                let mut style = convert_cell_style(&cell.fg, &cell.bg, &cell.attrs);
                if is_selected {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                buf.set_string(x, y, &cell.contents, style);
            } else if is_selected {
                // Empty selected cell: show visible highlight
                if let Some(buf_cell) = buf.cell_mut((x, y)) {
                    buf_cell.set_char(' ');
                    buf_cell.set_style(Style::default().bg(Color::White).fg(Color::Black));
                }
            }
        }
    }

    // Only render cursor when at live view (not scrolled) and no selection active
    if scroll_offset == 0 && screen.cursor_visible && !has_selection {
        let (cursor_row, cursor_col) = screen.cursor;
        let cursor_x = area.x + cursor_col;
        let cursor_y = area.y + cursor_row;

        if cursor_y < area.y + area.height && cursor_x < area.x + area.width {
            if let Some(cell) = buf.cell_mut((cursor_x, cursor_y)) {
                cell.set_style(Style::default().bg(Color::White).fg(Color::Black));
            }
        }
    }
}

fn convert_cell_style(fg: &TermColor, bg: &TermColor, attrs: &CellAttrs) -> Style {
    let mut style = Style::default();

    style = style.fg(fg.to_ratatui());
    style = style.bg(bg.to_ratatui());

    if attrs.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if attrs.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if attrs.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if attrs.inverse {
        style = style.add_modifier(Modifier::REVERSED);
    }

    style
}
