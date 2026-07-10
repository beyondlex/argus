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

const SCROLL_MARGIN: usize = 3;

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
    let title = format!(" {} ", view_root_path.display());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_alignment(ratatui::layout::Alignment::Left);
    let inner = block.inner(area);
    let content_width = inner.width.saturating_sub(1);

    let is_active_match = filter_mode == FilterMode::Active && current_match < match_indices.len();
    let available_height = inner.height.saturating_sub(1).max(1) as usize;

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

    let mut rendered_lines: Vec<Line> = Vec::new();
    rendered_lines.push(filter_status_line(
        filter_mode,
        filter_word,
        cursor_visible,
        match_indices,
        lines,
    ));

    let end = (scroll_offset + available_height).min(lines.len());
    let visible_lines = &lines[scroll_offset..end];

    for (display_offset, line) in visible_lines.iter().enumerate() {
        let global_idx = scroll_offset + display_offset;
        let is_selected = global_idx == cursor;
        let is_current_match =
            is_active_match && match_indices[current_match].tree_idx == Some(global_idx);

        rendered_lines.push(render_tree_line(
            line,
            is_selected,
            is_current_match,
            filter_mode != FilterMode::Inactive && !filter_word.is_empty(),
            filter_word,
            content_width,
        ));
    }

    let has_scrollbar = lines.len() > available_height;
    if has_scrollbar {
        let total_visible = lines.len().saturating_add(1);
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

    f.render_widget(Paragraph::new(rendered_lines).block(block), area);
}

fn filter_status_line<'a>(
    filter_mode: FilterMode,
    filter_word: &'a str,
    cursor_visible: bool,
    match_indices: &'a [SearchMatch],
    lines: &'a [TreeLine],
) -> Line<'a> {
    match filter_mode {
        FilterMode::Inactive => Line::from(vec![Span::styled(
            "[type / to filter]",
            Style::default().fg(Color::DarkGray),
        )]),
        FilterMode::Input => {
            let mut display = filter_word.to_string();
            display.push(if cursor_visible { '▎' } else { ' ' });
            let count = format!(" ({}/{})", match_indices.len(), lines.len());
            Line::from(vec![
                Span::styled(display, Style::default().fg(Color::Yellow)),
                Span::styled(count, Style::default().fg(Color::DarkGray)),
            ])
        }
        FilterMode::Active => {
            let count = format!(" ({}/{})", match_indices.len(), lines.len());
            Line::from(vec![
                Span::styled(filter_word.to_string(), Style::default().fg(Color::Green)),
                Span::styled(count, Style::default().fg(Color::DarkGray)),
            ])
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
    has_filter: bool,
    filter_word: &'a str,
    content_width: u16,
) -> Line<'a> {
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
        line.node.has_metadata(),
        line.node.is_dir(),
        line.has_scan_data,
        line.node.current_size(),
    );

    let mut spans = name_spans(
        line,
        NameSpanContext {
            name_prefix,
            base,
            bg,
            is_selected,
            is_current_match,
            has_filter,
            filter_word,
        },
    );

    let size_style = if !line.node.has_metadata() || (line.node.is_dir() && !line.has_scan_data) {
        base.fg(Color::DarkGray).bg(Color::Reset)
    } else {
        base.fg(Color::Yellow).bg(Color::Reset)
    };

    let delta = line.delta;
    let delta_str = if delta != 0 {
        let delta_color = if delta > 0 { Color::Red } else { Color::Green };
        Some((
            util::format_delta(delta),
            base.fg(delta_color).add_modifier(Modifier::BOLD),
        ))
    } else {
        None
    };

    // Calculate visible widths for right-alignment
    let name_width: usize = spans.iter().map(|s| s.content.len()).sum();
    let size_width = size_str.len();
    let delta_width = delta_str.as_ref().map(|(s, _)| s.len()).unwrap_or(0);
    let gap = if delta_str.is_some() { 2 } else { 1 };
    let right_block = delta_width + gap + size_width;
    let pad = (content_width as usize).saturating_sub(name_width + right_block);

    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    } else {
        spans.push(Span::raw(" "));
    }
    if let Some((ds, style)) = delta_str {
        spans.push(Span::styled(ds, style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(size_str, size_style));

    Line::from(spans)
}

struct NameSpanContext<'a> {
    name_prefix: String,
    base: Style,
    bg: Color,
    is_selected: bool,
    is_current_match: bool,
    has_filter: bool,
    filter_word: &'a str,
}

fn name_spans<'a>(line: &'a TreeLine, ctx: NameSpanContext<'a>) -> Vec<Span<'a>> {
    let name_text = line.node.name();

    if ctx.has_filter && !ctx.filter_word.is_empty() {
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

        if let Some(indices) = fuzzy_match_indices(ctx.filter_word, name_text) {
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
