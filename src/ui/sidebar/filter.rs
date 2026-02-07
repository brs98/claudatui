//! Filter input handling for the sidebar.

use crossterm::event::{KeyCode, KeyEvent};

/// Result of processing a key in the filter input
pub enum FilterKeyResult {
    /// No visual change needed
    Continue,
    /// Query text changed -- re-filter and reset selection
    QueryChanged,
    /// Enter pressed -- exit insert mode, keep filter text visible
    Deactivated,
}

/// Filter-related methods on `SidebarState`.
impl super::SidebarState {
    /// Activate the inline filter input (enter insert mode on filter)
    pub fn activate_filter(&mut self) {
        self.filter_active = true;
        self.filter_cursor_pos = self.filter_query.len();
    }

    /// Deactivate the filter input (keep text visible but stop accepting keystrokes)
    pub fn deactivate_filter(&mut self) {
        self.filter_active = false;
    }

    /// Clear the filter entirely (text, cursor, active state)
    pub fn clear_filter(&mut self) {
        self.filter_query.clear();
        self.filter_cursor_pos = 0;
        self.filter_active = false;
    }

    /// Whether there is a non-empty filter query
    pub fn has_filter(&self) -> bool {
        !self.filter_query.is_empty()
    }

    /// Handle a key event while the filter input is active
    pub fn handle_filter_key(&mut self, key: KeyEvent) -> FilterKeyResult {
        match key.code {
            KeyCode::Char(c) => {
                self.filter_query.insert(self.filter_cursor_pos, c);
                self.filter_cursor_pos += 1;
                FilterKeyResult::QueryChanged
            }
            KeyCode::Backspace => {
                if self.filter_cursor_pos > 0 {
                    self.filter_cursor_pos -= 1;
                    self.filter_query.remove(self.filter_cursor_pos);
                    FilterKeyResult::QueryChanged
                } else {
                    FilterKeyResult::Continue
                }
            }
            KeyCode::Delete => {
                if self.filter_cursor_pos < self.filter_query.len() {
                    self.filter_query.remove(self.filter_cursor_pos);
                    FilterKeyResult::QueryChanged
                } else {
                    FilterKeyResult::Continue
                }
            }
            KeyCode::Left => {
                self.filter_cursor_pos = self.filter_cursor_pos.saturating_sub(1);
                FilterKeyResult::Continue
            }
            KeyCode::Right => {
                self.filter_cursor_pos =
                    (self.filter_cursor_pos + 1).min(self.filter_query.len());
                FilterKeyResult::Continue
            }
            KeyCode::Home => {
                self.filter_cursor_pos = 0;
                FilterKeyResult::Continue
            }
            KeyCode::End => {
                self.filter_cursor_pos = self.filter_query.len();
                FilterKeyResult::Continue
            }
            KeyCode::Enter => FilterKeyResult::Deactivated,
            _ => FilterKeyResult::Continue,
        }
    }
}
