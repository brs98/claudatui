//! Which-key popup widget for displaying available leader commands.
//!
//! This widget renders a popup at the bottom of the screen showing the
//! current leader key path and available commands at that level.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::input::which_key::{LeaderCommand, WhichKeyConfig};

/// Widget that renders the which-key popup
pub struct WhichKeyWidget<'a> {
    /// The which-key configuration containing the command tree
    config: &'a WhichKeyConfig,
    /// Current path in the leader key sequence
    path: &'a [char],
}

impl<'a> WhichKeyWidget<'a> {
    /// Create a new which-key widget
    pub fn new(config: &'a WhichKeyConfig, path: &'a [char]) -> Self {
        Self { config, path }
    }

    /// Calculate the area for the which-key popup
    /// Positioned at the bottom of the screen, above the help bar
    pub fn calculate_area(screen: Rect) -> Rect {
        let height = 4; // Title + 2 lines of commands + border
        let y = screen.height.saturating_sub(height + 1); // +1 for help bar

        Rect {
            x: 0,
            y,
            width: screen.width,
            height,
        }
    }

    /// Build the title showing the current path
    fn build_title(&self) -> String {
        let path_str = if self.path.is_empty() {
            "SPC".to_string()
        } else {
            format!(
                "SPC {}",
                self.path
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };

        let submenu_title = self.config.submenu_title(self.path);
        format!("{} | {}", path_str, submenu_title)
    }

    /// Build command display lines
    fn build_command_lines(&self) -> Vec<Line<'a>> {
        let commands = match self.config.commands_at_path(self.path) {
            Some(cmds) => cmds,
            None => return vec![Line::from("No commands available")],
        };

        // Group commands into rows (4-5 commands per row for readability)
        let commands_per_row = 5;
        let mut lines = Vec::new();

        for chunk in commands.chunks(commands_per_row) {
            let spans: Vec<Span> = chunk
                .iter()
                .flat_map(|cmd| {
                    vec![
                        Span::styled(
                            format!(" {} ", cmd.key),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{} ", Self::format_label(cmd)),
                            Style::default().fg(Color::White),
                        ),
                    ]
                })
                .collect();

            lines.push(Line::from(spans));
        }

        lines
    }

    /// Format a command label, adding indicator for submenus
    fn format_label(cmd: &LeaderCommand) -> String {
        if cmd.is_submenu() {
            format!("{}+", cmd.label)
        } else {
            cmd.label.clone()
        }
    }
}

impl Widget for WhichKeyWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first (for overlay effect)
        Clear.render(area, buf);

        // Build content
        let title = self.build_title();
        let command_lines = self.build_command_lines();

        // Create block with title
        let block = Block::default()
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                format!(" {} ", title),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Black));

        // Create paragraph with commands
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
    fn build_title_shows_leader_label_for_empty_path() {
        let config = WhichKeyConfig::new();
        let widget = WhichKeyWidget::new(&config, &[]);
        assert_eq!(widget.build_title(), "SPC | Leader");
    }

    #[test]
    fn build_title_shows_submenu_label_for_bookmark_path() {
        let config = WhichKeyConfig::new();
        let widget = WhichKeyWidget::new(&config, &['b']);
        assert_eq!(widget.build_title(), "SPC b | Bookmarks");
    }

    #[test]
    fn calculate_area_positions_widget_near_bottom_of_screen() {
        let screen = Rect::new(0, 0, 100, 30);
        let area = WhichKeyWidget::calculate_area(screen);
        assert_eq!(area.height, 4);
        assert_eq!(area.y, 25); // 30 - 4 - 1
        assert_eq!(area.width, 100);
    }
}
