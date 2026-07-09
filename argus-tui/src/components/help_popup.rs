use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

/// Render the help popup overlay
pub fn render(f: &mut Frame, area: Rect) {
    let popup_area = centered_rect(60, 80, area);

    // Clear the area behind the popup
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .title_alignment(Alignment::Center)
        .style(Style::default().fg(Color::White).bg(Color::Black));

    let _inner = block.inner(popup_area);
    let lines = vec![
        Line::from(vec![Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![
            Span::styled("j/k          ", Style::default().fg(Color::Yellow)),
            Span::raw("Move cursor up/down"),
        ]),
        Line::from(vec![
            Span::styled("h            ", Style::default().fg(Color::Yellow)),
            Span::raw("Collapse directory / go to parent"),
        ]),
        Line::from(vec![
            Span::styled("l / Right   ", Style::default().fg(Color::Yellow)),
            Span::raw("Expand directory / enter child"),
        ]),
        Line::from(vec![
            Span::styled("Enter       ", Style::default().fg(Color::Yellow)),
            Span::raw("Edit filter word"),
        ]),
        Line::from(vec![
            Span::styled(".            ", Style::default().fg(Color::Yellow)),
            Span::raw("Set cursor dir as tree root"),
        ]),
        Line::from(vec![
            Span::styled("s            ", Style::default().fg(Color::Yellow)),
            Span::raw("Scan directory"),
        ]),
        Line::from(vec![
            Span::styled("o            ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle sort mode"),
        ]),
        Line::from(vec![
            Span::styled("d            ", Style::default().fg(Color::Yellow)),
            Span::raw("Delete selected item"),
        ]),
        Line::from(vec![
            Span::styled("Tab          ", Style::default().fg(Color::Yellow)),
            Span::raw("Focus next panel"),
        ]),
        Line::from(vec![
            Span::styled("/            ", Style::default().fg(Color::Yellow)),
            Span::raw("Filter items in tree"),
        ]),
        Line::from(vec![
            Span::styled("n/N          ", Style::default().fg(Color::Yellow)),
            Span::raw("Next/prev match (with filter)"),
        ]),
        Line::from(vec![
            Span::styled("?            ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle this help"),
        ]),
        Line::from(vec![
            Span::styled("q / Ctrl+C  ", Style::default().fg(Color::Yellow)),
            Span::raw("Quit"),
        ]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Sort Modes:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("  Name: Alphabetical by name")]),
        Line::from(vec![Span::raw("  Size: By total size (desc)")]),
        Line::from(vec![Span::raw("  Delta: By absolute change (desc)")]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Filter Bar:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw(
            "  Select 'from' and 'to' timestamps to show delta",
        )]),
        Line::from(vec![Span::raw(
            "  Set threshold to filter by minimum change",
        )]),
        Line::from(vec![Span::raw("  Press 'Clear' to reset filters")]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Press ? or Esc to close",
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    let text = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(text, popup_area);
}

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
