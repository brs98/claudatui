use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};

/// Terminal pane widget for displaying PTY output
pub struct TerminalPane<'a> {
    parser: Option<&'a vt100::Parser>,
    focused: bool,
}

impl<'a> TerminalPane<'a> {
    pub fn new(parser: Option<&'a vt100::Parser>, focused: bool) -> Self {
        Self { parser, focused }
    }
}

impl<'a> Widget for TerminalPane<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .title(" Claude Code ")
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner_area = block.inner(area);
        block.render(area, buf);

        match self.parser {
            Some(parser) => {
                render_vt100_screen(parser.screen(), inner_area, buf);
            }
            None => {
                // Show placeholder when no PTY is active
                let placeholder = "Press Enter to start Claude Code in selected directory";
                let x = inner_area.x + (inner_area.width.saturating_sub(placeholder.len() as u16)) / 2;
                let y = inner_area.y + inner_area.height / 2;
                if y < inner_area.y + inner_area.height && x < inner_area.x + inner_area.width {
                    buf.set_string(x, y, placeholder, Style::default().fg(Color::DarkGray));
                }
            }
        }
    }
}

fn render_vt100_screen(screen: &vt100::Screen, area: Rect, buf: &mut Buffer) {
    let (rows, cols) = screen.size();

    for row in 0..rows.min(area.height) {
        let y = area.y + row;

        for col in 0..cols.min(area.width) {
            let x = area.x + col;
            let cell = screen.cell(row, col);

            if let Some(cell) = cell {
                let contents = cell.contents();
                if !contents.is_empty() {
                    let style = convert_vt100_style(&cell);
                    buf.set_string(x, y, &contents, style);
                }
            }
        }
    }

    // Render cursor if visible
    let (cursor_row, cursor_col) = screen.cursor_position();
    let cursor_x = area.x + cursor_col;
    let cursor_y = area.y + cursor_row;

    if cursor_y < area.y + area.height && cursor_x < area.x + area.width {
        if let Some(cell) = buf.cell_mut((cursor_x, cursor_y)) {
            cell.set_style(Style::default().bg(Color::White).fg(Color::Black));
        }
    }
}

fn convert_vt100_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();

    // Convert foreground color
    let fg = cell.fgcolor();
    style = style.fg(convert_vt100_color(fg));

    // Convert background color
    let bg = cell.bgcolor();
    style = style.bg(convert_vt100_color(bg));

    // Convert attributes
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }

    style
}

fn convert_vt100_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(0) => Color::Black,
        vt100::Color::Idx(1) => Color::Red,
        vt100::Color::Idx(2) => Color::Green,
        vt100::Color::Idx(3) => Color::Yellow,
        vt100::Color::Idx(4) => Color::Blue,
        vt100::Color::Idx(5) => Color::Magenta,
        vt100::Color::Idx(6) => Color::Cyan,
        vt100::Color::Idx(7) => Color::Gray,
        vt100::Color::Idx(8) => Color::DarkGray,
        vt100::Color::Idx(9) => Color::LightRed,
        vt100::Color::Idx(10) => Color::LightGreen,
        vt100::Color::Idx(11) => Color::LightYellow,
        vt100::Color::Idx(12) => Color::LightBlue,
        vt100::Color::Idx(13) => Color::LightMagenta,
        vt100::Color::Idx(14) => Color::LightCyan,
        vt100::Color::Idx(15) => Color::White,
        vt100::Color::Idx(idx) => Color::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
