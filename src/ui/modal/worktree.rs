//! Worktree creation modal dialog.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

/// State for the worktree creation modal dialog.
pub struct WorktreeModalState {
    /// Branch name input
    pub branch_input: String,
    /// Cursor position in input
    pub cursor_pos: usize,
    /// Error message to display
    pub error_message: Option<String>,
    /// Group key for the selected group
    pub group_key: String,
    /// Display name for the repo
    pub repo_display_name: String,
}

impl WorktreeModalState {
    pub fn new(group_key: String, repo_display_name: String) -> Self {
        Self {
            branch_input: String::new(),
            cursor_pos: 0,
            error_message: None,
            group_key,
            repo_display_name,
        }
    }

    /// Handle key input. Returns Some(branch_name) on Enter, None otherwise.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        match key.code {
            KeyCode::Enter => {
                let branch = self.branch_input.trim().to_string();
                if branch.is_empty() {
                    self.error_message = Some("Branch name cannot be empty".to_string());
                    return None;
                }
                // Basic branch name validation
                if branch.contains(' ') || branch.contains("..") || branch.starts_with('-') {
                    self.error_message =
                        Some("Invalid branch name (no spaces, '..', or leading '-')".to_string());
                    return None;
                }
                Some(branch)
            }
            KeyCode::Char(c) => {
                self.branch_input.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                self.error_message = None;
                None
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.branch_input.remove(self.cursor_pos);
                    self.error_message = None;
                }
                None
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.branch_input.len() {
                    self.branch_input.remove(self.cursor_pos);
                    self.error_message = None;
                }
                None
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
                None
            }
            KeyCode::Right => {
                if self.cursor_pos < self.branch_input.len() {
                    self.cursor_pos += 1;
                }
                None
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                None
            }
            KeyCode::End => {
                self.cursor_pos = self.branch_input.len();
                None
            }
            _ => None,
        }
    }
}

/// Widget for rendering the worktree creation modal.
pub struct WorktreeModal<'a> {
    state: &'a WorktreeModalState,
}

impl<'a> WorktreeModal<'a> {
    pub fn new(state: &'a WorktreeModalState) -> Self {
        Self { state }
    }

    /// Calculate the modal area (centered, ~40% width, ~8 lines).
    pub fn calculate_area(total: Rect) -> Rect {
        let width = (total.width * 40 / 100)
            .max(36)
            .min(total.width.saturating_sub(4));
        let height = 10u16.min(total.height.saturating_sub(4));

        let x = (total.width.saturating_sub(width)) / 2;
        let y = (total.height.saturating_sub(height)) / 2;

        Rect::new(x, y, width, height)
    }
}

impl Widget for WorktreeModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 20 || area.height < 6 {
            return;
        }

        Clear.render(area, buf);

        let title = format!(" New Worktree — {} ", self.state.repo_display_name);
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        let chunks = Layout::vertical([
            Constraint::Length(2), // Label
            Constraint::Length(3), // Input field
            Constraint::Length(1), // Error message
            Constraint::Min(0),    // Spacer
            Constraint::Length(1), // Help bar
        ])
        .split(inner);

        // Label
        let label = Paragraph::new("Branch name:")
            .style(Style::default().fg(Color::White));
        label.render(chunks[0], buf);

        // Input field with border
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let input_inner = input_block.inner(chunks[1]);
        input_block.render(chunks[1], buf);

        // Render input text with cursor
        let display_text = &self.state.branch_input;
        let cursor_pos = self.state.cursor_pos;
        let available_width = input_inner.width as usize;

        let (visible_text, cursor_offset) = if display_text.len() <= available_width {
            (display_text.as_str(), cursor_pos)
        } else {
            let start = if cursor_pos >= available_width {
                cursor_pos - available_width + 1
            } else {
                0
            };
            let end = (start + available_width).min(display_text.len());
            (&display_text[start..end], cursor_pos - start)
        };

        let mut spans = Vec::new();
        for (i, c) in visible_text.chars().enumerate() {
            if i == cursor_offset {
                spans.push(Span::styled(
                    c.to_string(),
                    Style::default().bg(Color::White).fg(Color::Black),
                ));
            } else {
                spans.push(Span::raw(c.to_string()));
            }
        }
        if cursor_offset >= visible_text.len() {
            spans.push(Span::styled(" ", Style::default().bg(Color::White)));
        }

        Paragraph::new(Line::from(spans)).render(input_inner, buf);

        // Error message
        if let Some(ref error) = self.state.error_message {
            let error_line =
                Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red)));
            Paragraph::new(error_line).render(chunks[2], buf);
        }

        // Help bar
        let help = Paragraph::new(Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("create "),
            Span::styled(" Esc ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("cancel"),
        ]))
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
        help.render(chunks[4], buf);
    }
}
