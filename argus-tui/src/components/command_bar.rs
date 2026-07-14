use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use ratatui::layout::Rect;

use crate::theme::ColorTheme;

pub fn render(
    f: &mut Frame,
    area: Rect,
    input: &str,
    matches: &[&str],
    selected: usize,
    theme: &ColorTheme,
) {
    let input_y = area.y + area.height - 1;

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
            .border_style(Style::default().fg(theme.border_unfocused))
            .style(Style::default().bg(theme.popup_bg));
        let inner = popup_block.inner(popup_area);
        f.render_widget(popup_block, popup_area);

        let inner_w = inner.width as usize;
        let mut lines = Vec::new();
        for (i, m) in matches.iter().enumerate().take(8) {
            let style = if i == selected {
                Style::default().fg(theme.focus_fg).bg(theme.focus_bg)
            } else {
                Style::default().fg(theme.text).bg(theme.popup_bg)
            };
            lines.push(Line::from(Span::styled(
                format!("{:<width$}", m, width = inner_w),
                style,
            )));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }

    let input_area = Rect {
        x: area.x,
        y: input_y,
        width: area.width,
        height: 1,
    };
    f.render_widget(Clear, input_area);

    let input_text = format!(":{:<width$}", input, width = area.width as usize - 1);
    let input_style = Style::default()
        .fg(theme.accent)
        .bg(theme.bg)
        .add_modifier(ratatui::style::Modifier::BOLD);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(input_text, input_style))),
        input_area,
    );

    let cursor_x = area.x
        + (input.len() as u16)
            .min(area.width.saturating_sub(2))
            .saturating_add(1);
    f.set_cursor_position((cursor_x, input_y));
}
