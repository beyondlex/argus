use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};

use crate::theme::ColorTheme;

#[derive(Clone, Copy)]
pub enum PopupStyle {
    Normal,
    Danger,
}

impl PopupStyle {
    fn border_color(&self, theme: &ColorTheme) -> Style {
        match self {
            PopupStyle::Normal => Style::default().fg(theme.popup_border_normal),
            PopupStyle::Danger => Style::default().fg(theme.danger),
        }
    }

    fn title_color(&self, theme: &ColorTheme) -> Style {
        match self {
            PopupStyle::Normal => Style::default().fg(theme.accent),
            PopupStyle::Danger => Style::default().fg(theme.danger),
        }
    }
}

/// Build a consistently-styled popup block.
/// Callers can chain `.title_bottom(...)` for a footer line.
pub fn popup_block(
    title: impl Into<Line<'static>>,
    style: PopupStyle,
    theme: &ColorTheme,
) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(style.border_color(theme))
        .style(Style::default().bg(theme.popup_bg))
        .title_style(style.title_color(theme))
        .title_alignment(Alignment::Center)
        .title(title)
}

/// Center a rect within another rect by percentage of width and height.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}
