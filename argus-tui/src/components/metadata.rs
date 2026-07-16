use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use chrono::{DateTime, Local, Utc};

use ratatui::{
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use crate::components::popup::{popup_block, PopupStyle};
use crate::theme::ColorTheme;
use crate::util;
use crate::util::key_hints;

/// Render a centered popup with file metadata
pub fn render(
    f: &mut Frame,
    area: Rect,
    path: &Path,
    metadata: &std::fs::Metadata,
    theme: &ColorTheme,
) {
    let popup_area = crate::components::popup::centered_rect(60, 40, area);

    let block = popup_block(" File Info ", PopupStyle::Normal, theme)
        .title_bottom(Line::from(key_hints(&[("Esc", "Close")], theme)))
        .title_alignment(ratatui::layout::Alignment::Right);

    let type_str = if metadata.is_dir() {
        "Directory"
    } else if metadata.is_symlink() {
        "Symlink"
    } else if metadata.is_file() {
        "File"
    } else {
        "Other"
    };

    let modified: DateTime<Local> = metadata
        .modified()
        .ok()
        .map(|t| -> DateTime<Local> {
            let utc: DateTime<Utc> = t.into();
            utc.with_timezone(&Local)
        })
        .unwrap_or_default();
    let created: DateTime<Local> = metadata
        .created()
        .ok()
        .map(|t| -> DateTime<Local> {
            let utc: DateTime<Utc> = t.into();
            utc.with_timezone(&Local)
        })
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
                    Style::default().fg(theme.text_secondary),
                ),
                Span::styled(
                    parts[1],
                    Style::default().fg(util::filesize_unit_color(parts[1], theme)),
                ),
            )
        } else {
            (
                Span::styled(size_str.clone(), Style::default().fg(theme.text_secondary)),
                Span::raw(""),
            )
        }
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                "Path:    ",
                Style::default().fg(theme.text_secondary).bold(),
            ),
            Span::styled(path_str, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled(
                "Size:    ",
                Style::default().fg(theme.text_secondary).bold(),
            ),
            num_span,
            Span::styled(" ", Style::default()),
            unit_span,
        ]),
        Line::from(vec![
            Span::styled(
                "Type:    ",
                Style::default().fg(theme.text_secondary).bold(),
            ),
            Span::styled(type_str, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled(
                "Modified:",
                Style::default().fg(theme.text_secondary).bold(),
            ),
            Span::styled(
                format!(" {}", modified.format("%Y-%m-%d %H:%M:%S")),
                Style::default().fg(theme.text),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Created: ",
                Style::default().fg(theme.text_secondary).bold(),
            ),
            Span::styled(
                format!(" {}", created.format("%Y-%m-%d %H:%M:%S")),
                Style::default().fg(theme.text),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Perms:   ",
                Style::default().fg(theme.text_secondary).bold(),
            ),
            Span::styled(perm_str, Style::default().fg(theme.text)),
        ]),
        Line::from(Span::raw("")),
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
        if mode & 0x020 != 0 { 'w' } else { '-' },
        if mode & 0o010 != 0 { 'x' } else { '-' },
        if mode & 0o004 != 0 { 'r' } else { '-' },
        if mode & 0x002 != 0 { 'w' } else { '-' },
        if mode & 0x001 != 0 { 'x' } else { '-' },
    ];
    chars.iter().collect()
}
