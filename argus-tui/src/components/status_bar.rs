use crate::app::{AppMode, ScanSummary, SortMode};
use crate::theme::ColorTheme;
use crate::util;
use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

/// Render the status bar
pub fn render(
    f: &mut Frame,
    area: Rect,
    mode: AppMode,
    scan_summary: Option<&ScanSummary>,
    has_error: Option<&str>,
    status_is_error: bool,
    sort_mode: SortMode,
    multi_select: bool,
    theme: &ColorTheme,
) {
    let mut left_spans: Vec<Span> = Vec::new();

    // Multi-select indicator
    if multi_select {
        left_spans.push(Span::styled(
            " ● MULTI ",
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    }

    if matches!(mode, AppMode::DeletePrompt | AppMode::DeletePermanentPrompt) {
        left_spans.push(Span::styled(
            " DELETE CONFIRM ",
            Style::default()
                .fg(theme.danger)
                .bg(theme.bg)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    } else if matches!(mode, AppMode::Help) {
        left_spans.push(Span::styled(
            " HELP ",
            Style::default().fg(theme.accent).bg(theme.bg),
        ));
    }

    // Daemon status indicator removed — moved to header (top-right)

    if let Some(summary) = scan_summary {
        left_spans.push(Span::raw("   "));
        left_spans.push(Span::styled(
            util::display_path(&summary.root_path),
            Style::default().fg(theme.text_secondary),
        ));
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled(
            "Size:".to_string(),
            Style::default().fg(theme.text_secondary),
        ));
        left_spans.push(Span::styled(
            format!(" {}", util::format_size(summary.total_size)),
            Style::default().fg(theme.text_highlight),
        ));
        left_spans.push(Span::styled(
            " Items:".to_string(),
            Style::default().fg(theme.text_secondary),
        ));
        left_spans.push(Span::styled(
            format!(" {}", util::format_count(summary.total_files)),
            Style::default().fg(theme.text_highlight),
        ));
        left_spans.push(Span::styled(
            " Took:".to_string(),
            Style::default().fg(theme.text_secondary),
        ));
        left_spans.push(Span::styled(
            format!(" {}", util::format_duration(summary.duration)),
            Style::default().fg(theme.text_highlight),
        ));
    } else {
        left_spans.push(Span::raw("   "));
        left_spans.push(Span::styled(
            "Press 's' to scan",
            Style::default().fg(theme.text_tertiary),
        ));
    }

    if let Some(msg) = has_error {
        left_spans.push(Span::raw("   "));
        let fg = if status_is_error {
            theme.danger
        } else {
            theme.success
        };
        left_spans.push(Span::styled(msg, Style::default().fg(fg).bg(theme.bg)));
    }

    // Right side: sort mode indicator
    let right_spans = vec![
        Span::raw(" Sort: "),
        Span::styled(
            sort_mode.label(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    let right_width: u16 = right_spans.iter().map(|s| s.content.len() as u16).sum();
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
