use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use chrono::{DateTime, Local, Utc};

use ratatui::{
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Clear, Padding, Paragraph, Wrap},
    Frame,
};

use crate::components::popup::{popup_block, PopupStyle};
use crate::theme::ColorTheme;
use crate::types::{AiPathVerdict, RiskLevel};
use crate::util;
use crate::util::key_hints;

fn cjk_width(c: char) -> u16 {
    if c >= '\u{2E80}' && c <= '\u{9FFF}' { 2 }
    else if c >= '\u{F900}' && c <= '\u{FAFF}' { 2 }
    else if c >= '\u{FE30}' && c <= '\u{FE6F}' { 2 }
    else if c >= '\u{FF00}' && c <= '\u{FFEF}' { 2 }
    else { 1 }
}

fn text_lines(text: &str, max_width: u16) -> u16 {
    if max_width < 2 { return text.len().max(1) as u16; }
    let mut col = 0u16;
    let mut lines = 1u16;
    for c in text.chars() {
        let w = cjk_width(c);
        if col + w > max_width {
            lines += 1;
            col = w;
        } else {
            col += w;
        }
    }
    lines
}

fn render_label(f: &mut Frame, area: Rect, label: &str, theme: &ColorTheme) {
    let p = Paragraph::new(Line::from(Span::styled(
        label,
        Style::default().fg(theme.text_secondary).bold(),
    )));
    f.render_widget(p, area);
}

fn render_val(f: &mut Frame, area: Rect, val: &str, _theme: &ColorTheme, color: ratatui::style::Color) {
    let p = Paragraph::new(Line::from(Span::styled(val, Style::default().fg(color))))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

/// Render a centered popup with file metadata
pub fn render(
    f: &mut Frame,
    area: Rect,
    path: &Path,
    metadata: &std::fs::Metadata,
    ai: Option<&AiPathVerdict>,
    theme: &ColorTheme,
) {
    let height_pct = if ai.is_some() { 60 } else { 40 };
    let popup_area = crate::components::popup::centered_rect(60, height_pct, area);

    let block = popup_block(" File Info ", PopupStyle::Normal, theme)
        .title_bottom(Line::from(key_hints(&[("Esc", "Close")], theme)))
        .title_alignment(Alignment::Right)
        .padding(Padding::horizontal(2));

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

    let path_str = path.to_string_lossy().to_string();
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

    f.render_widget(Clear, popup_area);
    f.render_widget(&block, popup_area);
    let inner = block.inner(popup_area);

    let mut rows = vec![
        Constraint::Length(1), // Path
        Constraint::Length(1), // Size
        Constraint::Length(1), // Type
        Constraint::Length(1), // Modified
        Constraint::Length(1), // Created
        Constraint::Length(1), // Perms
    ];
    if let Some(ref ai) = ai {
        let val_w = inner.width.saturating_sub(13).max(1);
        rows.push(Constraint::Length(1)); // blank
        rows.push(Constraint::Length(1)); // AI header
        rows.push(Constraint::Length(1)); // blank
        rows.push(Constraint::Length(1)); // Label
        rows.push(Constraint::Length(1)); // Risk
        rows.push(Constraint::Length(1)); // Size
        rows.push(Constraint::Length(text_lines(&ai.purpose, val_w))); // Purpose
        rows.push(Constraint::Length(text_lines(&ai.suggestion, val_w))); // Suggestion
        if !ai.background.is_empty() {
            rows.push(Constraint::Length(text_lines(&ai.background, val_w))); // Background
        }
    }

    let row_areas = Layout::vertical(rows).split(inner);
    let label_w = 13;
    let mut r = 0;

    // Path
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Path:", theme);
    render_val(f, v, &path_str, theme, theme.text);
    r += 1;

    // Size
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Size:", theme);
    let size_line = Line::from(vec![num_span, Span::styled(" ", Style::default()), unit_span]);
    f.render_widget(Paragraph::new(size_line), v);
    r += 1;

    // Type
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Type:", theme);
    render_val(f, v, type_str, theme, theme.text);
    r += 1;

    // Modified
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Modified:", theme);
    render_val(f, v, &modified.format("%Y-%m-%d %H:%M:%S").to_string(), theme, theme.text);
    r += 1;

    // Created
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Created:", theme);
    render_val(f, v, &created.format("%Y-%m-%d %H:%M:%S").to_string(), theme, theme.text);
    r += 1;

    // Perms
    let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
        .flex(Flex::Start)
        .areas(row_areas[r]);
    render_label(f, l, "Perms:", theme);
    render_val(f, v, &perm_str, theme, theme.text);
    r += 1;

    if let Some(ai) = ai {
        let risk_color = match ai.risk_level {
            RiskLevel::Safe => theme.success,
            RiskLevel::Low => theme.warning,
            RiskLevel::Medium => theme.unit_mb,
            RiskLevel::High => theme.danger,
        };
        let ai_size = util::format_size(ai.size);

        // blank
        r += 1;

        // AI header
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "── AI Analysis ──",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))),
            row_areas[r],
        );
        r += 1;

        // blank
        r += 1;

        // Label
        let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
            .flex(Flex::Start)
            .areas(row_areas[r]);
        render_label(f, l, "Label:", theme);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &ai.label,
                Style::default()
                    .fg(theme.text_highlight)
                    .add_modifier(Modifier::BOLD),
            ))),
            v,
        );
        r += 1;

        // Risk + Deletable
        let [l, r1, r2] = Layout::horizontal([
            Constraint::Length(label_w),
            Constraint::Length(12),
            Constraint::Min(0),
        ])
        .flex(Flex::Start)
        .areas(row_areas[r]);
        render_label(f, l, "Risk:", theme);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                ai.risk_level.label(),
                Style::default().fg(risk_color).add_modifier(Modifier::BOLD),
            ))),
            r1,
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                if ai.deletable { "Deletable: Yes" } else { "Deletable: No" },
                Style::default().fg(if ai.deletable { theme.success } else { theme.danger }),
            ))),
            r2,
        );
        r += 1;

        // Size
        let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
            .flex(Flex::Start)
            .areas(row_areas[r]);
        render_label(f, l, "Size:", theme);
        render_val(f, v, &ai_size, theme, theme.text);
        r += 1;

        // Purpose
        let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
            .flex(Flex::Start)
            .areas(row_areas[r]);
        render_label(f, l, "Purpose:", theme);
        render_val(f, v, &ai.purpose, theme, theme.text_secondary);
        r += 1;

        // Suggestion
        let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
            .flex(Flex::Start)
            .areas(row_areas[r]);
        render_label(f, l, "Suggestion:", theme);
        render_val(f, v, &ai.suggestion, theme, theme.text_secondary);
        r += 1;

        // Background
        if !ai.background.is_empty() {
            let [l, v] = Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)])
                .flex(Flex::Start)
                .areas(row_areas[r]);
            render_label(f, l, "Background:", theme);
            render_val(f, v, &ai.background, theme, theme.text_secondary);
        }
    }
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