//! Search modal dialog for finding conversations.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget},
};

use crate::search::{SearchFilterType, SearchQuery, SearchResult};

/// State for the search modal dialog
pub struct SearchModalState {
    /// Current search query text
    pub query: String,
    /// Cursor position in query
    pub cursor_pos: usize,
    /// Current filter type
    pub filter_type: SearchFilterType,
    /// Search results
    pub results: Vec<SearchResult>,
    /// Selected result index
    pub selected: usize,
    /// List state for scrolling
    pub list_state: ListState,
}

impl Default for SearchModalState {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchModalState {
    /// Create a new search modal state
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            query: String::new(),
            cursor_pos: 0,
            filter_type: SearchFilterType::All,
            results: Vec::new(),
            selected: 0,
            list_state,
        }
    }

    /// Get the current search query
    pub fn search_query(&self) -> SearchQuery {
        SearchQuery::new(&self.query, self.filter_type)
    }

    /// Update search results
    pub fn set_results(&mut self, results: Vec<SearchResult>) {
        self.results = results;
        // Reset selection if out of bounds
        if self.selected >= self.results.len() && !self.results.is_empty() {
            self.selected = self.results.len() - 1;
        }
        self.list_state.select(if self.results.is_empty() {
            None
        } else {
            Some(self.selected)
        });
    }

    /// Get the selected result
    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected)
    }

    /// Handle key input for the modal
    /// Returns Some(session_id) if a result was selected, None otherwise
    pub fn handle_key(&mut self, key: KeyEvent) -> SearchKeyResult {
        match key.code {
            // Tab cycles filter type
            KeyCode::Tab => {
                self.filter_type = self.filter_type.next();
                SearchKeyResult::QueryChanged
            }
            // Navigation
            KeyCode::Down if !self.results.is_empty() => {
                self.selected = (self.selected + 1).min(self.results.len().saturating_sub(1));
                self.list_state.select(Some(self.selected));
                SearchKeyResult::Continue
            }
            KeyCode::Up if !self.results.is_empty() => {
                self.selected = self.selected.saturating_sub(1);
                self.list_state.select(Some(self.selected));
                SearchKeyResult::Continue
            }
            KeyCode::Char('j')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !self.results.is_empty() =>
            {
                self.selected = (self.selected + 1).min(self.results.len().saturating_sub(1));
                self.list_state.select(Some(self.selected));
                SearchKeyResult::Continue
            }
            KeyCode::Char('k')
                if key.modifiers.contains(KeyModifiers::CONTROL) && !self.results.is_empty() =>
            {
                self.selected = self.selected.saturating_sub(1);
                self.list_state.select(Some(self.selected));
                SearchKeyResult::Continue
            }
            // Select result
            KeyCode::Enter => {
                if let Some(result) = self.results.get(self.selected) {
                    SearchKeyResult::Selected(result.conversation.session_id.clone())
                } else {
                    SearchKeyResult::Continue
                }
            }
            // Text input
            KeyCode::Char(c) => {
                self.query.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                SearchKeyResult::QueryChanged
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.query.remove(self.cursor_pos);
                    SearchKeyResult::QueryChanged
                } else {
                    SearchKeyResult::Continue
                }
            }
            KeyCode::Delete => {
                if self.cursor_pos < self.query.len() {
                    self.query.remove(self.cursor_pos);
                    SearchKeyResult::QueryChanged
                } else {
                    SearchKeyResult::Continue
                }
            }
            KeyCode::Left => {
                self.cursor_pos = self.cursor_pos.saturating_sub(1);
                SearchKeyResult::Continue
            }
            KeyCode::Right => {
                self.cursor_pos = (self.cursor_pos + 1).min(self.query.len());
                SearchKeyResult::Continue
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                SearchKeyResult::Continue
            }
            KeyCode::End => {
                self.cursor_pos = self.query.len();
                SearchKeyResult::Continue
            }
            _ => SearchKeyResult::Continue,
        }
    }

    /// Navigate to first result
    pub fn jump_to_first(&mut self) {
        if !self.results.is_empty() {
            self.selected = 0;
            self.list_state.select(Some(0));
        }
    }

    /// Navigate to last result
    pub fn jump_to_last(&mut self) {
        if !self.results.is_empty() {
            self.selected = self.results.len() - 1;
            self.list_state.select(Some(self.selected));
        }
    }
}

/// Result of handling a key in the search modal
pub enum SearchKeyResult {
    /// Continue with no action
    Continue,
    /// Query changed, need to re-search
    QueryChanged,
    /// User selected a result (session_id)
    Selected(String),
}

/// Widget for rendering the search modal
pub struct SearchModal<'a> {
    state: &'a mut SearchModalState,
}

impl<'a> SearchModal<'a> {
    pub fn new(state: &'a mut SearchModalState) -> Self {
        Self { state }
    }

    /// Calculate the modal area (centered, 70% width, 60% height)
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

impl Widget for SearchModal<'_> {
    fn render(mut self, area: Rect, buf: &mut Buffer) {
        // Skip rendering if area is too small
        if area.width < 40 || area.height < 10 {
            return;
        }

        // Clear the area first
        Clear.render(area, buf);

        // Main modal block
        let block = Block::default()
            .title(" Search ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        // Layout: filter bar, search input, results, help bar
        let chunks = Layout::vertical([
            Constraint::Length(1), // Filter bar
            Constraint::Length(3), // Search input
            Constraint::Min(3),    // Results
            Constraint::Length(1), // Help bar
        ])
        .split(inner);

        // Render filter bar
        self.render_filter_bar(chunks[0], buf);

        // Render search input
        self.render_search_input(chunks[1], buf);

        // Render results
        self.render_results(chunks[2], buf);

        // Render help bar
        self.render_help_bar(chunks[3], buf);
    }
}

impl SearchModal<'_> {
    fn render_filter_bar(&self, area: Rect, buf: &mut Buffer) {
        let filters = [
            SearchFilterType::All,
            SearchFilterType::Content,
            SearchFilterType::Project,
        ];

        let mut spans = vec![Span::raw(" Filter: ")];
        for (i, filter) in filters.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" | "));
            }
            let style = if *filter == self.state.filter_type {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!(" {} ", filter.display_name()), style));
        }
        spans.push(Span::styled(
            " (Tab to cycle)",
            Style::default().fg(Color::DarkGray),
        ));

        Paragraph::new(Line::from(spans)).render(area, buf);
    }

    fn render_search_input(&self, area: Rect, buf: &mut Buffer) {
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Query ");
        let input_inner = input_block.inner(area);
        input_block.render(area, buf);

        // Render the input text with cursor
        let display_text = &self.state.query;
        let cursor_pos = self.state.cursor_pos;

        // Calculate visible portion if text is too long
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

        Paragraph::new(Line::from(spans)).render(input_inner, buf);
    }

    fn render_results(&mut self, area: Rect, buf: &mut Buffer) {
        if self.state.results.is_empty() {
            let message = if self.state.query.is_empty() {
                "Type to search..."
            } else {
                "No results found"
            };
            let paragraph = Paragraph::new(message)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            paragraph.render(area, buf);
            return;
        }

        // Build list items from results
        let items: Vec<ListItem> = self
            .state
            .results
            .iter()
            .map(|result| {
                let project_name = result
                    .conversation
                    .project_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Unknown".to_string());

                let display = result
                    .conversation
                    .summary
                    .as_ref()
                    .unwrap_or(&result.conversation.display);

                let truncated = truncate_str(display, 57);

                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("[{}] ", project_name),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(truncated),
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

        ratatui::widgets::StatefulWidget::render(list, area, buf, &mut self.state.list_state);
    }

    fn render_help_bar(&self, area: Rect, buf: &mut Buffer) {
        let help_spans = vec![
            Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
            Span::raw("filter "),
            Span::styled(" C-j/k ", Style::default().fg(Color::Cyan)),
            Span::raw("nav "),
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("select "),
            Span::styled(" Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("cancel"),
        ];

        Paragraph::new(Line::from(help_spans))
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .render(area, buf);
    }
}

/// Safely truncate a string to a maximum number of characters (not bytes).
/// Appends "..." if truncated.
fn truncate_str(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars + 3 {
        // Don't truncate if we'd only save a few characters
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}
