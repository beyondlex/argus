use crate::app::App;
use crate::theme::ColorTheme;
use crate::types::{AiReviewState, AiStatus, RiskLevel};
use crate::util;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(ref state) = app.ai_state else {
        return;
    };

    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border_normal))
        .style(Style::default().bg(theme.popup_bg))
        .title_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
        .title_alignment(Alignment::Center);

    let title = match state.status {
        AiStatus::Loading => " AI Analysis (loading...) ",
        AiStatus::Ready => " AI Analysis ",
        AiStatus::Error(_) => " AI Analysis (error) ",
        AiStatus::Idle => " AI Analysis ",
    };
    let block = block.title(title);

    let footer_spans = render_footer(state, theme);
    let block = block.title_bottom(Line::from(footer_spans).centered());

    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    let [summary_area, list_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);

    render_summary(f, summary_area, state, theme);
    render_result_list(f, list_area, state, app);
}

fn render_summary(f: &mut Frame, area: Rect, state: &AiReviewState, theme: &ColorTheme) {
    let total_size: u64 = state.results.iter().map(|r| r.size).sum();
    let marked_size: u64 = state
        .mark_for_delete
        .iter()
        .filter_map(|&i| state.results.get(i))
        .map(|r| r.size)
        .sum();

    let left = format!(
        "  Selected {} path(s), total {}",
        state.results.len(),
        util::format_size(total_size)
    );

    let right = if state.mark_for_delete.is_empty() {
        String::new()
    } else {
        format!(
            "Marked {} item(s) ({} to free)  ",
            state.mark_for_delete.len(),
            util::format_size(marked_size)
        )
    };

    let text = Paragraph::new(Line::from(vec![
        Span::styled(left, Style::default().fg(theme.text_secondary)),
        Span::raw("  "),
        Span::styled(right, Style::default().fg(theme.success)),
    ]));

    f.render_widget(text, area);
}

fn render_result_list(f: &mut Frame, area: Rect, state: &AiReviewState, app: &App) {
    let theme = &app.theme;
    let available = area.height;

    let mut y_offset = area.y;
    for (i, result) in state.results.iter().enumerate() {
        if i >= (available as usize) {
            break;
        }
        let item_area = Rect {
            x: area.x + 1,
            y: y_offset,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        y_offset += 1;

        let is_cursor = i == state.cursor;
        let is_marked = state.mark_for_delete.contains(&i);

        let cursor_arrow = if is_cursor { "▸" } else { " " };
        let prefix = if is_marked { "●" } else { "○" };

        let risk_color = match result.risk_level {
            RiskLevel::Safe => theme.success,
            RiskLevel::Low => theme.warning,
            RiskLevel::Medium => theme.unit_mb,
            RiskLevel::High => theme.danger,
        };

        let size_str = util::format_size(result.size);
        let path_str = result.path.to_string_lossy().to_string();

        let prefix_style = if is_cursor {
            Style::default().fg(theme.accent)
        } else if is_marked {
            Style::default().fg(theme.success)
        } else {
            Style::default().fg(theme.text_tertiary)
        };

        let spans = vec![
            Span::styled(format!("{} {} ", cursor_arrow, prefix), prefix_style),
            Span::styled(
                path_str,
                Style::default().fg(theme.text).bg(theme.popup_bg),
            ),
            Span::raw("  "),
            Span::styled(
                format!(" {} ", result.label),
                Style::default()
                    .fg(theme.text_highlight)
                    .add_modifier(Modifier::BOLD)
                    .bg(theme.popup_bg),
            ),
            Span::styled(
                format!("({})  ", size_str),
                Style::default().fg(theme.text_tertiary).bg(theme.popup_bg),
            ),
            Span::styled(
                result.purpose.clone(),
                Style::default()
                    .fg(theme.text_secondary)
                    .bg(theme.popup_bg),
            ),
        ];

        let block = Block::default().style(Style::default().bg(theme.popup_bg));
        let text = Paragraph::new(Line::from(spans)).block(block);
        f.render_widget(text, item_area);

        // Second line: risk + suggestion
        y_offset += 1;
        if y_offset > area.y + available {
            break;
        }
        let detail_area = Rect {
            x: area.x + 3,
            y: y_offset,
            width: area.width.saturating_sub(4),
            height: 1,
        };
        y_offset += 1;

        let detail_spans = vec![
            Span::styled(
                format!(" [{}] ", result.risk_level.label()),
                Style::default().fg(risk_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                result.suggestion.clone(),
                Style::default().fg(theme.text_tertiary),
            ),
        ];
        let detail_text = Paragraph::new(Line::from(detail_spans))
            .block(Block::default().style(Style::default().bg(theme.popup_bg)));
        f.render_widget(detail_text, detail_area);

        // Separator line after each item
        if y_offset > area.y + available {
            break;
        }
        let sep_area = Rect {
            x: area.x + 2,
            y: y_offset,
            width: area.width.saturating_sub(4),
            height: 1,
        };
        y_offset += 1;

        let sep = Paragraph::new(Line::from(Span::styled(
            "─".repeat(sep_area.width as usize),
            Style::default().fg(theme.text_tertiary).bg(theme.popup_bg),
        )));
        f.render_widget(sep, sep_area);
    }
}

fn render_footer(state: &AiReviewState, theme: &ColorTheme) -> Vec<Span<'static>> {
    let label = |s: &str, color| Span::styled(s.to_string(), Style::default().fg(color));

    let mut spans = vec![
        label(" j/k ", theme.accent),
        label("Navigate  ", theme.text_tertiary),
        label(" Space ", theme.accent),
        label("Mark/Unmark  ", theme.text_tertiary),
        label(" Enter ", theme.accent),
        label("Confirm  ", theme.text_tertiary),
        label(" Esc ", theme.accent),
        label("Close", theme.text_tertiary),
    ];

    if !state.mark_for_delete.is_empty() {
        let marked_size: u64 = state
            .mark_for_delete
            .iter()
            .filter_map(|&i| state.results.get(i))
            .map(|r| r.size)
            .sum();
        spans.push(Span::raw("  "));
        spans.push(label(
            &format!(
                "({} item(s), {} to free)",
                state.mark_for_delete.len(),
                util::format_size(marked_size)
            ),
            theme.success,
        ));
    }

    spans
}
