use chrono::{DateTime, Utc};

use ratatui::{
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::TreeLine;
use crate::util;

/// Render the metadata panel
pub fn render(
    f: &mut Frame,
    area: Rect,
    selected: Option<&TreeLine>,
    has_delta: bool,
    has_scan: bool,
    last_scan: Option<DateTime<Utc>>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Metadata ")
        .title_alignment(ratatui::layout::Alignment::Left);

    let inner = block.inner(area);
    let content_width = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();

    if let Some(line) = selected {
        let node = &line.node;

        // Path
        push_kv_line(
            &mut lines,
            content_width,
            "Path:",
            node.name().to_string(),
            Style::default().fg(Color::Gray).bold(),
            Style::default().fg(Color::White),
        );
        lines.push(Line::from(vec![Span::raw("")]));

        // Current Size
        let size_display = util::display_size_label(
            node.has_metadata(),
            node.is_dir(),
            line.has_scan_data,
            node.current_size(),
        );
        push_kv_line(
            &mut lines,
            content_width,
            "Size:",
            size_display,
            Style::default().fg(Color::Gray).bold(),
            Style::default().fg(Color::Yellow),
        );

        // Size Delta
        if has_delta {
            // Use the flattened tree overlay delta so metadata matches the tree view.
            let delta = line.delta;
            let delta_str = util::format_delta(delta);
            let delta_color = if delta > 0 {
                Color::Red
            } else if delta < 0 {
                Color::Green
            } else {
                Color::Gray
            };
            push_kv_line(
                &mut lines,
                content_width,
                "Delta:",
                delta_str,
                Style::default().fg(Color::Gray).bold(),
                Style::default().fg(delta_color),
            );
        }

        // Modified Time
        if let Some(modified) = node.modified() {
            push_kv_line(
                &mut lines,
                content_width,
                "Modified:",
                modified.format("%Y-%m-%d %H:%M:%S").to_string(),
                Style::default().fg(Color::Gray).bold(),
                Style::default().fg(Color::White),
            );
        }

        // Created Time
        if let Some(created) = node.created() {
            push_kv_line(
                &mut lines,
                content_width,
                "Created:",
                created.format("%Y-%m-%d %H:%M:%S").to_string(),
                Style::default().fg(Color::Gray).bold(),
                Style::default().fg(Color::White),
            );
        }

        // Type
        let type_str = match node.file_type() {
            argus_core::FileType::Directory => "Directory",
            argus_core::FileType::Symlink => "Symlink",
            argus_core::FileType::Fifo => "FIFO",
            argus_core::FileType::Socket => "Socket",
            argus_core::FileType::Device => "Device",
            argus_core::FileType::Other => "Other",
            argus_core::FileType::File => "File",
        };
        push_kv_line(
            &mut lines,
            content_width,
            "Type:",
            type_str.to_string(),
            Style::default().fg(Color::Gray).bold(),
            Style::default().fg(Color::White),
        );
    } else {
        lines.push(Line::from(vec![Span::styled(
            "No selection",
            Style::default().fg(Color::Gray),
        )]));
    }

    // Scan status
    lines.push(Line::from(vec![Span::raw("")]));
    if has_scan {
        if let Some(ts) = last_scan {
            push_kv_line(
                &mut lines,
                content_width,
                "Scanned:",
                ts.format("%Y-%m-%d %H:%M:%S").to_string(),
                Style::default().fg(Color::Green).bold(),
                Style::default().fg(Color::White),
            );
        }
    } else {
        push_kv_line(
            &mut lines,
            content_width,
            "Scanned:",
            "Press s to scan".to_string(),
            Style::default().fg(Color::Gray).bold(),
            Style::default().fg(Color::DarkGray),
        );
    }

    let text = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(text, area);
}

fn push_kv_line(
    lines: &mut Vec<Line>,
    width: usize,
    label: &str,
    value: String,
    label_style: Style,
    value_style: Style,
) {
    let label_width = label.chars().count();
    let value_width = value.chars().count();
    let padding = width.saturating_sub(label_width + value_width).max(1);

    lines.push(Line::from(vec![
        Span::styled(label.to_string(), label_style),
        Span::raw(" ".repeat(padding)),
        Span::styled(value, value_style),
    ]));
}
