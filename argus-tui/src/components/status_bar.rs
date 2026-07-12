use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use crate::app::{AppMode, ScanSummary, SortMode};
use crate::util;
use crate::util::key_hints;
use std::path::Path;
use std::time::Duration;

const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Render the status bar
pub fn render(
    f: &mut Frame,
    area: Rect,
    mode: AppMode,
    view_root_path: &Path,
    scanning: bool,
    scan_progress: Option<(u64, u64)>,
    scan_spinner: u8,
    scan_elapsed: Option<Duration>,
    scan_summary: Option<&ScanSummary>,
    has_error: Option<&str>,
    server_connected: bool,
    sort_mode: SortMode,
) {
    let mut left_spans: Vec<Span> = Vec::new();

    // Daemon status indicator
    let (status_text, status_color) = if server_connected {
        (" ● Daemon", Color::Green)
    } else {
        (" ○ Daemon", Color::DarkGray)
    };
    left_spans.push(Span::raw("   "));
    left_spans.push(Span::styled(status_text, Style::default().fg(status_color)));

    if matches!(mode, AppMode::DeletePrompt | AppMode::DeletePermanentPrompt) {
        left_spans.push(Span::styled(
            " DELETE CONFIRM ",
            Style::default()
                .fg(Color::Red)
                .bg(Color::Black)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    } else if matches!(mode, AppMode::Help) {
        left_spans.push(Span::styled(
            " HELP ",
            Style::default().fg(Color::Cyan).bg(Color::Black),
        ));
    }

    if scanning {
        left_spans.push(Span::styled(
            format!("  {}", SPINNER_FRAMES[scan_spinner as usize]),
            Style::default().fg(Color::Yellow),
        ));
        left_spans.push(Span::styled(
            util::display_path(view_root_path),
            Style::default().fg(Color::Gray),
        ));
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled("Size:", Style::default().fg(Color::Gray)));
        if let Some((current, total_bytes)) = scan_progress {
            left_spans.push(Span::styled(
                format!(" {}", util::format_size(total_bytes)),
                Style::default().fg(Color::Yellow),
            ));
            left_spans.push(Span::raw("  "));
            left_spans.push(Span::styled("Items:", Style::default().fg(Color::Gray)));
            left_spans.push(Span::styled(
                format!(" {}", util::format_count(current)),
                Style::default().fg(Color::Yellow),
            ));
        }
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled("Took:", Style::default().fg(Color::Gray)));
        left_spans.push(Span::styled(
            format!(
                " {}",
                util::format_duration(scan_elapsed.unwrap_or_default())
            ),
            Style::default().fg(Color::Yellow),
        ));
        left_spans.extend(key_hints(&[("Esc", "cancel")]));
    } else if let Some(summary) = scan_summary {
        left_spans.push(Span::raw("   "));
        left_spans.push(Span::styled(
            util::display_path(&summary.root_path),
            Style::default().fg(Color::Gray),
        ));
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled(
            "Size:".to_string(),
            Style::default().fg(Color::Gray),
        ));
        left_spans.push(Span::styled(
            format!(" {}", util::format_size(summary.total_size)),
            Style::default().fg(Color::Yellow),
        ));
        left_spans.push(Span::styled(
            " Items:".to_string(),
            Style::default().fg(Color::Gray),
        ));
        left_spans.push(Span::styled(
            format!(" {}", util::format_count(summary.total_files)),
            Style::default().fg(Color::Yellow),
        ));
        left_spans.push(Span::styled(
            " Took:".to_string(),
            Style::default().fg(Color::Gray),
        ));
        left_spans.push(Span::styled(
            format!(" {}", util::format_duration(summary.duration)),
            Style::default().fg(Color::Yellow),
        ));
    }

    if let Some(err) = has_error {
        left_spans.push(Span::raw("   "));
        left_spans.push(Span::styled(
            err,
            Style::default().fg(Color::Red).bg(Color::Black),
        ));
    }

    // Right side: sort mode indicator
    let right_spans = vec![
        Span::raw(" Sort: "),
        Span::styled(
            sort_mode.label(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    let right_width: u16 = right_spans.iter().map(|s| s.content.len() as u16).sum();
    let right_line = Line::from(right_spans);

    let block = Block::default().style(Style::default().bg(Color::Black));
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
