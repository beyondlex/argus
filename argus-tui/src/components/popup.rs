use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};

pub const NORMAL_BORDER_COLOR: Color = Color::White;
pub const NORMAL_TITLE_COLOR: Color = Color::Cyan;
pub const DANGER_BORDER_COLOR: Color = Color::Red;
pub const DANGER_TITLE_COLOR: Color = Color::Red;
pub const POPUP_BG: Color = Color::Black;

#[derive(Clone, Copy)]
pub enum PopupStyle {
    Normal,
    Danger,
}

impl PopupStyle {
    fn border_color(&self) -> Color {
        match self {
            PopupStyle::Normal => NORMAL_BORDER_COLOR,
            PopupStyle::Danger => DANGER_BORDER_COLOR,
        }
    }

    fn title_color(&self) -> Color {
        match self {
            PopupStyle::Normal => NORMAL_TITLE_COLOR,
            PopupStyle::Danger => DANGER_TITLE_COLOR,
        }
    }
}

/// Build a consistently-styled popup block.
/// Callers can chain `.title_bottom(...)` for a footer line.
pub fn popup_block(title: impl Into<Line<'static>>, style: PopupStyle) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(style.border_color()))
        .style(Style::default().bg(POPUP_BG))
        .title_style(Style::default().fg(style.title_color()))
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
