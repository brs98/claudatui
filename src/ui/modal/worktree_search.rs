//! Worktree search modal: browse projects then create a worktree.
//!
//! Phase 1 — select a git project from a searchable list.
//! Phase 2 — enter a branch name for the selected project.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

/// A unique git project collected from the sidebar groups.
#[derive(Debug, Clone)]
pub struct WorktreeProject {
    /// Human-readable name (repo display name).
    pub display_name: String,
    /// A project path that belongs to this repo (used for worktree creation).
    pub project_path: PathBuf,
    /// The canonical repo path (bare dir or normal repo root).
    pub repo_path: PathBuf,
}

/// Which phase the modal is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeSearchPhase {
    /// Browsing / filtering the project list.
    ProjectSelect,
    /// Entering a branch name for the selected project.
    BranchInput,
}

/// Result of handling a key event.
pub enum WorktreeSearchKeyResult {
    /// Nothing to do externally.
    Continue,
    /// The search query changed — caller should call `refilter()`.
    QueryChanged,
    /// User confirmed a project + branch name.
    Confirmed {
        project_path: PathBuf,
        branch_name: String,
    },
    /// User wants to close the modal entirely.
    Close,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct WorktreeSearchModalState {
    pub phase: WorktreeSearchPhase,

    // Phase 1 — project selection
    pub query: String,
    pub cursor_pos: usize,
    pub all_projects: Vec<WorktreeProject>,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    pub list_state: ListState,

    // Phase 2 — branch input
    pub branch_input: String,
    pub branch_cursor_pos: usize,
    pub error_message: Option<String>,
    pub selected_project: Option<WorktreeProject>,
}

impl WorktreeSearchModalState {
    pub fn new(projects: Vec<WorktreeProject>) -> Self {
        let filtered_indices: Vec<usize> = (0..projects.len()).collect();
        let mut list_state = ListState::default();
        if !filtered_indices.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            phase: WorktreeSearchPhase::ProjectSelect,
            query: String::new(),
            cursor_pos: 0,
            all_projects: projects,
            filtered_indices,
            selected: 0,
            list_state,
            branch_input: String::new(),
            branch_cursor_pos: 0,
            error_message: None,
            selected_project: None,
        }
    }

    /// Re-filter the project list based on the current query.
    pub fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered_indices = self
            .all_projects
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                if q.is_empty() {
                    return true;
                }
                p.display_name.to_lowercase().contains(&q)
                    || p.project_path.to_string_lossy().to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();

        // Clamp selection
        if self.filtered_indices.is_empty() {
            self.selected = 0;
            self.list_state.select(None);
        } else {
            self.selected = self.selected.min(self.filtered_indices.len() - 1);
            self.list_state.select(Some(self.selected));
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> WorktreeSearchKeyResult {
        match self.phase {
            WorktreeSearchPhase::ProjectSelect => self.handle_project_select_key(key),
            WorktreeSearchPhase::BranchInput => self.handle_branch_input_key(key),
        }
    }

    // -- Phase 1 key handling ------------------------------------------------

    fn handle_project_select_key(&mut self, key: KeyEvent) -> WorktreeSearchKeyResult {
        match key.code {
            KeyCode::Esc => WorktreeSearchKeyResult::Close,

            // Navigation
            KeyCode::Down if !self.filtered_indices.is_empty() => {
                self.selected =
                    (self.selected + 1).min(self.filtered_indices.len().saturating_sub(1));
                self.list_state.select(Some(self.selected));
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Up if !self.filtered_indices.is_empty() => {
                self.selected = self.selected.saturating_sub(1);
                self.list_state.select(Some(self.selected));
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Char('j')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !self.filtered_indices.is_empty() =>
            {
                self.selected =
                    (self.selected + 1).min(self.filtered_indices.len().saturating_sub(1));
                self.list_state.select(Some(self.selected));
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Char('k')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !self.filtered_indices.is_empty() =>
            {
                self.selected = self.selected.saturating_sub(1);
                self.list_state.select(Some(self.selected));
                WorktreeSearchKeyResult::Continue
            }

            // Select project → transition to Phase 2
            KeyCode::Enter => {
                if let Some(&idx) = self.filtered_indices.get(self.selected) {
                    let project = self.all_projects[idx].clone();
                    self.selected_project = Some(project);
                    self.phase = WorktreeSearchPhase::BranchInput;
                    self.branch_input.clear();
                    self.branch_cursor_pos = 0;
                    self.error_message = None;
                }
                WorktreeSearchKeyResult::Continue
            }

            // Text input for search query
            KeyCode::Char(c) => {
                self.query.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                WorktreeSearchKeyResult::QueryChanged
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.query.remove(self.cursor_pos);
                    WorktreeSearchKeyResult::QueryChanged
                } else {
                    WorktreeSearchKeyResult::Continue
                }
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.query.len() {
                    self.query.remove(self.cursor_pos);
                    WorktreeSearchKeyResult::QueryChanged
                } else {
                    WorktreeSearchKeyResult::Continue
                }
            }
            KeyCode::Left => {
                self.cursor_pos = self.cursor_pos.saturating_sub(1);
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Right => {
                self.cursor_pos = (self.cursor_pos + 1).min(self.query.len());
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::End => {
                self.cursor_pos = self.query.len();
                WorktreeSearchKeyResult::Continue
            }
            _ => WorktreeSearchKeyResult::Continue,
        }
    }

    // -- Phase 2 key handling ------------------------------------------------

    fn handle_branch_input_key(&mut self, key: KeyEvent) -> WorktreeSearchKeyResult {
        match key.code {
            // Esc goes back to Phase 1 (not close)
            KeyCode::Esc => {
                self.phase = WorktreeSearchPhase::ProjectSelect;
                self.error_message = None;
                WorktreeSearchKeyResult::Continue
            }

            KeyCode::Enter => {
                let branch = self.branch_input.trim().to_string();
                if branch.is_empty() {
                    self.error_message = Some("Branch name cannot be empty".to_string());
                    return WorktreeSearchKeyResult::Continue;
                }
                if branch.contains(' ') || branch.contains("..") || branch.starts_with('-') {
                    self.error_message = Some(
                        "Invalid branch name (no spaces, '..', or leading '-')".to_string(),
                    );
                    return WorktreeSearchKeyResult::Continue;
                }
                if let Some(ref project) = self.selected_project {
                    WorktreeSearchKeyResult::Confirmed {
                        project_path: project.project_path.clone(),
                        branch_name: branch,
                    }
                } else {
                    WorktreeSearchKeyResult::Continue
                }
            }

            KeyCode::Char(c) => {
                self.branch_input.insert(self.branch_cursor_pos, c);
                self.branch_cursor_pos += 1;
                self.error_message = None;
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Backspace => {
                if self.branch_cursor_pos > 0 {
                    self.branch_cursor_pos -= 1;
                    self.branch_input.remove(self.branch_cursor_pos);
                    self.error_message = None;
                }
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Delete => {
                if self.branch_cursor_pos < self.branch_input.len() {
                    self.branch_input.remove(self.branch_cursor_pos);
                    self.error_message = None;
                }
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Left => {
                if self.branch_cursor_pos > 0 {
                    self.branch_cursor_pos -= 1;
                }
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Right => {
                if self.branch_cursor_pos < self.branch_input.len() {
                    self.branch_cursor_pos += 1;
                }
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::Home => {
                self.branch_cursor_pos = 0;
                WorktreeSearchKeyResult::Continue
            }
            KeyCode::End => {
                self.branch_cursor_pos = self.branch_input.len();
                WorktreeSearchKeyResult::Continue
            }
            _ => WorktreeSearchKeyResult::Continue,
        }
    }
}

impl super::Modal for WorktreeSearchModalState {
    fn handle_key_modal(&mut self, key: KeyEvent) -> super::ModalKeyResult {
        match self.handle_key(key) {
            WorktreeSearchKeyResult::Continue => super::ModalKeyResult::Continue,
            WorktreeSearchKeyResult::QueryChanged => {
                self.refilter();
                super::ModalKeyResult::WorktreeSearchQueryChanged
            }
            WorktreeSearchKeyResult::Confirmed {
                project_path,
                branch_name,
            } => super::ModalKeyResult::WorktreeSearchConfirmed {
                project_path,
                branch_name,
            },
            WorktreeSearchKeyResult::Close => super::ModalKeyResult::Close,
        }
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct WorktreeSearchModal<'a> {
    state: &'a mut WorktreeSearchModalState,
}

impl<'a> WorktreeSearchModal<'a> {
    pub fn new(state: &'a mut WorktreeSearchModalState) -> Self {
        Self { state }
    }

    /// Calculate the modal area (centered, 70% width, 60% height).
    pub fn calculate_area(total: Rect) -> Rect {
        let width = (total.width * 70 / 100)
            .max(50)
            .min(total.width.saturating_sub(4));
        let height = (total.height * 60 / 100)
            .max(12)
            .min(total.height.saturating_sub(4));

        let x = (total.width.saturating_sub(width)) / 2;
        let y = (total.height.saturating_sub(height)) / 2;

        Rect::new(x, y, width, height)
    }
}

impl Widget for WorktreeSearchModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 40 || area.height < 10 {
            return;
        }

        Clear.render(area, buf);

        match self.state.phase {
            WorktreeSearchPhase::ProjectSelect => render_project_select(self.state, area, buf),
            WorktreeSearchPhase::BranchInput => render_branch_input(self.state, area, buf),
        }
    }
}

// ---------------------------------------------------------------------------
// Render helpers
// ---------------------------------------------------------------------------

fn render_project_select(state: &mut WorktreeSearchModalState, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(" New Worktree — Select Project ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    block.render(area, buf);

    let chunks = Layout::vertical([
        Constraint::Length(3), // Search input
        Constraint::Min(3),    // Project list
        Constraint::Length(1), // Help bar
    ])
    .split(inner);

    // Search input
    render_search_input(state, chunks[0], buf);

    // Project list
    render_project_list(state, chunks[1], buf);

    // Help bar
    let help = Paragraph::new(Line::from(vec![
        Span::styled(
            " C-j/k ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("nav "),
        Span::styled(
            " Enter ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("select "),
        Span::styled(
            " Esc ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("cancel"),
    ]))
    .style(Style::default().fg(Color::DarkGray))
    .alignment(Alignment::Center);
    help.render(chunks[2], buf);
}

fn render_search_input(state: &WorktreeSearchModalState, area: Rect, buf: &mut Buffer) {
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Search ");
    let input_inner = input_block.inner(area);
    input_block.render(area, buf);

    let display_text = &state.query;
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

fn render_project_list(state: &mut WorktreeSearchModalState, area: Rect, buf: &mut Buffer) {
    if state.filtered_indices.is_empty() {
        let message = if state.query.is_empty() {
            "No git repositories found"
        } else {
            "No matching projects"
        };
        Paragraph::new(message)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .render(area, buf);
        return;
    }

    let items: Vec<ListItem> = state
        .filtered_indices
        .iter()
        .map(|&idx| {
            let project = &state.all_projects[idx];
            let path_str = project.project_path.to_string_lossy();
            ListItem::new(Line::from(vec![
                Span::styled(
                    project.display_name.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}", path_str), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    ratatui::widgets::StatefulWidget::render(list, area, buf, &mut state.list_state);
}

fn render_branch_input(state: &WorktreeSearchModalState, area: Rect, buf: &mut Buffer) {
    let project_name = state
        .selected_project
        .as_ref()
        .map(|p| p.display_name.as_str())
        .unwrap_or("?");
    let title = format!(" New Worktree — {} ", project_name);

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
    Paragraph::new("Branch name:")
        .style(Style::default().fg(Color::White))
        .render(chunks[0], buf);

    // Input field with border
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let input_inner = input_block.inner(chunks[1]);
    input_block.render(chunks[1], buf);

    // Render input text with cursor
    let display_text = &state.branch_input;
    let cursor_pos = state.branch_cursor_pos;
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
    if let Some(ref error) = state.error_message {
        Paragraph::new(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::Red),
        )))
        .render(chunks[2], buf);
    }

    // Help bar
    let help = Paragraph::new(Line::from(vec![
        Span::styled(
            " Enter ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("create "),
        Span::styled(
            " Esc ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("back"),
    ]))
    .style(Style::default().fg(Color::DarkGray))
    .alignment(Alignment::Center);
    help.render(chunks[4], buf);
}
