//! Workspace management modal: add/remove workspace directory prefixes.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget},
};
use ratatui_explorer::FileExplorer;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Which section of the modal has focus
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceModalFocus {
    /// Browsing current workspace directories
    CurrentList,
    /// Browsing file explorer to add directories
    AvailableList,
}

/// Result of handling a key in the workspace modal
pub enum WorkspaceModalKeyResult {
    Continue,
    Close,
    Added(String),
    Removed(usize),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// State for the workspace management modal
pub struct WorkspaceModalState {
    /// Current workspace directories (from config)
    pub workspaces: Vec<String>,
    /// File explorer for browsing directories
    pub file_explorer: FileExplorer,
    /// Which section has focus
    pub focus: WorkspaceModalFocus,
    /// Selected index in current workspaces list
    pub selected_current: usize,
    /// ListState for current workspaces
    pub list_state_current: ListState,
}

impl WorkspaceModalState {
    pub fn new(workspaces: Vec<String>) -> Self {
        let file_explorer = FileExplorer::new()
            .unwrap_or_else(|_| FileExplorer::new().expect("Failed to create file explorer"));
        let mut list_state_current = ListState::default();
        if !workspaces.is_empty() {
            list_state_current.select(Some(0));
        }
        Self {
            workspaces,
            file_explorer,
            focus: WorkspaceModalFocus::AvailableList,
            selected_current: 0,
            list_state_current,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> WorkspaceModalKeyResult {
        // Global keys
        match key.code {
            KeyCode::Esc => return WorkspaceModalKeyResult::Close,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    WorkspaceModalFocus::CurrentList => WorkspaceModalFocus::AvailableList,
                    WorkspaceModalFocus::AvailableList => WorkspaceModalFocus::CurrentList,
                };
                return WorkspaceModalKeyResult::Continue;
            }
            _ => {}
        }

        match self.focus {
            WorkspaceModalFocus::CurrentList => self.handle_current_list_key(key),
            WorkspaceModalFocus::AvailableList => self.handle_available_list_key(key),
        }
    }

    fn handle_current_list_key(&mut self, key: KeyEvent) -> WorkspaceModalKeyResult {
        match key.code {
            // Navigation
            KeyCode::Down | KeyCode::Char('j') if !self.workspaces.is_empty() => {
                self.selected_current =
                    (self.selected_current + 1).min(self.workspaces.len().saturating_sub(1));
                self.list_state_current.select(Some(self.selected_current));
                WorkspaceModalKeyResult::Continue
            }
            KeyCode::Up | KeyCode::Char('k') if !self.workspaces.is_empty() => {
                self.selected_current = self.selected_current.saturating_sub(1);
                self.list_state_current.select(Some(self.selected_current));
                WorkspaceModalKeyResult::Continue
            }
            // Remove selected workspace
            KeyCode::Char('d') | KeyCode::Delete if !self.workspaces.is_empty() => {
                let idx = self.selected_current;
                WorkspaceModalKeyResult::Removed(idx)
            }
            _ => WorkspaceModalKeyResult::Continue,
        }
    }

    fn handle_available_list_key(&mut self, key: KeyEvent) -> WorkspaceModalKeyResult {
        match key.code {
            KeyCode::Enter => {
                let current = self.file_explorer.current();
                let path = current.path().clone();

                // If it's a file, use its parent directory
                let dir_path = if path.is_file() {
                    path.parent().map(Path::to_path_buf).unwrap_or(path)
                } else {
                    path
                };

                if dir_path.is_dir() {
                    return WorkspaceModalKeyResult::Added(dir_path.to_string_lossy().into_owned());
                }
                WorkspaceModalKeyResult::Continue
            }
            // Vim-style navigation
            KeyCode::Char('h') | KeyCode::Left => {
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Left,
                        crossterm::event::KeyModifiers::NONE,
                    )));
                WorkspaceModalKeyResult::Continue
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Down,
                        crossterm::event::KeyModifiers::NONE,
                    )));
                WorkspaceModalKeyResult::Continue
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Up,
                        crossterm::event::KeyModifiers::NONE,
                    )));
                WorkspaceModalKeyResult::Continue
            }
            KeyCode::Char('l') | KeyCode::Right => {
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Right,
                        crossterm::event::KeyModifiers::NONE,
                    )));
                WorkspaceModalKeyResult::Continue
            }
            KeyCode::Char('g') => {
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Home,
                        crossterm::event::KeyModifiers::NONE,
                    )));
                WorkspaceModalKeyResult::Continue
            }
            KeyCode::Char('G') => {
                let _ = self
                    .file_explorer
                    .handle(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::End,
                        crossterm::event::KeyModifiers::NONE,
                    )));
                WorkspaceModalKeyResult::Continue
            }
            _ => WorkspaceModalKeyResult::Continue,
        }
    }
}

impl super::Modal for WorkspaceModalState {
    fn handle_key_modal(&mut self, key: KeyEvent) -> super::ModalKeyResult {
        match self.handle_key(key) {
            WorkspaceModalKeyResult::Continue => super::ModalKeyResult::Continue,
            WorkspaceModalKeyResult::Close => super::ModalKeyResult::Close,
            WorkspaceModalKeyResult::Added(path) => super::ModalKeyResult::WorkspaceAdded(path),
            WorkspaceModalKeyResult::Removed(idx) => super::ModalKeyResult::WorkspaceRemoved(idx),
        }
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct WorkspaceModal<'a> {
    state: &'a mut WorkspaceModalState,
}

impl<'a> WorkspaceModal<'a> {
    pub fn new(state: &'a mut WorkspaceModalState) -> Self {
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

impl Widget for WorkspaceModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 40 || area.height < 10 {
            return;
        }

        Clear.render(area, buf);

        let outer_block = Block::default()
            .title(" Manage Workspaces ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = outer_block.inner(area);
        outer_block.render(area, buf);

        // Split inner into: file explorer (top/larger), current workspaces (bottom/smaller), help bar
        let chunks = Layout::vertical([
            Constraint::Percentage(55), // File explorer section
            Constraint::Percentage(40), // Current workspaces section
            Constraint::Length(1),      // Help bar
        ])
        .split(inner);

        // File explorer section (top)
        render_file_explorer(self.state, chunks[0], buf);

        // Current workspaces section (bottom)
        render_current_workspaces(self.state, chunks[1], buf);

        // Help bar
        let help = Paragraph::new(Line::from(vec![
            Span::styled(
                " Tab ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("switch "),
            Span::styled(
                " h/j/k/l ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("nav "),
            Span::styled(
                " d ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("remove "),
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("add "),
            Span::styled(
                " Esc ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("close"),
        ]))
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
        help.render(chunks[2], buf);
    }
}

// ---------------------------------------------------------------------------
// Render helpers
// ---------------------------------------------------------------------------

fn render_file_explorer(state: &mut WorkspaceModalState, area: Rect, buf: &mut Buffer) {
    let is_focused = state.focus == WorkspaceModalFocus::AvailableList;
    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let cwd = state.file_explorer.cwd();
    let cwd_display = cwd.to_string_lossy();
    let title = format!(" Browse: {} ", cwd_display);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    block.render(area, buf);

    let explorer_widget = state.file_explorer.widget();
    explorer_widget.render(inner, buf);
}

fn render_current_workspaces(state: &mut WorkspaceModalState, area: Rect, buf: &mut Buffer) {
    let is_focused = state.focus == WorkspaceModalFocus::CurrentList;
    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(" Current Workspaces ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let list_area = block.inner(area);
    block.render(area, buf);

    if state.workspaces.is_empty() {
        Paragraph::new("No workspaces configured")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .render(list_area, buf);
        return;
    }

    let items: Vec<ListItem> = state
        .workspaces
        .iter()
        .map(|path| {
            ListItem::new(Line::from(Span::styled(
                path.clone(),
                Style::default().fg(Color::White),
            )))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    ratatui::widgets::StatefulWidget::render(list, list_area, buf, &mut state.list_state_current);
}
