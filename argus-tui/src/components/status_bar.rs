use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use crate::app::{AppMode, Focus};

const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Render the status bar
pub fn render(
    f: &mut Frame,
    area: Rect,
    mode: AppMode,
    focus: Focus,
    file_count: usize,
    scanning: bool,
    scan_progress: Option<(u64, u64)>,
    scan_spinner: u8,
    has_error: Option<&str>,
) {
    let mut left_spans: Vec<Span> = Vec::new();

    match mode {
        AppMode::Browsing => {
            left_spans.push(Span::styled(
                " Browsing ",
                Style::default().fg(Color::Green).bg(Color::Black),
            ));
        }
        AppMode::DeletePrompt => {
            left_spans.push(Span::styled(
                " DELETE CONFIRM ",
                Style::default()
                    .fg(Color::Red)
                    .bg(Color::Black)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ));
        }
        AppMode::Help => {
            left_spans.push(Span::styled(
                " HELP ",
                Style::default().fg(Color::Cyan).bg(Color::Black),
            ));
        }
    }

    left_spans.push(Span::raw(format!(" | files: {}", file_count)));

    if scanning {
        let spinner = SPINNER_FRAMES[scan_spinner as usize];
        left_spans.push(Span::raw(" | "));
        left_spans.push(Span::styled(
            format!("{} scanning", spinner),
            Style::default().fg(Color::Yellow),
        ));
        if let Some((current, _total)) = scan_progress {
            left_spans.push(Span::styled(
                format!(" {} files", current),
                Style::default().fg(Color::Yellow),
            ));
        }
    }

    if let Some(err) = has_error {
        left_spans.push(Span::raw(" | "));
        left_spans.push(Span::styled(
            err,
            Style::default().fg(Color::Red).bg(Color::Black),
        ));
    }

    // Focus indicator
    let focus_str = match focus {
        Focus::Tree => "[Tree]",
        Focus::FilterBar => "[Filter]",
    };
    left_spans.push(Span::raw(" | "));
    left_spans.push(Span::styled(focus_str, Style::default().fg(Color::Cyan)));

    let right_spans = vec![
        Span::styled(" [?] Help ", Style::default().fg(Color::DarkGray)),
        Span::styled(" [Q] Quit ", Style::default().fg(Color::DarkGray)),
    ];

    // Use full width, left and right aligned
    let left_line = Line::from(left_spans);
    let right_line = Line::from(right_spans);

    let block = Block::default().style(Style::default().bg(Color::Black));
    let inner = block.inner(area);

    // Render left part
    f.render_widget(
        Paragraph::new(left_line).block(Block::default()),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width.saturating_sub(20),
            height: inner.height,
        },
    );

    // Render right part
    f.render_widget(
        Paragraph::new(right_line)
            .block(Block::default())
            .right_aligned(),
        Rect {
            x: inner.x.saturating_add(inner.width.saturating_sub(20)),
            y: inner.y,
            width: 20,
            height: inner.height,
        },
    );
}
