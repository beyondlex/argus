use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::util::key_hints;

fn section(text: &str) -> Span<'static> {
    Span::styled(
        text.to_string(),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )
}

fn all_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![section("Usage:")]),
        Line::from(vec![Span::raw(
            "  :Time <N>[m|h|d|w]          N minutes/hours/days/weeks ago until now",
        )]),
        Line::from(vec![Span::raw(
            "  :Time HH:MM               from HH:MM today until now",
        )]),
        Line::from(vec![Span::raw(
            "  :Time MM-DD [HH:MM]       from date (optionally time) until now",
        )]),
        Line::from(vec![Span::raw(
            "  :Time <from> to <to>      range with 'to' separator",
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![section("Duration:")]),
        Line::from(vec![Span::raw("  <N>      N hours (e.g. 1, 24)")]),
        Line::from(vec![Span::raw("  <N>h     N hours (e.g. 1h, 12h)")]),
        Line::from(vec![Span::raw("  <N>d     N days (e.g. 7d, 30d)")]),
        Line::from(vec![Span::raw("  <N>w     N weeks (e.g. 1w, 2w)")]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![section("Absolute date:")]),
        Line::from(vec![Span::raw(
            "  MM-DD            date only (default 00:00)",
        )]),
        Line::from(vec![Span::raw("  MM-DD HH:MM      date with time")]),
        Line::from(vec![Span::raw(
            "  HH:MM            time only (inherits date from left, or today)",
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![section("Examples:")]),
        Line::from(vec![Span::raw("  :time 1w")]),
        Line::from(vec![Span::raw("  :time 13:00")]),
        Line::from(vec![Span::raw("  :time 06-12 13:00")]),
        Line::from(vec![Span::raw("  :time 1w to 3d")]),
        Line::from(vec![Span::raw("  :time 06-12 12:00 to 13:00")]),
        Line::from(vec![Span::raw("  :time 06-12 to 06-13")]),
        Line::from(vec![Span::raw("  :time 06-12 12:00 to 06-13 12:00")]),
        Line::from(vec![Span::raw("  :time 12:00 to 13:00")]),
    ]
}

pub fn render(f: &mut Frame, area: Rect, scroll: &mut usize) {
    let popup_area = centered_rect(55, 60, area);

    f.render_widget(Clear, popup_area);

    let lines = all_lines();
    let total = lines.len();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Time Command ")
        .title_alignment(Alignment::Center)
        .style(Style::default().fg(Color::Cyan).bg(Color::Black));

    let inner = block.inner(popup_area);
    let visible_height = inner.height.saturating_sub(1) as usize;

    if total <= visible_height {
        *scroll = 0;
    } else {
        *scroll = (*scroll).min(total.saturating_sub(visible_height));
    }

    let end = (*scroll + visible_height).min(total);
    let visible_lines: Vec<Line> = lines[*scroll..end].to_vec();

    let scroll_indicator = if total > visible_height {
        let pct = if total > 0 {
            (*scroll * 100) / (total.saturating_sub(visible_height))
        } else {
            0
        };
        format!(" [{}%]", pct)
    } else {
        String::new()
    };

    let title = format!(" Time Command{scroll_indicator} ");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_alignment(Alignment::Center)
        .title_bottom(key_hints(&[("j/k", "scroll"), ("Esc", "Close")]))
        .style(Style::default().fg(Color::Cyan).bg(Color::Black));

    let text = Paragraph::new(visible_lines)
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
