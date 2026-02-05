use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::config::{LayoutConfig, SidebarPosition};

/// Create the main split-pane layout (25% sidebar, 75% terminal)
#[allow(dead_code)]
pub fn create_layout(area: Rect) -> [Rect; 2] {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    [chunks[0], chunks[1]]
}

/// Create layout with help bar at the bottom using config
pub fn create_layout_with_help(area: Rect) -> (Rect, Rect, Rect) {
    // Use default layout config for backwards compatibility
    create_layout_with_help_config(area, &LayoutConfig::default())
}

/// Create layout with help bar at the bottom using provided config
pub fn create_layout_with_help_config(area: Rect, config: &LayoutConfig) -> (Rect, Rect, Rect) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let main_area = vertical[0];
    let help_area = vertical[1];

    // Handle minimized sidebar (3-column width for indicator)
    let sidebar_width = if config.sidebar_minimized {
        3
    } else {
        config.sidebar_width_pct as u16
    };
    let terminal_width = if config.sidebar_minimized {
        100 - 3 // Leave room for minimized indicator
    } else {
        100 - config.sidebar_width_pct as u16
    };

    // Create constraints based on sidebar position
    let constraints = if config.sidebar_minimized {
        // For minimized, use fixed width
        match config.sidebar_position {
            SidebarPosition::Left => [
                Constraint::Length(sidebar_width),
                Constraint::Min(0),
            ],
            SidebarPosition::Right => [
                Constraint::Min(0),
                Constraint::Length(sidebar_width),
            ],
        }
    } else {
        match config.sidebar_position {
            SidebarPosition::Left => [
                Constraint::Percentage(sidebar_width),
                Constraint::Percentage(terminal_width),
            ],
            SidebarPosition::Right => [
                Constraint::Percentage(terminal_width),
                Constraint::Percentage(sidebar_width),
            ],
        }
    };

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(main_area);

    // Return in consistent order: (sidebar, terminal, help)
    match config.sidebar_position {
        SidebarPosition::Left => (horizontal[0], horizontal[1], help_area),
        SidebarPosition::Right => (horizontal[1], horizontal[0], help_area),
    }
}
