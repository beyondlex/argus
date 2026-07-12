use std::collections::HashMap;
use std::path::Path;

use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::app::{fuzzy_match_indices, SearchMatch, SearchMode, SortMode, TreeLine};
use crate::util;
use crate::util::key_hints;

const SCROLL_MARGIN: usize = 3;

#[allow(clippy::too_many_arguments)]
pub fn render(
    f: &mut Frame,
    area: Rect,
    tree_lines: &[TreeLine],
    filtered_indices: &[usize],
    cursor: usize,
    scroll_offset: usize,
    _sort_mode: SortMode,
    view_root_path: &Path,
    search_word: &str,
    search_mode: SearchMode,
    match_indices: &[SearchMatch],
    current_match: usize,
    cursor_visible: bool,
    focus: bool,
    delta_cache: Option<&HashMap<Vec<String>, i64>>,
) {
    let title = format!("{} ", view_root_path.display());
    let title_style = Style::default().fg(if focus { Color::Magenta } else { Color::Gray });
    let border_style = Style::default().fg(if focus {
        Color::Magenta
    } else {
        Color::DarkGray
    });
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(border_style)
        .title(title)
        .title_style(title_style)
        .title_alignment(ratatui::layout::Alignment::Left);
    let inner = block.inner(area);
    let content_width = inner.width.saturating_sub(1);

    let is_active_match = search_mode == SearchMode::Active && current_match < match_indices.len();
    let available_height = inner.height.saturating_sub(1).max(1) as usize;

    // Use filtered_indices for total count; cursor is index into filtered view
    let total_filtered = filtered_indices.len();

    let scroll_offset = if cursor >= scroll_offset + available_height.saturating_sub(SCROLL_MARGIN)
    {
        cursor
            .saturating_sub(available_height)
            .saturating_add(SCROLL_MARGIN + 1)
    } else if cursor < scroll_offset + SCROLL_MARGIN {
        cursor.saturating_sub(SCROLL_MARGIN)
    } else {
        scroll_offset
    };

    let end = (scroll_offset + available_height).min(total_filtered);
    let visible_indices = &filtered_indices[scroll_offset..end];

    let has_delta = delta_cache.is_some();
    let has_scrollbar = total_filtered > available_height;

    f.render_widget(block, area);

    if has_scrollbar {
        let total_visible = total_filtered.saturating_add(1);
        let mut scrollbar_state =
            ScrollbarState::new(total_visible).position(scroll_offset.saturating_add(1));

        let scrollbar_area = Rect {
            x: inner.right().saturating_sub(1),
            y: inner.y,
            width: 1,
            height: inner.height,
        };
        f.render_stateful_widget(
            Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│")),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }

    let (status_left, status_right) = search_status_line(
        search_mode,
        search_word,
        cursor_visible,
        match_indices,
        filtered_indices.len(),
    );

    let status_area = Rect {
        x: inner.x,
        y: inner.y,
        width: content_width,
        height: 1,
    };

    let status_right_width: u16 = status_right.iter().map(|s| s.content.len() as u16).sum();
    if status_right_width > 0 {
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(status_right_width)])
                .flex(Flex::SpaceBetween)
                .areas(status_area);

        f.render_widget(Paragraph::new(Line::from(status_left)), left_area);
        f.render_widget(Paragraph::new(Line::from(status_right)), right_area);
    } else {
        f.render_widget(Paragraph::new(Line::from(status_left)), status_area);
    }

    for (display_offset, &tree_idx) in visible_indices.iter().enumerate() {
        let Some(line) = tree_lines.get(tree_idx) else {
            continue;
        };
        let global_idx = scroll_offset + display_offset;
        let is_selected = global_idx == cursor;
        let is_current_match =
            is_active_match && match_indices[current_match].tree_idx == Some(tree_idx);

        let delta = delta_cache.and_then(|c| c.get(&line.path).copied());

        let (left_spans, right_spans) = render_tree_line(
            line,
            is_selected,
            is_current_match,
            search_mode != SearchMode::Inactive && !search_word.is_empty(),
            search_word,
            has_delta,
            delta,
        );

        let row_y = inner.y + 1 + display_offset as u16;
        let row_area = Rect {
            x: inner.x,
            y: row_y,
            width: content_width,
            height: 1,
        };

        let right_width: u16 = right_spans.iter().map(|s| s.content.len() as u16).sum();
        let [name_area, info_area] =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(right_width)])
                .flex(Flex::SpaceBetween)
                .areas(row_area);

        f.render_widget(Paragraph::new(Line::from(left_spans)), name_area);
        f.render_widget(Paragraph::new(Line::from(right_spans)), info_area);
    }
}

fn search_status_line<'a>(
    search_mode: SearchMode,
    search_word: &'a str,
    cursor_visible: bool,
    match_indices: &'a [SearchMatch],
    total_visible: usize,
) -> (Vec<Span<'a>>, Vec<Span<'a>>) {
    match search_mode {
        SearchMode::Inactive => (
            vec![Span::styled(
                "  [type / to search]",
                Style::default().fg(Color::DarkGray),
            )],
            vec![],
        ),
        SearchMode::Input => {
            let mut display = search_word.to_string();
            display.push(if cursor_visible { '▎' } else { ' ' });
            let count = format!(" ({}/{})", match_indices.len(), total_visible);
            (
                vec![
                    Span::styled(format!("  {display}"), Style::default().fg(Color::Yellow)),
                    Span::styled(count, Style::default().fg(Color::DarkGray)),
                ],
                vec![],
            )
        }
        SearchMode::Active => {
            let count = format!(" ({}/{}) ", match_indices.len(), total_visible);
            let left = vec![
                Span::styled(
                    format!("  {}", search_word.to_string()),
                    Style::default().fg(Color::Green),
                ),
                Span::styled(count, Style::default().fg(Color::DarkGray)),
            ];
            let right = key_hints(&[
                ("n", "next"),
                ("N", "prev"),
                ("Esc", "clear"),
                ("Enter", "edit"),
            ]);
            (left, right)
        }
    }
}

fn line_indent(depth: usize) -> String {
    if depth == 0 {
        return String::new();
    }
    let mut p = String::new();
    for _ in 0..depth.saturating_sub(1) {
        p.push_str("  ");
    }
    p.push_str("  ");
    p
}

fn branch_marker(line: &TreeLine) -> &'static str {
    if line.node.is_dir() {
        if line.expanded {
            "- "
        } else {
            "+ "
        }
    } else {
        "  "
    }
}

fn is_symlink(line: &TreeLine) -> bool {
    line.node.file_type() == argus_core::FileType::Symlink
}

fn render_tree_line<'a>(
    line: &'a TreeLine,
    is_selected: bool,
    is_current_match: bool,
    has_search: bool,
    search_word: &'a str,
    has_delta: bool,
    delta: Option<i64>,
) -> (Vec<Span<'a>>, Vec<Span<'a>>) {
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
    let base = Style::default().fg(fg).bg(bg);

    let name_prefix = format!("{}{}", line_indent(line.depth), branch_marker(line));

    let size_str = util::display_size_label(
        line.node.is_dir(),
        line.has_scan_data,
        line.node.current_size(),
    );

    let left = name_spans(
        line,
        NameSpanContext {
            name_prefix,
            base,
            bg,
            is_selected,
            is_current_match,
            has_search,
            search_word,
        },
    );

    let mut right = Vec::new();

    if has_delta {
        let delta_str = match delta {
            Some(d) if d > 0 => format!("+{}", util::format_size(d as u64)),
            Some(d) if d < 0 => format!("-{}", util::format_size(d.unsigned_abs())),
            Some(_) => "-".to_string(),
            None => "-".to_string(),
        };
        let delta_style = match delta {
            Some(d) if d > 0 => {
                let unit = util::extract_unit(&delta_str);
                base.fg(util::delta_unit_color(unit)).bg(Color::Reset)
            }
            Some(d) if d < 0 => base.fg(Color::Green).bg(Color::Reset),
            Some(_) => base.fg(Color::DarkGray).bg(Color::Reset),
            None => base.fg(Color::DarkGray).bg(Color::Reset),
        };
        right.push(Span::styled(delta_str, delta_style));
        right.push(Span::raw(" "));
    }

    if line.node.is_dir() && !line.has_scan_data {
        right.push(Span::styled(
            size_str.clone(),
            base.fg(Color::DarkGray).bg(Color::Reset),
        ));
    } else {
        let trimmed = size_str.trim().to_string();
        if let Some(space_idx) = trimmed.rfind(' ') {
            let leading = size_str.len() - size_str.trim_start().len();
            let num = format!("{}{} ", &size_str[..leading], &trimmed[..space_idx]);
            right.push(Span::styled(num, base.fg(Color::Gray)));
            right.push(Span::styled(
                trimmed[space_idx + 1..].to_string(),
                base.fg(util::filesize_unit_color(&trimmed[space_idx + 1..])),
            ));
        } else {
            right.push(Span::styled(size_str.clone(), base.fg(Color::Gray)));
        }
    }

    (left, right)
}

struct NameSpanContext<'a> {
    name_prefix: String,
    base: Style,
    bg: Color,
    is_selected: bool,
    is_current_match: bool,
    has_search: bool,
    search_word: &'a str,
}

fn name_spans<'a>(line: &'a TreeLine, ctx: NameSpanContext<'a>) -> Vec<Span<'a>> {
    let name_text = line.node.name();

    if ctx.has_search && !ctx.search_word.is_empty() {
        let prefix_style = if ctx.is_current_match {
            Style::default().add_modifier(Modifier::BOLD)
        } else if ctx.is_selected {
            Style::default()
                .fg(Color::White)
                .bg(ctx.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut spans = vec![Span::styled(ctx.name_prefix, prefix_style)];

        if let Some(indices) = fuzzy_match_indices(ctx.search_word, name_text) {
            spans.extend(match_highlight_spans(
                name_text,
                &indices,
                ctx.is_current_match,
                ctx.is_selected,
                line.node.is_dir(),
                is_symlink(line),
            ));
        } else {
            let (fg, actual_bg) = if ctx.is_current_match {
                (Color::Green, ctx.bg)
            } else if ctx.is_selected {
                (Color::Black, Color::Blue)
            } else if line.node.is_dir() {
                (Color::Cyan, ctx.bg)
            } else if is_symlink(line) {
                (Color::Magenta, ctx.bg)
            } else {
                (Color::White, ctx.bg)
            };
            spans.push(Span::styled(name_text, ctx.base.fg(fg).bg(actual_bg)));
        }
        spans
    } else {
        let name_style = if ctx.is_current_match {
            Style::default().add_modifier(Modifier::BOLD)
        } else if ctx.is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightGreen)
                .add_modifier(Modifier::BOLD)
        } else if line.node.is_dir() {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if is_symlink(line) {
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        vec![
            Span::styled(ctx.name_prefix, Style::default()),
            Span::styled(name_text.to_string(), name_style),
        ]
    }
}

fn match_highlight_spans<'a>(
    text: &'a str,
    match_indices: &[usize],
    is_current_match: bool,
    is_selected: bool,
    is_dir: bool,
    is_symlink: bool,
) -> Vec<Span<'a>> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut prev_end = 0;
    let match_set: std::collections::HashSet<usize> = match_indices.iter().copied().collect();

    let (matched_fg, matched_bg, normal_fg, normal_bg) = if is_current_match || is_selected {
        (Color::Black, Color::Green, Color::Black, Color::Blue)
    } else {
        let normal_fg = if is_dir {
            Color::Cyan
        } else if is_symlink {
            Color::Magenta
        } else {
            Color::White
        };
        (Color::Green, Color::Black, normal_fg, Color::Black)
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
