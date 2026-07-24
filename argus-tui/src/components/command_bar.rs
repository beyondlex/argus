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
    scroll: usize,
    theme: &ColorTheme,
) {
    let input_y = area.y + area.height - 1;

    if !matches.is_empty() {
        let max_popup = 8u16;
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
        let visible = max_popup as usize;
        let total = matches.len();
        let scroll = scroll.min(total.saturating_sub(visible));
        for (i, m) in matches.iter().enumerate().skip(scroll).take(visible) {
            let style = if i == selected {
                Style::default().fg(theme.focus_fg).bg(theme.focus_bg)
            } else {
                Style::default().fg(theme.text).bg(theme.popup_bg)
            };
            let marker = if i == selected { ">" } else { " " };
            lines.push(Line::from(Span::styled(
                format!("{}{:<width$}", marker, m, width = inner_w - 1),
                style,
            )));
        }
        if total > visible {
            let at_top = scroll == 0;
            let at_bot = scroll + visible >= total;
            let scrollbar = if at_top {
                " ↓"
            } else if at_bot {
                " ↑"
            } else {
                " ↕"
            };
            let pct = if total > 0 {
                (scroll + visible) * 100 / total
            } else {
                100
            };
            lines.push(Line::from(Span::styled(
                format!("{scrollbar} {pct}%"),
                Style::default().fg(theme.text_tertiary),
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
