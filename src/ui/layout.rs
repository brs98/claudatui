use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Create the main split-pane layout (25% sidebar, 75% terminal)
#[allow(dead_code)]
pub fn create_layout(area: Rect) -> [Rect; 2] {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    [chunks[0], chunks[1]]
}

/// Create layout with help bar at the bottom
pub fn create_layout_with_help(area: Rect) -> (Rect, Rect, Rect) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let main_area = vertical[0];
    let help_area = vertical[1];

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(main_area);

    (horizontal[0], horizontal[1], help_area)
}
