use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use crate::app::{AppMode, ScanSummary};
use crate::util;
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
) {
    let mut left_spans: Vec<Span> = Vec::new();

    if matches!(mode, AppMode::DeletePrompt) {
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
        left_spans.push(Span::raw(" | "));
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
        left_spans.push(Span::styled(
            format!("  {}", SPINNER_FRAMES[scan_spinner as usize]),
            Style::default().fg(Color::Yellow),
        ));
        left_spans.push(Span::styled(
            " (press esc cancel)",
            Style::default().fg(Color::DarkGray),
        ));
    } else if let Some(summary) = scan_summary {
        left_spans.push(Span::raw(" | "));
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
        left_spans.push(Span::raw(" | "));
        left_spans.push(Span::styled(
            err,
            Style::default().fg(Color::Red).bg(Color::Black),
        ));
    }

    // Use full width for the single status line
    let left_line = Line::from(left_spans);

    let block = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(Paragraph::new(left_line).block(block), area);
}
