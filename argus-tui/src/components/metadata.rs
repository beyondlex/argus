use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use chrono::{DateTime, Utc};

use ratatui::{
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::components::help_popup::centered_rect;
use crate::util;
use crate::util::key_hints;

/// Render a centered popup with file metadata
pub fn render(f: &mut Frame, area: Rect, path: &Path, metadata: &std::fs::Metadata) {
    let popup_area = centered_rect(60, 40, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White))
        .title(" File Info ")
        .title_alignment(ratatui::layout::Alignment::Center);

    let type_str = if metadata.is_dir() {
        "Directory"
    } else if metadata.is_symlink() {
        "Symlink"
    } else if metadata.is_file() {
        "File"
    } else {
        "Other"
    };

    let modified: DateTime<Utc> = metadata
        .modified()
        .ok()
        .map(|t| t.into())
        .unwrap_or_default();
    let created: DateTime<Utc> = metadata
        .created()
        .ok()
        .map(|t| t.into())
        .unwrap_or_default();

    let mode = metadata.permissions().mode();
    let perm_str = unix_mode_string(mode);

    let path_str = path.to_string_lossy();
    let size_str = util::format_size(metadata.len());

    let (num_span, unit_span) = {
        let leading = size_str.len() - size_str.trim_start().len();
        let parts: Vec<&str> = size_str.trim().split_whitespace().collect();
        if parts.len() >= 2 {
            (
                Span::styled(
                    format!("{}{}", &size_str[..leading], parts[0]),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    parts[1],
                    Style::default().fg(util::filesize_unit_color(parts[1])),
                ),
            )
        } else {
            (
                Span::styled(size_str.clone(), Style::default().fg(Color::Gray)),
                Span::raw(""),
            )
        }
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Path:    ", Style::default().fg(Color::Gray).bold()),
            Span::styled(path_str, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Size:    ", Style::default().fg(Color::Gray).bold()),
            num_span,
            Span::styled(" ", Style::default()),
            unit_span,
        ]),
        Line::from(vec![
            Span::styled("Type:    ", Style::default().fg(Color::Gray).bold()),
            Span::styled(type_str, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Modified:", Style::default().fg(Color::Gray).bold()),
            Span::styled(
                format!(" {}", modified.format("%Y-%m-%d %H:%M:%S")),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("Created: ", Style::default().fg(Color::Gray).bold()),
            Span::styled(
                format!(" {}", created.format("%Y-%m-%d %H:%M:%S")),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("Perms:   ", Style::default().fg(Color::Gray).bold()),
            Span::styled(perm_str, Style::default().fg(Color::White)),
        ]),
        Line::from(Span::raw("")),
        Line::from(key_hints(&[("Esc", "Close")])),
    ];

    let text = Paragraph::new(lines).block(block);

    f.render_widget(Clear, popup_area);
    f.render_widget(text, popup_area);
}

fn unix_mode_string(mode: u32) -> String {
    let chars = [
        if mode & 0o400 != 0 { 'r' } else { '-' },
        if mode & 0o200 != 0 { 'w' } else { '-' },
        if mode & 0o100 != 0 { 'x' } else { '-' },
        if mode & 0o040 != 0 { 'r' } else { '-' },
        if mode & 0o020 != 0 { 'w' } else { '-' },
        if mode & 0o010 != 0 { 'x' } else { '-' },
        if mode & 0o004 != 0 { 'r' } else { '-' },
        if mode & 0o002 != 0 { 'w' } else { '-' },
        if mode & 0o001 != 0 { 'x' } else { '-' },
    ];
    chars.iter().collect()
}
