use crate::app::{App, AppMode, SortMode};
use crate::theme::ColorTheme;
use crate::types::DELTA_UNIT_LABELS;
use crate::util;
use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Everything the status bar needs from the app, gathered by the caller.
/// Keeps `render`'s signature manageable as indicators grow.
pub struct StatusCtx<'a> {
    pub mode: AppMode,
    pub has_error: Option<&'a str>,
    pub status_is_error: bool,
    pub sort_mode: SortMode,
    pub multi_select: bool,
    pub multi_select_count: usize,
    pub multi_select_size: u64,
    pub theme: &'a ColorTheme,
    pub time_custom: bool,
    pub time_preset: usize,
    pub time_custom_label: &'a str,
    pub delta_filter_active: bool,
    pub delta_filter_value: u64,
    pub delta_filter_unit: usize,
    pub current_dir_disk_usage: u64,
    pub current_dir_apparent_size: u64,
    pub current_dir_items: u64,
}

/// Render the status bar
pub fn render(f: &mut Frame, area: Rect, ctx: &StatusCtx) {
    let theme = ctx.theme;
    let mut left_spans: Vec<Span> = Vec::new();

    // Multi-select indicator
    if ctx.multi_select {
        left_spans.push(Span::styled(
            format!(
                " ● MULTI({}) {} ",
                ctx.multi_select_count,
                util::format_size(ctx.multi_select_size)
            ),
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    }

    if matches!(
        ctx.mode,
        AppMode::DeletePrompt | AppMode::DeletePermanentPrompt
    ) {
        left_spans.push(Span::styled(
            " DELETE CONFIRM ",
            Style::default()
                .fg(theme.danger)
                .bg(theme.bg)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    } else if matches!(ctx.mode, AppMode::Help) {
        left_spans.push(Span::styled(
            " HELP ",
            Style::default().fg(theme.accent).bg(theme.bg),
        ));
    } else if matches!(ctx.mode, AppMode::AiReview) {
        left_spans.push(Span::styled(
            " AI REVIEW ",
            Style::default()
                .fg(theme.success)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    }

    // Current directory stats (after scan)
    left_spans.push(Span::raw("   "));
    left_spans.push(Span::styled(
        "Disk:",
        Style::default().fg(theme.text_secondary),
    ));
    left_spans.push(Span::styled(
        format!(" {}", util::format_size(ctx.current_dir_disk_usage)),
        Style::default().fg(theme.text_highlight),
    ));
    left_spans.push(Span::styled(
        " Apparent:",
        Style::default().fg(theme.text_secondary),
    ));
    left_spans.push(Span::styled(
        format!(" {}", util::format_size(ctx.current_dir_apparent_size)),
        Style::default().fg(theme.text_highlight),
    ));
    left_spans.push(Span::styled(
        " Items:",
        Style::default().fg(theme.text_secondary),
    ));
    left_spans.push(Span::styled(
        format!(" {}", util::format_count(ctx.current_dir_items)),
        Style::default().fg(theme.text_highlight),
    ));

    if let Some(msg) = ctx.has_error {
        left_spans.push(Span::raw("   "));
        let fg = if ctx.status_is_error {
            theme.danger
        } else {
            theme.success
        };
        left_spans.push(Span::styled(msg, Style::default().fg(fg).bg(theme.bg)));
    }

    let filter_spans = filter_indicator(
        theme,
        ctx.time_custom,
        ctx.time_preset,
        ctx.time_custom_label,
        ctx.delta_filter_active,
        ctx.delta_filter_value,
        ctx.delta_filter_unit,
    );

    let mut right_spans: Vec<Span> = Vec::new();
    if !filter_spans.is_empty() {
        right_spans.extend(filter_spans);
        right_spans.push(Span::raw("   "));
    }
    right_spans.push(Span::raw("Sort: "));
    right_spans.push(Span::styled(
        ctx.sort_mode.label(),
        Style::default()
            .fg(theme.accent)
            .add_modifier(ratatui::style::Modifier::BOLD),
    ));
    right_spans.push(Span::raw(" "));
    let right_width: u16 = right_spans.iter().map(|s| s.content.width() as u16).sum();
    let right_line = Line::from(right_spans);

    let block = Block::default().style(Style::default().bg(theme.bg));
    if right_width + 4 < area.width {
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(right_width)])
                .flex(Flex::SpaceBetween)
                .areas(area);
        f.render_widget(
            Paragraph::new(Line::from(left_spans)).block(block.clone()),
            left_area,
        );
        f.render_widget(Paragraph::new(right_line), right_area);
    } else {
        f.render_widget(Paragraph::new(Line::from(left_spans)).block(block), area);
    }
}

fn filter_indicator(
    theme: &ColorTheme,
    time_custom: bool,
    time_preset: usize,
    time_custom_label: &str,
    delta_active: bool,
    delta_value: u64,
    delta_unit: usize,
) -> Vec<Span<'static>> {
    let data_style = Style::default().fg(theme.accent);
    let normal_style = Style::default().fg(theme.text);

    let val_span = |s: &str| Span::styled(s.to_string(), data_style);
    let lbl_span = |s: &str| Span::styled(s.to_string(), normal_style);

    let mut spans: Vec<Span> = Vec::new();
    let mut has_time = false;
    let mut has_delta = false;

    // Time part
    if time_custom && !time_custom_label.is_empty() {
        has_time = true;
        if time_custom_label.contains('~') {
            // Range: time [07-12 ~ 07-13)
            spans.push(lbl_span("time"));
            spans.push(lbl_span(" ["));
            spans.push(val_span(time_custom_label));
            spans.push(lbl_span(")"));
        } else {
            // Single duration: time in 2h
            spans.push(lbl_span("time"));
            spans.push(lbl_span(" in "));
            spans.push(val_span(time_custom_label));
        }
    } else if !time_custom {
        has_time = true;
        spans.push(lbl_span("time"));
        spans.push(lbl_span(" in "));
        spans.push(val_span(App::time_preset_label(time_preset)));
    }

    // Delta part
    let unit_label = DELTA_UNIT_LABELS.get(delta_unit).copied().unwrap_or("--");

    if delta_active {
        has_delta = true;
        if has_time {
            spans.push(lbl_span(","));
            spans.push(lbl_span(" "));
        }
        spans.push(lbl_span("delta"));
        spans.push(lbl_span(" "));
        spans.push(lbl_span(">="));
        spans.push(lbl_span(" "));
        spans.push(val_span(&delta_value.to_string()));
        spans.push(lbl_span(" "));
        spans.push(val_span(unit_label));
    }

    if !has_time && !has_delta {
        return Vec::new();
    }

    spans
}
