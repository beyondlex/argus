use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::util::key_hints;

pub fn render(f: &mut Frame, area: Rect) {
    let popup_area = centered_rect(55, 60, area);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Time Command ")
        .title_alignment(Alignment::Center)
        .style(Style::default().fg(Color::Cyan).bg(Color::Black));

    let lines = vec![
        Line::from(vec![Span::styled(
            "Usage:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::raw(
            "  :Time <duration>              single duration (backward compat)",
        )]),
        Line::from(vec![Span::raw(
            "  :Time <from> to <to>          range with 'to' separator",
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Duration formats:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("  <N>      N hours (e.g. 1, 24)")]),
        Line::from(vec![Span::raw("  <N>h     N hours (e.g. 1h, 12h)")]),
        Line::from(vec![Span::raw("  <N>d     N days (e.g. 7d, 30d)")]),
        Line::from(vec![Span::raw("  <N>w     N weeks (e.g. 1w, 2w)")]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Absolute date formats:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("  MM-DD            date only (default 00:00)")]),
        Line::from(vec![Span::raw("  MM-DD HH:MM      date with time")]),
        Line::from(vec![Span::raw("  HH:MM            time only (right side only)")]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Examples:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("  :time 1w")]),
        Line::from(vec![Span::raw("  :time 1w to 3d")]),
        Line::from(vec![Span::raw("  :time 06-12 12:00 to 13:00")]),
        Line::from(vec![Span::raw("  :time 06-12 to 06-13")]),
        Line::from(vec![Span::raw("  :time 6-12 12:00 to 06-13 12:00")]),
        Line::from(vec![Span::raw("")]),
        Line::from(key_hints(&[("Esc", "Close")])),
    ];

    let text = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(text, popup_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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