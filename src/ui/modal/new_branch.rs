//! New Branch modal dialog for creating git worktrees with new feature branches.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::git::{validate_branch_name, RepoInfo};

/// State for the new branch modal dialog
pub struct NewBranchModalState {
    /// Text input for branch name
    pub branch_input: String,
    /// Cursor position in branch input
    pub cursor_pos: usize,
    /// Error message to display (e.g., invalid branch name)
    pub error_message: Option<String>,
    /// Information about the repository
    pub repo_info: RepoInfo,
    /// The worktree path we're branching from
    pub base_path: PathBuf,
}

impl NewBranchModalState {
    /// Create a new modal state for the given repository
    pub fn new(repo_info: RepoInfo, base_path: PathBuf) -> Self {
        Self {
            branch_input: String::new(),
            cursor_pos: 0,
            error_message: None,
            repo_info,
            base_path,
        }
    }

    /// Handle key input for the modal.
    /// Returns Some(branch_name) if the user confirms, None otherwise.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        match key.code {
            KeyCode::Enter => {
                // Validate and confirm branch name
                if let Err(reason) = validate_branch_name(&self.branch_input) {
                    self.error_message = Some(reason);
                    return None;
                }

                // Return the branch name for creation
                return Some(self.branch_input.clone());
            }
            KeyCode::Char(c) => {
                self.branch_input.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                // Real-time validation
                self.validate_input();
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.branch_input.remove(self.cursor_pos);
                    self.validate_input();
                }
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.branch_input.len() {
                    self.branch_input.remove(self.cursor_pos);
                    self.validate_input();
                }
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor_pos < self.branch_input.len() {
                    self.cursor_pos += 1;
                }
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
            }
            KeyCode::End => {
                self.cursor_pos = self.branch_input.len();
            }
            _ => {}
        }
        None
    }

    /// Validate the current input and update error_message
    fn validate_input(&mut self) {
        if self.branch_input.is_empty() {
            self.error_message = None;
        } else if let Err(reason) = validate_branch_name(&self.branch_input) {
            self.error_message = Some(reason);
        } else {
            self.error_message = None;
        }
    }

    /// Get the repository name for display
    pub fn repo_name(&self) -> String {
        self.repo_info
            .repo_root
            .file_name()
            .map(|n| n.to_string_lossy().replace(".git", ""))
            .unwrap_or_else(|| "repo".to_string())
    }
}

/// Widget for rendering the new branch modal
pub struct NewBranchModal<'a> {
    state: &'a mut NewBranchModalState,
}

impl<'a> NewBranchModal<'a> {
    pub fn new(state: &'a mut NewBranchModalState) -> Self {
        Self { state }
    }

    /// Calculate the modal area (centered, 50% width, 40% height)
    pub fn calculate_area(total: Rect) -> Rect {
        let width = (total.width * 50 / 100)
            .max(40)
            .min(total.width.saturating_sub(4));
        let height = (total.height * 40 / 100)
            .max(12)
            .min(total.height.saturating_sub(4));

        let x = (total.width.saturating_sub(width)) / 2;
        let y = (total.height.saturating_sub(height)) / 2;

        Rect::new(x, y, width, height)
    }
}

impl Widget for NewBranchModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Skip rendering if area is too small
        if area.width < 30 || area.height < 8 {
            return;
        }

        // Clear the area first
        Clear.render(area, buf);

        // Main modal block
        let block = Block::default()
            .title(" New Feature Branch ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        // Layout: repo info, base branch, input, error, help
        let chunks = Layout::vertical([
            Constraint::Length(1), // Repository name
            Constraint::Length(1), // Base branch
            Constraint::Length(1), // Spacer
            Constraint::Length(3), // Input field
            Constraint::Length(1), // Error message
            Constraint::Min(1),    // Spacer
            Constraint::Length(1), // Help bar
        ])
        .split(inner);

        // Repository name
        let repo_name = self.state.repo_name();
        let repo_line = Line::from(vec![
            Span::styled("Repository: ", Style::default().fg(Color::DarkGray)),
            Span::styled(repo_name, Style::default().fg(Color::Yellow)),
        ]);
        Paragraph::new(repo_line).render(chunks[0], buf);

        // Base branch
        let base_line = Line::from(vec![
            Span::styled("Base: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &self.state.repo_info.current_branch,
                Style::default().fg(Color::Green),
            ),
        ]);
        Paragraph::new(base_line).render(chunks[1], buf);

        // Input field with border
        let input_block = Block::default()
            .title(" Branch name ")
            .borders(Borders::ALL)
            .border_style(if self.state.error_message.is_some() {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Yellow)
            });
        let input_inner = input_block.inner(chunks[3]);
        input_block.render(chunks[3], buf);

        // Render the input text with cursor
        self.render_input_text(input_inner, buf);

        // Error message if any
        if let Some(ref error) = self.state.error_message {
            let error_line = Line::from(Span::styled(
                error.clone(),
                Style::default().fg(Color::Red),
            ));
            Paragraph::new(error_line).render(chunks[4], buf);
        }

        // Help bar
        let help_text = vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("create "),
            Span::styled(" Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("cancel"),
        ];
        let help = Paragraph::new(Line::from(help_text))
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        help.render(chunks[6], buf);
    }
}

impl NewBranchModal<'_> {
    fn render_input_text(&self, area: Rect, buf: &mut Buffer) {
        let display_text = &self.state.branch_input;
        let cursor_pos = self.state.cursor_pos;

        // Calculate visible portion if text is too long
        let available_width = area.width as usize;
        let (visible_text, cursor_offset) = if display_text.len() <= available_width {
            (display_text.as_str(), cursor_pos)
        } else {
            // Scroll to keep cursor visible
            let start = if cursor_pos >= available_width {
                cursor_pos - available_width + 1
            } else {
                0
            };
            let end = (start + available_width).min(display_text.len());
            (&display_text[start..end], cursor_pos - start)
        };

        // Build text with cursor highlight
        let mut spans = Vec::new();
        for (i, c) in visible_text.chars().enumerate() {
            if i == cursor_offset {
                spans.push(Span::styled(
                    c.to_string(),
                    Style::default()
                        .bg(Color::White)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(c.to_string()));
            }
        }
        // If cursor is at the end, show a block cursor
        if cursor_offset >= visible_text.len() {
            spans.push(Span::styled(
                " ",
                Style::default().bg(Color::White),
            ));
        }

        let input_line = Line::from(spans);
        Paragraph::new(input_line).render(area, buf);
    }
}
