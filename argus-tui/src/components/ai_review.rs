use crate::app::App;
use crate::components::popup;
use crate::render::SPINNER_FRAMES;
use crate::theme::ColorTheme;
use crate::types::{AiReviewState, AiStatus, RiskLevel};
use crate::util;
use ratatui::{
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthChar;

const ITEM_LINES: usize = 4;

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
        AiStatus::Loading => format!(
            " AI Analysis [{}] ",
            SPINNER_FRAMES[app.ai_spinner as usize % SPINNER_FRAMES.len()]
        ),
        AiStatus::Ready => " AI Analysis ".to_string(),
        AiStatus::Error(_) => " AI Analysis (error) ".to_string(),
        AiStatus::Idle => " AI Analysis ".to_string(),
    };
    let block = block.title(title);

    let footer_spans = render_footer(state, theme);
    let block = block.title_bottom(Line::from(footer_spans).centered());

    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    let [summary_area, list_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);

    render_summary(f, summary_area, state, app.deleted_bytes, theme);
    render_result_list(f, list_area, state, app);

    if let Some(idx) = state.info_item {
        if let Some(result) = state.results.get(idx) {
            render_info_popup(f, area, state, result, theme);
        }
    }

    if state.delete_confirm.is_some() {
        render_delete_confirm(f, area, state, theme);
    }
}

fn render_label(f: &mut Frame, area: Rect, label: &str, theme: &ColorTheme) {
    let p = Paragraph::new(Line::from(Span::styled(
        label,
        Style::default().fg(theme.text_tertiary),
    )));
    f.render_widget(p, area);
}

fn render_val(f: &mut Frame, area: Rect, val: &str, color: ratatui::style::Color) {
    let p = Paragraph::new(Line::from(Span::styled(val, Style::default().fg(color))))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn char_width(c: char) -> u16 {
    UnicodeWidthChar::width(c).unwrap_or(0) as u16
}

fn text_lines(text: &str, max_width: u16) -> u16 {
    if max_width < 2 {
        return text.len().max(1) as u16;
    }
    let mut col = 0u16;
    let mut lines = 1u16;
    for c in text.chars() {
        let w = char_width(c);
        if col + w > max_width {
            lines += 1;
            col = w;
        } else {
            col += w;
        }
    }
    lines
}

fn render_info_popup(
    f: &mut Frame,
    area: Rect,
    _state: &AiReviewState,
    result: &crate::types::AiPathVerdict,
    theme: &ColorTheme,
) {
    let popup_area = popup::centered_rect(70, 60, area);
    f.render_widget(Clear, popup_area);

    let risk_color = match result.risk_level {
        RiskLevel::Safe => theme.success,
        RiskLevel::Low => theme.warning,
        RiskLevel::Medium => theme.unit_mb,
        RiskLevel::High => theme.danger,
    };

    let block = popup::popup_block(" Item Details ", popup::PopupStyle::Normal, theme)
        .title_bottom(Line::from(util::key_hints(&[("Esc", "Close")], theme)).centered());

    let inner = block.inner(popup_area);
    f.render_widget(Clear, popup_area);
    f.render_widget(block, popup_area);

    let path_str = result.path.to_string_lossy();
    let size_str = util::format_size(result.size);
    let label_w = 13;
    let val_w = inner.width.saturating_sub(label_w).max(1);

    let mut rows = vec![
        Constraint::Length(1),                                     // Path
        Constraint::Length(1),                                     // blank
        Constraint::Length(1),                                     // Label
        Constraint::Length(1),                                     // Size
        Constraint::Length(1),                                     // Risk + Deletable
        Constraint::Length(1),                                     // blank
        Constraint::Length(text_lines(&result.purpose, val_w)),    // Purpose
        Constraint::Length(1),                                     // blank
        Constraint::Length(text_lines(&result.suggestion, val_w)), // Suggestion
    ];
    if !result.background.is_empty() {
        rows.push(Constraint::Length(1)); // blank
        rows.push(Constraint::Length(text_lines(&result.background, val_w))); // Background
    }

    let row_areas = Layout::vertical(rows).split(inner);
    let mut r = 0;

    // Path
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Path:", theme);
    render_val(f, v, &path_str, theme.text);
    r += 1;

    // blank
    r += 1;

    // Label
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Label:", theme);
    render_val(f, v, &result.label, theme.text_highlight);
    r += 1;

    // Size
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Size:", theme);
    render_val(f, v, &size_str, theme.text);
    r += 1;

    // Risk
    let [l, r1, r2] = Layout::horizontal([
        Constraint::Length(label_w),
        Constraint::Length(8),
        Constraint::Min(0),
    ])
    .flex(Flex::Start)
    .areas(row_areas[r]);
    render_label(f, l, "Risk:", theme);
    render_val(f, r1, result.risk_level.label(), risk_color);
    render_val(
        f,
        r2,
        &if result.deletable {
            "Deletable: Yes"
        } else {
            "Deletable: No"
        },
        if result.deletable {
            theme.success
        } else {
            theme.danger
        },
    );
    r += 1;

    // blank
    r += 1;

    // Purpose
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Purpose:", theme);
    render_val(f, v, &result.purpose, theme.text_secondary);
    r += 1;

    // blank
    r += 1;

    // Suggestion
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Suggestion:", theme);
    render_val(f, v, &result.suggestion, theme.text_secondary);
    r += 1;

    // Background
    if !result.background.is_empty() {
        r += 1; // blank
        let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
            .flex(Flex::Start)
            .areas(row_areas[r]);
        render_label(f, l, "Background:", theme);
        render_val(f, v, &result.background, theme.text_secondary);
    }
}

fn render_delete_confirm(f: &mut Frame, area: Rect, state: &AiReviewState, theme: &ColorTheme) {
    let (ref paths, permanent) = state.delete_confirm.as_ref().unwrap();
    let count = paths.len();

    let height_fixed: u16 = 11;
    let popup_area = popup::centered_rect(50, 70, area);
    let height = height_fixed.min(area.height);
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect {
        x: popup_area.x,
        y,
        width: popup_area.width,
        height,
    };

    f.render_widget(Clear, popup);

    let title = format!(" Delete {} items? ", count);
    let action_text = if *permanent {
        "This will permanently delete all selected items."
    } else {
        "This will move all selected items to trash."
    };
    let confirm_label = if *permanent {
        "Permanently delete"
    } else {
        "Confirm delete"
    };

    let block = popup::popup_block(title, popup::PopupStyle::Danger, theme).title_bottom(
        Line::from(util::key_hints(
            &[("y", confirm_label), ("n", "Cancel")],
            theme,
        ))
        .centered(),
    );

    let text = Paragraph::new(vec![
        Line::from(vec![Span::styled(
            "WARNING:",
            Style::default()
                .fg(theme.danger)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            format!("{} items selected for deletion", count),
            Style::default().fg(theme.text),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            action_text,
            Style::default().fg(theme.text),
        )]),
    ])
    .block(block)
    .alignment(Alignment::Center);
    f.render_widget(text, popup);
}

fn render_summary(
    f: &mut Frame,
    area: Rect,
    state: &AiReviewState,
    deleted_bytes: u64,
    theme: &ColorTheme,
) {
    let marked_size: u64 = state
        .mark_for_delete
        .iter()
        .filter_map(|&i| state.results.get(i))
        .map(|r| r.size)
        .sum();

    let count = match state.status {
        AiStatus::Loading | AiStatus::Error(_) => state.pending_paths.len(),
        _ => state.results.len(),
    };

    let left = format!(
        "  Selected {} path(s), total {}",
        count,
        util::format_size(state.pending_total_size)
    );

    let mut right = String::new();
    if !state.mark_for_delete.is_empty() {
        right.push_str(&format!(
            "Marked {} item(s) ({} to free)  ",
            state.mark_for_delete.len(),
            util::format_size(marked_size)
        ));
    }
    if deleted_bytes > 0 {
        right.push_str(&format!("Freed: {}  ", util::format_size(deleted_bytes)));
    }

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

    if state.status == AiStatus::Loading {
        render_pending_list(f, area, state, theme, available);
        return;
    }

    if let AiStatus::Error(ref msg) = state.status {
        render_error(f, area, msg, theme);
        return;
    }

    let mut y_offset = area.y;
    let max_items = (available as usize).saturating_div(ITEM_LINES);
    let end = state.results.len().min(state.scroll_offset + max_items);
    let label_width = (area.width as usize).saturating_sub(14);
    let suggestion_width = (area.width as usize).saturating_sub(16);

    for abs_idx in state.scroll_offset..end {
        let Some(result) = state.results.get(abs_idx) else {
            break;
        };

        let is_cursor = abs_idx == state.cursor;
        let is_marked = state.mark_for_delete.contains(&abs_idx);

        let cursor_arrow = if is_cursor { "▸" } else { " " };
        let mark_char = if is_marked { "●" } else { "○" };
        let prefix_style = if is_cursor {
            Style::default().fg(theme.accent)
        } else if is_marked {
            Style::default().fg(theme.success)
        } else {
            Style::default().fg(theme.text_tertiary)
        };

        let risk_color = match result.risk_level {
            RiskLevel::Safe => theme.success,
            RiskLevel::Low => theme.warning,
            RiskLevel::Medium => theme.unit_mb,
            RiskLevel::High => theme.danger,
        };

        let size_str = util::format_size(result.size);
        let path_str = truncate_path(&result.path.to_string_lossy(), label_width);
        let suggestion_truncated = truncate_str(&result.suggestion, suggestion_width);

        let item_num = abs_idx + 1;

        // Line 1: ▸ ○ N. <path>
        let line1 = Line::from(vec![
            Span::styled(
                format!("{} {} {}. ", cursor_arrow, mark_char, item_num),
                prefix_style,
            ),
            Span::styled(path_str, Style::default().fg(theme.text)),
        ]);
        let line1_area = Rect {
            x: area.x + 1,
            y: y_offset,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        f.render_widget(
            Paragraph::new(line1)
                .block(Block::default().style(Style::default().bg(theme.popup_bg))),
            line1_area,
        );
        y_offset += 1;

        // Line 2: Label: <label>  Size: <size>  Risk: <risk>
        let label_str = truncate_str(&result.label, 28);
        let label_padded = format!("{:<28}", label_str);
        let size_padded = format!("{:>10}", size_str);
        let line2 = Line::from(vec![
            Span::styled("Label: ", Style::default().fg(theme.text_tertiary)),
            Span::styled(
                label_padded,
                Style::default()
                    .fg(theme.text_highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Size: ", Style::default().fg(theme.text_tertiary)),
            Span::styled(size_padded, Style::default().fg(theme.text_secondary)),
            Span::raw("  "),
            Span::styled("Risk: ", Style::default().fg(theme.text_tertiary)),
            Span::styled(
                result.risk_level.label(),
                Style::default().fg(risk_color).add_modifier(Modifier::BOLD),
            ),
        ]);
        let line2_area = Rect {
            x: area.x + 3,
            y: y_offset,
            width: area.width.saturating_sub(4),
            height: 1,
        };
        f.render_widget(
            Paragraph::new(line2)
                .block(Block::default().style(Style::default().bg(theme.popup_bg))),
            line2_area,
        );
        y_offset += 1;

        // Line 3: Suggestion: <suggestion>
        let line3 = Line::from(vec![
            Span::styled("Suggestion: ", Style::default().fg(theme.text_tertiary)),
            Span::styled(
                suggestion_truncated,
                Style::default().fg(theme.text_secondary),
            ),
        ]);
        let line3_area = Rect {
            x: area.x + 3,
            y: y_offset,
            width: area.width.saturating_sub(4),
            height: 1,
        };
        f.render_widget(
            Paragraph::new(line3)
                .block(Block::default().style(Style::default().bg(theme.popup_bg))),
            line3_area,
        );
        y_offset += 1;

        // Line 4: Separator
        let sep_area = Rect {
            x: area.x + 2,
            y: y_offset,
            width: area.width.saturating_sub(4),
            height: 1,
        };
        let sep_line = "─".repeat(sep_area.width as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                sep_line,
                Style::default().fg(theme.text_tertiary).bg(theme.popup_bg),
            ))),
            sep_area,
        );
        y_offset += 1;
    }
}

fn truncate_path(s: &str, max: usize) -> String {
    if max < 5 || s.len() <= max {
        return s.to_string();
    }
    let tail_len = max.saturating_sub(3) / 2;
    let head_len = max.saturating_sub(3).saturating_sub(tail_len);
    let head: String = s.chars().take(head_len).collect();
    let tail: String = s
        .chars()
        .rev()
        .take(tail_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{}...{}", head, tail)
}

fn truncate_str(s: &str, max: usize) -> String {
    if max < 4 {
        return s.chars().take(max).collect();
    }
    if s.len() <= max {
        return s.to_string();
    }
    format!(
        "{}...",
        s.chars().take(max.saturating_sub(3)).collect::<String>()
    )
}

fn render_pending_list(
    f: &mut Frame,
    area: Rect,
    state: &AiReviewState,
    theme: &ColorTheme,
    available: u16,
) {
    let mut y_offset = area.y;
    for (i, path) in state.pending_paths.iter().enumerate() {
        if i >= (available as usize) {
            break;
        }
        let item_area = Rect {
            x: area.x + 2,
            y: y_offset,
            width: area.width.saturating_sub(4),
            height: 1,
        };
        y_offset += 1;

        let path_str = path.to_string_lossy().to_string();
        let text = Paragraph::new(Line::from(Span::styled(
            format!("  {}", path_str),
            Style::default().fg(theme.text_secondary),
        )))
        .block(Block::default().style(Style::default().bg(theme.popup_bg)));
        f.render_widget(text, item_area);
    }
}

fn render_error(f: &mut Frame, area: Rect, msg: &str, theme: &ColorTheme) {
    let text = Paragraph::new(vec![
        Line::from(vec![Span::styled(
            "Analysis failed",
            Style::default()
                .fg(theme.danger)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            msg,
            Style::default().fg(theme.text_secondary),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Press Esc/q to close",
            Style::default().fg(theme.text_tertiary),
        )]),
    ])
    .alignment(Alignment::Center)
    .block(Block::default().style(Style::default().bg(theme.popup_bg)));
    f.render_widget(text, area);
}

fn render_footer(state: &AiReviewState, theme: &ColorTheme) -> Vec<Span<'static>> {
    let label = |s: &str, color| Span::styled(s.to_string(), Style::default().fg(color));

    let mut spans = vec![
        label(" j/k ", theme.accent),
        label("Navigate  ", theme.text_tertiary),
        label(" Space ", theme.accent),
        label("Mark  ", theme.text_tertiary),
        label(" d ", theme.accent),
        label("Delete  ", theme.text_tertiary),
        label(" i ", theme.accent),
        label("Info  ", theme.text_tertiary),
        label(" x ", theme.accent),
        label("Del-Analysis  ", theme.text_tertiary),
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
