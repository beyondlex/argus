use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
    Frame,
};

use crate::components::popup::{popup_block, PopupStyle};
use crate::theme::ColorTheme;
use crate::util::key_hints;

fn section(text: &str, theme: &ColorTheme) -> Span<'static> {
    Span::styled(
        text.to_string(),
        Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
    )
}

fn all_lines(theme: &ColorTheme) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![section("Usage:", theme)]),
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
        Line::from(vec![section("Duration:", theme)]),
        Line::from(vec![Span::raw("  <N>      N hours (e.g. 1, 24)")]),
        Line::from(vec![Span::raw("  <N>h     N hours (e.g. 1h, 12h)")]),
        Line::from(vec![Span::raw("  <N>d     N days (e.g. 7d, 30d)")]),
        Line::from(vec![Span::raw("  <N>w     N weeks (e.g. 1w, 2w)")]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![section("Absolute date:", theme)]),
        Line::from(vec![Span::raw(
            "  MM-DD            date only (default 00:00)",
        )]),
        Line::from(vec![Span::raw("  MM-DD HH:MM      date with time")]),
        Line::from(vec![Span::raw(
            "  HH:MM            time only (inherits date from left, or today)",
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![section("Examples:", theme)]),
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

pub fn render(f: &mut Frame, area: Rect, scroll: &mut usize, theme: &ColorTheme) {
    let popup_area = crate::components::popup::centered_rect(55, 60, area);

    f.render_widget(Clear, popup_area);

    let lines = all_lines(theme);
    let total = lines.len();

    let block = popup_block(" Time Command ", PopupStyle::Normal, theme);

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
    let block = popup_block(title, PopupStyle::Normal, theme)
        .title_bottom(key_hints(&[("j/k", "scroll"), ("Esc", "Close")], theme));

    let text = Paragraph::new(visible_lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(text, popup_area);
}
