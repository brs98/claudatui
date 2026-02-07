use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::app::{App, TerminalPosition, TextSelection};

/// Map absolute screen coordinates to terminal content coordinates.
/// Returns None if the position is outside the terminal inner area.
pub(crate) fn screen_to_terminal_pos(app: &App, col: u16, row: u16) -> Option<TerminalPosition> {
    let inner = app.terminal_inner_area?;
    if col < inner.x
        || col >= inner.x + inner.width
        || row < inner.y
        || row >= inner.y + inner.height
    {
        return None;
    }
    let t_col = (col - inner.x) as usize;
    let t_row = (row - inner.y) as usize;

    // Clamp to actual screen state bounds
    if let Some(ref state) = app.session_state_cache {
        let max_row = state.screen.rows.len().saturating_sub(1);
        let max_col = state
            .screen
            .rows
            .first()
            .map_or(0, |r| r.cells.len().saturating_sub(1));
        Some(TerminalPosition {
            row: t_row.min(max_row),
            col: t_col.min(max_col),
        })
    } else {
        Some(TerminalPosition {
            row: t_row,
            col: t_col,
        })
    }
}

pub(crate) fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    const SCROLL_LINES: usize = 3;

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Start a new selection if click is inside terminal pane
            // Do NOT change focus or input mode — mouse selection is overlay-only
            if let Some(pos) = screen_to_terminal_pos(app, mouse.column, mouse.row) {
                app.text_selection = Some(TextSelection {
                    anchor: pos,
                    cursor: pos,
                });
            } else {
                // Click outside terminal — clear selection
                app.clear_selection();
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if app.text_selection.is_some() {
                if let Some(inner) = app.terminal_inner_area {
                    // Compute new cursor position before mutating selection
                    let clamped_col = mouse.column.max(inner.x).min(inner.x + inner.width - 1);
                    let clamped_row = mouse.row.max(inner.y).min(inner.y + inner.height - 1);
                    let new_pos = screen_to_terminal_pos(app, clamped_col, clamped_row);

                    if let (Some(ref mut sel), Some(pos)) = (&mut app.text_selection, new_pos) {
                        sel.cursor = pos;
                    }

                    // Auto-scroll when dragging above or below terminal area
                    if mouse.row < inner.y {
                        if let Some(ref session_id) = app.active_session_id.clone() {
                            if let Some(session) = app.session_manager.get_session_mut(session_id) {
                                session.scroll_up(1);
                            }
                        }
                    } else if mouse.row >= inner.y + inner.height {
                        if let Some(ref session_id) = app.active_session_id.clone() {
                            if let Some(session) = app.session_manager.get_session_mut(session_id) {
                                session.scroll_down(1);
                            }
                        }
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // On release: if there's a real selection, copy to clipboard and clear immediately
            if let Some(ref sel) = app.text_selection {
                if !sel.is_empty() {
                    app.copy_selection_to_clipboard();
                }
            }
            app.clear_selection();
        }
        MouseEventKind::ScrollUp => {
            app.clear_selection();
            app.scroll_up(SCROLL_LINES);
        }
        MouseEventKind::ScrollDown => {
            app.clear_selection();
            app.scroll_down(SCROLL_LINES);
        }
        _ => {}
    }
}
