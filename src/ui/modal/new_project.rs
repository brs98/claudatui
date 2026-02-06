//! New Project modal dialog for starting conversations in arbitrary directories.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use ratatui_explorer::FileExplorer;

/// Which tab is active in the new project modal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NewProjectTab {
    #[default]
    Browse,
    EnterPath,
}

/// State for the new project modal dialog
pub struct NewProjectModalState {
    /// Currently active tab
    pub active_tab: NewProjectTab,
    /// File explorer for Browse tab
    pub file_explorer: FileExplorer,
    /// Text input for Enter Path tab
    pub path_input: String,
    /// Cursor position in path input
    pub cursor_pos: usize,
    /// Error message to display (e.g., invalid path)
    pub error_message: Option<String>,
}

impl Default for NewProjectModalState {
    fn default() -> Self {
        Self::new()
    }
}

impl NewProjectModalState {
    /// Create a new modal state starting at the home directory
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let file_explorer = FileExplorer::new()
            .unwrap_or_else(|_| FileExplorer::new().expect("Failed to create file explorer"));

        Self {
            active_tab: NewProjectTab::Browse,
            file_explorer,
            path_input: home.to_string_lossy().into_owned(),
            cursor_pos: home.to_string_lossy().len(),
            error_message: None,
        }
    }

    /// Switch to the next tab
    pub fn next_tab(&mut self) {
        self.active_tab = match self.active_tab {
            NewProjectTab::Browse => NewProjectTab::EnterPath,
            NewProjectTab::EnterPath => NewProjectTab::Browse,
        };
        self.error_message = None;
    }

    /// Handle key input for the modal
    /// Returns Some(path) if a path was confirmed, None otherwise
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<PathBuf> {
        // Tab switches between tabs
        if key.code == KeyCode::Tab {
            self.next_tab();
            return None;
        }

        match self.active_tab {
            NewProjectTab::Browse => self.handle_browse_key(key),
            NewProjectTab::EnterPath => self.handle_path_input_key(key),
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> Option<PathBuf> {
        match key.code {
            KeyCode::Enter => {
                // Confirm selection
                let current = self.file_explorer.current();
                let path = current.path().to_path_buf();

                // If it's a file, use its parent directory
                let dir_path = if path.is_file() {
                    path.parent().map(Path::to_path_buf).unwrap_or(path)
                } else {
                    path
                };

                if dir_path.is_dir() {
                    return Some(dir_path);
                } else {
                    self.error_message = Some("Not a valid directory".to_string());
                }
            }
            // Use vim-style navigation: h/j/k/l
            KeyCode::Char('h') | KeyCode::Left => {
                // Go to parent directory
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Left,
                        KeyModifiers::NONE,
                    )));
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Down,
                        KeyModifiers::NONE,
                    )));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Up,
                        KeyModifiers::NONE,
                    )));
            }
            KeyCode::Char('l') | KeyCode::Right => {
                // Enter directory
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Right,
                        KeyModifiers::NONE,
                    )));
            }
            KeyCode::Char('g') => {
                // Jump to first
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Home,
                        KeyModifiers::NONE,
                    )));
            }
            KeyCode::Char('G') => {
                // Jump to last
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::End,
                        KeyModifiers::NONE,
                    )));
            }
            _ => {}
        }
        None
    }

    fn handle_path_input_key(&mut self, key: KeyEvent) -> Option<PathBuf> {
        match key.code {
            KeyCode::Enter => {
                // Validate and confirm path
                let expanded = expand_tilde(&self.path_input);
                let path = PathBuf::from(&expanded);

                if path.is_dir() {
                    return Some(path);
                } else if path.is_file() {
                    // Use parent directory if file was specified
                    if let Some(parent) = path.parent() {
                        return Some(parent.to_path_buf());
                    }
                }

                // Check if path exists at all
                if !path.exists() {
                    self.error_message = Some(format!("Path does not exist: {}", expanded));
                } else {
                    self.error_message = Some("Not a valid directory".to_string());
                }
            }
            KeyCode::Char(c) => {
                self.path_input.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                self.error_message = None;
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.path_input.remove(self.cursor_pos);
                    self.error_message = None;
                }
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.path_input.len() {
                    self.path_input.remove(self.cursor_pos);
                    self.error_message = None;
                }
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor_pos < self.path_input.len() {
                    self.cursor_pos += 1;
                }
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
            }
            KeyCode::End => {
                self.cursor_pos = self.path_input.len();
            }
            _ => {}
        }
        None
    }

    /// Get the current working directory being browsed
    pub fn current_browse_path(&self) -> PathBuf {
        self.file_explorer.cwd().to_path_buf()
    }
}

/// Expand ~ to home directory in a path string
fn expand_tilde(path: &str) -> String {
    if path.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            return path.replacen('~', &home.to_string_lossy(), 1);
        }
    }
    path.to_string()
}

/// Widget for rendering the new project modal
pub struct NewProjectModal<'a> {
    state: &'a mut NewProjectModalState,
}

impl<'a> NewProjectModal<'a> {
    pub fn new(state: &'a mut NewProjectModalState) -> Self {
        Self { state }
    }

    /// Calculate the modal area (centered, 60% width, 70% height)
    pub fn calculate_area(total: Rect) -> Rect {
        let width = (total.width * 60 / 100)
            .max(40)
            .min(total.width.saturating_sub(4));
        let height = (total.height * 70 / 100)
            .max(15)
            .min(total.height.saturating_sub(4));

        let x = (total.width.saturating_sub(width)) / 2;
        let y = (total.height.saturating_sub(height)) / 2;

        Rect::new(x, y, width, height)
    }
}

impl Widget for NewProjectModal<'_> {
    fn render(mut self, area: Rect, buf: &mut Buffer) {
        // Skip rendering if area is too small
        if area.width < 30 || area.height < 10 {
            return;
        }

        // Clear the area first
        Clear.render(area, buf);

        // Main modal block
        let block = Block::default()
            .title(" New Project ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        // Layout: tab bar, content, help bar
        let chunks = Layout::vertical([
            Constraint::Length(3), // Tab bar
            Constraint::Min(5),    // Content
            Constraint::Length(2), // Help bar
        ])
        .split(inner);

        // Render tab bar
        self.render_tab_bar(chunks[0], buf);

        // Render content based on active tab
        match self.state.active_tab {
            NewProjectTab::Browse => self.render_browse_tab(chunks[1], buf),
            NewProjectTab::EnterPath => self.render_path_input_tab(chunks[1], buf),
        }

        // Render help bar
        self.render_help_bar(chunks[2], buf);
    }
}

impl NewProjectModal<'_> {
    fn render_tab_bar(&self, area: Rect, buf: &mut Buffer) {
        let browse_style = if self.state.active_tab == NewProjectTab::Browse {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let path_style = if self.state.active_tab == NewProjectTab::EnterPath {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let tabs = Line::from(vec![
            Span::raw("  "),
            Span::styled(" Browse ", browse_style),
            Span::raw(" | "),
            Span::styled(" Enter Path ", path_style),
            Span::raw("  "),
        ]);

        let tab_paragraph = Paragraph::new(tabs).alignment(Alignment::Center);
        tab_paragraph.render(area, buf);
    }

    fn render_browse_tab(&mut self, area: Rect, buf: &mut Buffer) {
        // Show current directory at top
        let cwd = self.state.file_explorer.cwd();
        let cwd_display = cwd.to_string_lossy();

        let chunks = Layout::vertical([
            Constraint::Length(1), // Current path
            Constraint::Min(3),    // File explorer
            Constraint::Length(1), // Error message
        ])
        .split(area);

        // Current path header
        let path_line = Line::from(vec![
            Span::styled("Path: ", Style::default().fg(Color::DarkGray)),
            Span::styled(cwd_display.to_string(), Style::default().fg(Color::Yellow)),
        ]);
        Paragraph::new(path_line).render(chunks[0], buf);

        // File explorer widget
        let explorer_widget = self.state.file_explorer.widget();
        explorer_widget.render(chunks[1], buf);

        // Error message if any
        if let Some(ref error) = self.state.error_message {
            let error_line =
                Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red)));
            Paragraph::new(error_line).render(chunks[2], buf);
        }
    }

    fn render_path_input_tab(&self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::vertical([
            Constraint::Length(2), // Label
            Constraint::Length(3), // Input field
            Constraint::Length(1), // Error message
            Constraint::Min(0),    // Spacer
        ])
        .split(area);

        // Label
        let label = Paragraph::new("Enter the project directory path:")
            .style(Style::default().fg(Color::White));
        label.render(chunks[0], buf);

        // Input field with border
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let input_inner = input_block.inner(chunks[1]);
        input_block.render(chunks[1], buf);

        // Render the input text with cursor
        let display_text = &self.state.path_input;
        let cursor_pos = self.state.cursor_pos;

        // Calculate visible portion if text is too long
        let available_width = input_inner.width as usize;
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
                    Style::default().bg(Color::White).fg(Color::Black),
                ));
            } else {
                spans.push(Span::raw(c.to_string()));
            }
        }
        // If cursor is at the end, show a block cursor
        if cursor_offset >= visible_text.len() {
            spans.push(Span::styled(" ", Style::default().bg(Color::White)));
        }

        let input_line = Line::from(spans);
        Paragraph::new(input_line).render(input_inner, buf);

        // Error message if any
        if let Some(ref error) = self.state.error_message {
            let error_line =
                Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red)));
            Paragraph::new(error_line).render(chunks[2], buf);
        }
    }

    fn render_help_bar(&self, area: Rect, buf: &mut Buffer) {
        let help_text = match self.state.active_tab {
            NewProjectTab::Browse => vec![
                Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
                Span::raw("switch "),
                Span::styled(" h/j/k/l ", Style::default().fg(Color::Cyan)),
                Span::raw("nav "),
                Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
                Span::raw("select "),
                Span::styled(" Esc ", Style::default().fg(Color::Cyan)),
                Span::raw("cancel"),
            ],
            NewProjectTab::EnterPath => vec![
                Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
                Span::raw("switch "),
                Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
                Span::raw("confirm "),
                Span::styled(" Esc ", Style::default().fg(Color::Cyan)),
                Span::raw("cancel"),
            ],
        };

        let help = Paragraph::new(Line::from(help_text))
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        help.render(area, buf);
    }
}
