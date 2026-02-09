//! Help menu overlay showing all normal-mode keybindings.
//!
//! Toggled by `?` in normal mode. Displays a which-key-style popup
//! with all available keybindings organized into rows.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

/// A single keybinding entry for display
struct HelpEntry {
    key: &'static str,
    label: &'static str,
}

/// Widget that renders the help menu overlay
#[derive(Default)]
pub struct HelpMenuWidget;

impl HelpMenuWidget {
    /// Create a new help menu widget
    pub fn new() -> Self {
        Self
    }

    /// Calculate the area for the help menu popup.
    /// Positioned at the bottom of the screen, above the help bar.
    pub fn calculate_area(screen: Rect) -> Rect {
        let entries = Self::entries();
        let commands_per_row = 5;
        let row_count = entries.len().div_ceil(commands_per_row);
        let height = row_count as u16 + 2; // +2 for top border + padding
        let y = screen.height.saturating_sub(height + 1); // +1 for help bar

        Rect {
            x: 0,
            y,
            width: screen.width,
            height,
        }
    }

    /// All normal-mode keybinding entries
    fn entries() -> Vec<HelpEntry> {
        vec![
            HelpEntry { key: "j/k", label: "nav" },
            HelpEntry { key: "g", label: "first" },
            HelpEntry { key: "G", label: "last" },
            HelpEntry { key: "1-9", label: "count" },
            HelpEntry { key: "Enter", label: "open" },
            HelpEntry { key: "l", label: "terminal" },
            HelpEntry { key: "dd", label: "close" },
            HelpEntry { key: "a", label: "add" },
            HelpEntry { key: "p", label: "preview" },
            HelpEntry { key: "Tab", label: "inactive" },
            HelpEntry { key: "/", label: "search" },
            HelpEntry { key: "n", label: "project" },
            HelpEntry { key: "f", label: "filter" },
            HelpEntry { key: "r", label: "refresh" },
            HelpEntry { key: "y", label: "yank" },
            HelpEntry { key: "x", label: "archive" },
            HelpEntry { key: "u", label: "unarchive" },
            HelpEntry { key: "A", label: "cycle archive" },
            HelpEntry { key: "D", label: "dangerous" },
            HelpEntry { key: "w", label: "worktree" },
            HelpEntry { key: "W", label: "wt search" },
            HelpEntry { key: "q", label: "quit" },
            HelpEntry { key: "C-q", label: "quit" },
            HelpEntry { key: "Alt+.", label: "next proj" },
            HelpEntry { key: "Alt+,", label: "prev proj" },
        ]
    }

    /// Build command display lines grouped into rows
    fn build_command_lines() -> Vec<Line<'static>> {
        let entries = Self::entries();
        let commands_per_row = 5;
        let mut lines = Vec::new();

        for chunk in entries.chunks(commands_per_row) {
            let spans: Vec<Span> = chunk
                .iter()
                .flat_map(|entry| {
                    vec![
                        Span::styled(
                            format!(" {} ", entry.key),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{} ", entry.label),
                            Style::default().fg(Color::White),
                        ),
                    ]
                })
                .collect();

            lines.push(Line::from(spans));
        }

        lines
    }
}

impl Widget for HelpMenuWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first (overlay effect)
        Clear.render(area, buf);

        // Build content
        let command_lines = Self::build_command_lines();

        // Create block with title (same style as which-key)
        let block = Block::default()
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " ? | Keybindings ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Black));

        let paragraph = Paragraph::new(command_lines)
            .block(block)
            .style(Style::default().bg(Color::Black));

        paragraph.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculate_area_positions_above_help_bar() {
        let screen = Rect::new(0, 0, 100, 30);
        let area = HelpMenuWidget::calculate_area(screen);
        // 25 entries / 5 per row = 5 rows + 2 border = 7 height
        assert_eq!(area.height, 7);
        assert_eq!(area.y, 22); // 30 - 7 - 1
        assert_eq!(area.width, 100);
    }

    #[test]
    fn entries_are_not_empty() {
        let entries = HelpMenuWidget::entries();
        assert!(!entries.is_empty());
    }
}
