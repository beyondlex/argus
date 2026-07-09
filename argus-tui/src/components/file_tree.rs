use std::path::Path;

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::app::{fuzzy_match_indices, FilterMode, SearchMatch, SortMode, TreeLine};
use crate::util;

#[allow(clippy::too_many_arguments)]
pub fn render(
    f: &mut Frame,
    area: Rect,
    lines: &[TreeLine],
    cursor: usize,
    scroll_offset: usize,
    _sort_mode: SortMode,
    view_root_path: &Path,
    filter_word: &str,
    filter_mode: FilterMode,
    match_indices: &[SearchMatch],
    current_match: usize,
    cursor_visible: bool,
) {
    let path_str = view_root_path.display().to_string();
    let title = format!(" {} ", path_str);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_alignment(ratatui::layout::Alignment::Left);

    let inner = block.inner(area);

    let is_active_current = filter_mode == FilterMode::Active;

    // Status line + scroll accounting
    let available_height = inner.height.saturating_sub(1).max(1) as usize;

    let scroll_offset = if cursor >= scroll_offset + available_height {
        cursor.saturating_sub(available_height).saturating_add(1)
    } else if cursor < scroll_offset {
        cursor
    } else {
        scroll_offset
    };

    let mut rendered_lines: Vec<Line> = Vec::new();

    // ── Filter status line ─────────────────────────────────────────────
    match filter_mode {
        FilterMode::Inactive => {
            rendered_lines.push(Line::from(vec![Span::styled(
                "[type / to filter]",
                Style::default().fg(Color::DarkGray),
            )]));
        }
        FilterMode::Input => {
            let mut display = filter_word.to_string();
            if cursor_visible {
                display.push('▎');
            } else {
                display.push(' ');
            }
            let count_str = format!(" ({}/{})", match_indices.len(), lines.len());
            rendered_lines.push(Line::from(vec![
                Span::styled(display, Style::default().fg(Color::Yellow)),
                Span::styled(count_str, Style::default().fg(Color::DarkGray)),
            ]));
        }
        FilterMode::Active => {
            let count_str = format!(" ({}/{})", match_indices.len(), lines.len());
            rendered_lines.push(Line::from(vec![
                Span::styled(filter_word.to_string(), Style::default().fg(Color::Green)),
                Span::styled(count_str, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    // ── Tree content ───────────────────────────────────────────────────
    let end = (scroll_offset + available_height).min(lines.len());
    let visible_lines = &lines[scroll_offset..end];

    for (display_offset, line) in visible_lines.iter().enumerate() {
        let global_idx = scroll_offset + display_offset;
        let is_selected = global_idx == cursor;
        let is_current_match = is_active_current
            && current_match < match_indices.len()
            && match_indices[current_match].tree_idx == Some(global_idx);

        let prefix = if line.depth == 0 {
            String::new()
        } else {
            let mut p = String::new();
            for _ in 0..line.depth.saturating_sub(1) {
                p.push_str("   ");
            }
            if line.depth > 0 {
                p.push_str("    ");
            }
            p
        };

        let branch = if line.node.is_dir() {
            if line.expanded {
                "- "
            } else {
                "+ "
            }
        } else {
            "  "
        };

        let name_prefix = format!("{}{}", prefix, branch);

        let size_str = if line.node.is_dir() && !line.has_scan_data {
            "-".to_string()
        } else {
            util::format_size(line.node.current_size())
        };
        let delta_str = if line.node.size_delta() != 0 {
            util::format_delta(line.node.size_delta())
        } else {
            String::new()
        };

        // Determine background/foreground
        let bg = if is_current_match {
            Color::Blue
        } else {
            Color::Reset
        };

        let fg = if is_selected {
            Color::Black
        } else {
            Color::White
        };

        let base_style = Style::default().fg(fg).bg(bg);

        // Build name spans (with or without match highlighting)
        let name_spans = if filter_mode != FilterMode::Inactive && !filter_word.is_empty() {
            let name_text = line.node.name();
            let mut spans: Vec<Span> = vec![Span::styled(
                name_prefix,
                if is_current_match {
                    Style::default().add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default()
                        .fg(Color::White)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            )];
            if let Some(indices) = fuzzy_match_indices(filter_word, name_text) {
                spans.extend(match_highlight_spans(
                    name_text,
                    &indices,
                    is_current_match,
                    is_selected,
                ));
            } else {
                let (fg, bg) = if is_current_match {
                    (Color::Green, bg)
                } else if is_selected {
                    (Color::Black, Color::Blue)
                } else {
                    (Color::White, bg)
                };
                spans.push(Span::styled(name_text, base_style.fg(fg).bg(bg)));
            }
            spans
        } else {
            let name_style = if is_current_match {
                Style::default().add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD)
            } else if line.node.is_dir() {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            vec![
                Span::styled(name_prefix.clone(), Style::default()),
                Span::styled(line.node.name().to_string(), name_style),
            ]
        };

        let size_style = if line.node.is_dir() && !line.has_scan_data {
            base_style.fg(Color::DarkGray).bg(Color::Reset)
        } else {
            base_style.fg(Color::Yellow).bg(Color::Reset)
        };

        let mut spans = name_spans;
        spans.push(Span::raw("  "));
        spans.push(Span::styled(size_str, size_style));

        if !delta_str.is_empty() {
            let delta_color = if line.node.size_delta() > 0 {
                Color::Red
            } else {
                Color::Green
            };
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                delta_str,
                base_style.fg(delta_color).add_modifier(Modifier::BOLD),
            ));
        }

        rendered_lines.push(Line::from(spans));
    }

    let total_visible = lines.len().saturating_add(1);
    let mut scrollbar_state =
        ScrollbarState::new(total_visible).position(scroll_offset.saturating_add(1));

    f.render_widget(Paragraph::new(rendered_lines).block(block), area);

    let scrollbar_area = Rect {
        x: area.right().saturating_sub(1),
        y: area.y,
        width: 1,
        height: area.height,
    };
    f.render_stateful_widget(
        Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        scrollbar_area,
        &mut scrollbar_state,
    );
}

fn match_highlight_spans<'a>(
    text: &'a str,
    match_indices: &[usize],
    is_current_match: bool,
    is_selected: bool,
) -> Vec<Span<'a>> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut prev_end = 0;
    let match_set: std::collections::HashSet<usize> = match_indices.iter().copied().collect();

    let (matched_fg, matched_bg, normal_fg, normal_bg) = if is_current_match || is_selected {
        (Color::Black, Color::Green, Color::Black, Color::Blue)
    } else {
        (Color::Green, Color::Black, Color::White, Color::Black)
    };

    for (ci, ch) in chars.iter().enumerate() {
        if match_set.contains(&ci) {
            if ci > prev_end {
                let normal: String = chars[prev_end..ci].iter().collect();
                spans.push(Span::styled(
                    normal,
                    Style::default().fg(normal_fg).bg(normal_bg),
                ));
            }
            spans.push(Span::styled(
                ch.to_string(),
                Style::default()
                    .fg(matched_fg)
                    .bg(matched_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            prev_end = ci + 1;
        }
    }
    if prev_end < chars.len() {
        let rest: String = chars[prev_end..].iter().collect();
        spans.push(Span::styled(
            rest,
            Style::default().fg(normal_fg).bg(normal_bg),
        ));
    }

    spans
}
