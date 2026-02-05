use crate::ui::toast::{Toast, ToastType};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub struct ToastWidget<'a> {
    toasts: &'a [&'a Toast],
    position: ToastPosition,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ToastPosition {
    #[default]
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
    Center,
}

impl<'a> ToastWidget<'a> {
    pub fn new(toasts: &'a [&'a Toast]) -> Self {
        Self {
            toasts,
            position: ToastPosition::default(),
        }
    }

    pub fn position(mut self, pos: ToastPosition) -> Self {
        self.position = pos;
        self
    }

    pub fn render(self, frame: &mut Frame, area: Rect) {
        if self.toasts.is_empty() {
            return;
        }

        // Calculate toast dimensions
        let toast_width = 32u16;
        let toast_height = 3u16;
        let gap = 1u16;

        for (idx, toast) in self.toasts.iter().enumerate() {
            let toast_area =
                self.calculate_position(area, toast_width, toast_height, idx as u16, gap);

            // Clear background
            frame.render_widget(Clear, toast_area);

            // Render toast
            let border_style = self.get_border_style(toast.toast_type);
            let bg_style = self.get_background_style(toast.toast_type);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .style(bg_style);

            let icon = self.get_icon(toast.toast_type);
            let text = Paragraph::new(Line::from(vec![
                Span::styled(icon, border_style.add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::raw(toast.message.clone()),
            ]))
            .block(block)
            .alignment(Alignment::Left);

            frame.render_widget(text, toast_area);
        }
    }

    fn calculate_position(
        &self,
        area: Rect,
        width: u16,
        height: u16,
        index: u16,
        gap: u16,
    ) -> Rect {
        let offset = index * (height + gap);

        let (x, y) = match self.position {
            ToastPosition::BottomRight => {
                let x = area.right().saturating_sub(width + 2);
                let y = area.bottom().saturating_sub(height + 2 + offset);
                (x, y)
            }
            ToastPosition::BottomLeft => {
                let x = area.left() + 2;
                let y = area.bottom().saturating_sub(height + 2 + offset);
                (x, y)
            }
            ToastPosition::TopRight => {
                let x = area.right().saturating_sub(width + 2);
                let y = area.top() + 2 + offset;
                (x, y)
            }
            ToastPosition::TopLeft => {
                let x = area.left() + 2;
                let y = area.top() + 2 + offset;
                (x, y)
            }
            ToastPosition::Center => {
                let x = area.left() + (area.width.saturating_sub(width)) / 2;
                let y = area.top() + (area.height.saturating_sub(height)) / 2;
                (x, y)
            }
        };

        Rect::new(x, y, width.min(area.width), height.min(area.height))
    }

    fn get_icon(&self, toast_type: ToastType) -> &'static str {
        match toast_type {
            ToastType::Info => "ℹ",
            ToastType::Success => "✓",
            ToastType::Warning => "⚠",
            ToastType::Error => "✗",
        }
    }

    fn get_border_style(&self, toast_type: ToastType) -> Style {
        let color = match toast_type {
            ToastType::Info => Color::Cyan,
            ToastType::Success => Color::Green,
            ToastType::Warning => Color::Yellow,
            ToastType::Error => Color::Red,
        };
        Style::default().fg(color)
    }

    fn get_background_style(&self, toast_type: ToastType) -> Style {
        let color = match toast_type {
            ToastType::Info => Color::Black,
            ToastType::Success => Color::Black,
            ToastType::Warning => Color::Black,
            ToastType::Error => Color::Black,
        };
        Style::default().bg(color)
    }
}
