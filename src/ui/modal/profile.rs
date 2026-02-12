//! Profile management modal: create, rename, delete, and activate profiles.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget},
};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Sub-mode within the profile modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileModalMode {
    /// Navigating the profile list.
    List,
    /// Text input for creating or renaming a profile.
    Input,
}

/// What the current text input is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileInputPurpose {
    /// Creating a brand-new profile.
    Create,
    /// Renaming an existing profile at the given index.
    Rename(usize),
}

/// Internal key-handling result (translated to `ModalKeyResult` via the trait).
enum ProfileModalKeyResult {
    Continue,
    Close,
    Created(String),
    Renamed { index: usize, new_name: String },
    Deleted(usize),
    Activated(usize),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// State for the profile management modal dialog.
pub struct ProfileModalState {
    /// Snapshot of profile names (kept in sync by the `App` action methods).
    pub profiles: Vec<String>,
    /// Currently highlighted index in the list.
    pub selected: usize,
    /// Ratatui list widget state.
    pub list_state: ListState,
    /// Current sub-mode (list navigation vs text input).
    pub mode: ProfileModalMode,
    /// Purpose of the current text input, if any.
    pub input_purpose: Option<ProfileInputPurpose>,
    /// Text buffer for name input.
    pub input_buffer: String,
    /// Cursor position within `input_buffer`.
    pub cursor_pos: usize,
    /// Inline error message (displayed in red).
    pub error_message: Option<String>,
    /// Index of the currently active profile (for the "(active)" marker).
    pub active_profile: Option<usize>,
}

impl ProfileModalState {
    pub fn new(profiles: Vec<String>, active_profile: Option<usize>) -> Self {
        let mut list_state = ListState::default();
        if !profiles.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            profiles,
            selected: 0,
            list_state,
            mode: ProfileModalMode::List,
            input_purpose: None,
            input_buffer: String::new(),
            cursor_pos: 0,
            error_message: None,
            active_profile,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> ProfileModalKeyResult {
        match self.mode {
            ProfileModalMode::List => self.handle_list_key(key),
            ProfileModalMode::Input => self.handle_input_key(key),
        }
    }

    // -- List mode -----------------------------------------------------------

    fn handle_list_key(&mut self, key: KeyEvent) -> ProfileModalKeyResult {
        match key.code {
            KeyCode::Esc => ProfileModalKeyResult::Close,

            // Navigation
            KeyCode::Char('j') | KeyCode::Down if !self.profiles.is_empty() => {
                self.selected =
                    (self.selected + 1).min(self.profiles.len().saturating_sub(1));
                self.list_state.select(Some(self.selected));
                self.error_message = None;
                ProfileModalKeyResult::Continue
            }
            KeyCode::Char('k') | KeyCode::Up if !self.profiles.is_empty() => {
                self.selected = self.selected.saturating_sub(1);
                self.list_state.select(Some(self.selected));
                self.error_message = None;
                ProfileModalKeyResult::Continue
            }

            // Create
            KeyCode::Char('n') => {
                self.mode = ProfileModalMode::Input;
                self.input_purpose = Some(ProfileInputPurpose::Create);
                self.input_buffer.clear();
                self.cursor_pos = 0;
                self.error_message = None;
                ProfileModalKeyResult::Continue
            }

            // Rename
            KeyCode::Char('r') if !self.profiles.is_empty() => {
                let idx = self.selected;
                self.mode = ProfileModalMode::Input;
                self.input_purpose = Some(ProfileInputPurpose::Rename(idx));
                self.input_buffer.clone_from(&self.profiles[idx]);
                self.cursor_pos = self.input_buffer.len();
                self.error_message = None;
                ProfileModalKeyResult::Continue
            }

            // Delete
            KeyCode::Char('d' | 'x') if !self.profiles.is_empty() => {
                let idx = self.selected;
                ProfileModalKeyResult::Deleted(idx)
            }

            // Activate
            KeyCode::Enter if !self.profiles.is_empty() => {
                ProfileModalKeyResult::Activated(self.selected)
            }

            _ => ProfileModalKeyResult::Continue,
        }
    }

    // -- Input mode ----------------------------------------------------------

    fn handle_input_key(&mut self, key: KeyEvent) -> ProfileModalKeyResult {
        match key.code {
            KeyCode::Esc => {
                // Cancel back to list mode
                self.mode = ProfileModalMode::List;
                self.input_purpose = None;
                self.input_buffer.clear();
                self.cursor_pos = 0;
                self.error_message = None;
                ProfileModalKeyResult::Continue
            }
            KeyCode::Enter => self.validate_and_confirm(),
            KeyCode::Char(c) => {
                self.input_buffer.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                self.error_message = None;
                ProfileModalKeyResult::Continue
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.input_buffer.remove(self.cursor_pos);
                    self.error_message = None;
                }
                ProfileModalKeyResult::Continue
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.input_buffer.len() {
                    self.input_buffer.remove(self.cursor_pos);
                    self.error_message = None;
                }
                ProfileModalKeyResult::Continue
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
                ProfileModalKeyResult::Continue
            }
            KeyCode::Right => {
                if self.cursor_pos < self.input_buffer.len() {
                    self.cursor_pos += 1;
                }
                ProfileModalKeyResult::Continue
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                ProfileModalKeyResult::Continue
            }
            KeyCode::End => {
                self.cursor_pos = self.input_buffer.len();
                ProfileModalKeyResult::Continue
            }
            _ => ProfileModalKeyResult::Continue,
        }
    }

    /// Validate the input buffer and return the appropriate result.
    fn validate_and_confirm(&mut self) -> ProfileModalKeyResult {
        let name = self.input_buffer.trim().to_string();

        if name.is_empty() {
            self.error_message = Some("Name cannot be empty".to_string());
            return ProfileModalKeyResult::Continue;
        }

        // Case-insensitive duplicate check, excluding self when renaming
        let exclude_idx = match self.input_purpose {
            Some(ProfileInputPurpose::Rename(idx)) => Some(idx),
            _ => None,
        };
        let is_duplicate = self
            .profiles
            .iter()
            .enumerate()
            .any(|(i, p)| Some(i) != exclude_idx && p.eq_ignore_ascii_case(&name));

        if is_duplicate {
            self.error_message = Some("A profile with this name already exists".to_string());
            return ProfileModalKeyResult::Continue;
        }

        match self.input_purpose {
            Some(ProfileInputPurpose::Create) => ProfileModalKeyResult::Created(name),
            Some(ProfileInputPurpose::Rename(idx)) => ProfileModalKeyResult::Renamed {
                index: idx,
                new_name: name,
            },
            None => ProfileModalKeyResult::Continue,
        }
    }
}

impl super::Modal for ProfileModalState {
    fn handle_key_modal(&mut self, key: KeyEvent) -> super::ModalKeyResult {
        match self.handle_key(key) {
            ProfileModalKeyResult::Continue => super::ModalKeyResult::Continue,
            ProfileModalKeyResult::Close => super::ModalKeyResult::Close,
            ProfileModalKeyResult::Created(name) => super::ModalKeyResult::ProfileCreated(name),
            ProfileModalKeyResult::Renamed { index, new_name } => {
                super::ModalKeyResult::ProfileRenamed { index, new_name }
            }
            ProfileModalKeyResult::Deleted(idx) => super::ModalKeyResult::ProfileDeleted(idx),
            ProfileModalKeyResult::Activated(idx) => super::ModalKeyResult::ProfileActivated(idx),
        }
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// Widget for rendering the profile management modal.
pub struct ProfileModal<'a> {
    state: &'a ProfileModalState,
}

impl<'a> ProfileModal<'a> {
    pub fn new(state: &'a ProfileModalState) -> Self {
        Self { state }
    }

    /// Calculate the modal area (centered, ~50% width, ~50% height).
    pub fn calculate_area(total: Rect) -> Rect {
        let width = (total.width * 50 / 100)
            .max(40)
            .min(total.width.saturating_sub(4));
        let height = (total.height * 50 / 100)
            .max(12)
            .min(total.height.saturating_sub(4));

        let x = (total.width.saturating_sub(width)) / 2;
        let y = (total.height.saturating_sub(height)) / 2;

        Rect::new(x, y, width, height)
    }
}

impl Widget for ProfileModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 30 || area.height < 8 {
            return;
        }

        Clear.render(area, buf);

        let outer_block = Block::default()
            .title(" Manage Profiles ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = outer_block.inner(area);
        outer_block.render(area, buf);

        // Determine layout based on whether we're in input mode
        let is_input = self.state.mode == ProfileModalMode::Input;

        let chunks = if is_input {
            Layout::vertical([
                Constraint::Min(3),    // Profile list
                Constraint::Length(1), // Input label
                Constraint::Length(3), // Input field
                Constraint::Length(1), // Error message
                Constraint::Length(1), // Help bar
            ])
            .split(inner)
        } else {
            Layout::vertical([
                Constraint::Min(3),    // Profile list
                Constraint::Length(1), // Error message
                Constraint::Length(1), // Help bar
            ])
            .split(inner)
        };

        // Profile list
        render_profile_list(self.state, chunks[0], buf);

        if is_input {
            // Input label
            let label_text = match self.state.input_purpose {
                Some(ProfileInputPurpose::Create) => "New profile name:",
                Some(ProfileInputPurpose::Rename(_)) => "Rename to:",
                None => "",
            };
            Paragraph::new(label_text)
                .style(Style::default().fg(Color::White))
                .render(chunks[1], buf);

            // Input field with border
            render_input_field(self.state, chunks[2], buf);

            // Error message
            if let Some(ref error) = self.state.error_message {
                Paragraph::new(Line::from(Span::styled(
                    error.clone(),
                    Style::default().fg(Color::Red),
                )))
                .render(chunks[3], buf);
            }

            // Help bar (input mode)
            render_input_help_bar(chunks[4], buf);
        } else {
            // Error message
            if let Some(ref error) = self.state.error_message {
                Paragraph::new(Line::from(Span::styled(
                    error.clone(),
                    Style::default().fg(Color::Red),
                )))
                .render(chunks[1], buf);
            }

            // Help bar (list mode)
            render_list_help_bar(self.state.profiles.is_empty(), chunks[2], buf);
        }
    }
}

// ---------------------------------------------------------------------------
// Render helpers
// ---------------------------------------------------------------------------

fn render_profile_list(state: &ProfileModalState, area: Rect, buf: &mut Buffer) {
    if state.profiles.is_empty() {
        Paragraph::new("No profiles. Press n to create one.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .render(area, buf);
        return;
    }

    let items: Vec<ListItem> = state
        .profiles
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let is_active = state.active_profile == Some(i);
            let mut spans = vec![Span::styled(name.clone(), Style::default().fg(Color::White))];
            if is_active {
                spans.push(Span::styled(
                    " (active)",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::ITALIC),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    // We need a mutable ListState for rendering; clone from the immutable reference
    let mut list_state = state.list_state.clone();
    ratatui::widgets::StatefulWidget::render(list, area, buf, &mut list_state);
}

fn render_input_field(state: &ProfileModalState, area: Rect, buf: &mut Buffer) {
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let input_inner = input_block.inner(area);
    input_block.render(area, buf);

    let display_text = &state.input_buffer;
    let cursor_pos = state.cursor_pos;
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
}

fn render_list_help_bar(is_empty: bool, area: Rect, buf: &mut Buffer) {
    let mut spans = vec![
        Span::styled(
            " n ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("new "),
    ];

    if !is_empty {
        spans.extend([
            Span::styled(
                " r ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("rename "),
            Span::styled(
                " d ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("delete "),
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("activate "),
        ]);
    }

    spans.extend([
        Span::styled(
            " Esc ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("close"),
    ]);

    Paragraph::new(Line::from(spans))
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center)
        .render(area, buf);
}

fn render_input_help_bar(area: Rect, buf: &mut Buffer) {
    let spans = vec![
        Span::styled(
            " Enter ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("confirm "),
        Span::styled(
            " Esc ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("cancel"),
    ];

    Paragraph::new(Line::from(spans))
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center)
        .render(area, buf);
}
