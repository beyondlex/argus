use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use ratatui::layout::Rect;

pub fn render(f: &mut Frame, area: Rect, input: &str, matches: &[&str], selected: usize) {
    // Input line at the very bottom of the screen
    let input_y = area.y + area.height - 1;

    // Completion popup above the input line
    if !matches.is_empty() {
        let max_popup = matches.len().min(8) as u16;
        let popup_h = max_popup + 2;
        let popup_y = input_y.saturating_sub(popup_h);
        let popup_area = Rect {
            x: area.x,
            y: popup_y,
            width: area.width.min(40),
            height: popup_h,
        };

        f.render_widget(Clear, popup_area);

        let popup_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(Color::Black));
        let inner = popup_block.inner(popup_area);
        f.render_widget(popup_block, popup_area);

        let mut lines = Vec::new();
        for (i, m) in matches.iter().enumerate().take(8) {
            let style = if i == selected {
                Style::default().fg(Color::Black).bg(Color::LightYellow)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(*m, style)));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }

    // Input line — clear area and render
    let input_area = Rect {
        x: area.x,
        y: input_y,
        width: area.width,
        height: 1,
    };
    f.render_widget(Clear, input_area);

    let input_text = format!(":{}", input);
    let input_style = Style::default()
        .fg(Color::Cyan)
        .bg(Color::Black)
        .add_modifier(ratatui::style::Modifier::BOLD);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(input_text, input_style))),
        input_area,
    );

    // Cursor
    let cursor_x = area.x + (input.len() as u16).min(area.width.saturating_sub(2)).saturating_add(1);
    f.set_cursor_position((cursor_x, input_y));
}