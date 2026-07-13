use std::path::Path;
use std::{collections::HashMap, str::FromStr};

use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::app::{SearchMatch, SearchMode, TreeLine};
use crate::search::fuzzy_match_indices;
use crate::util;
use crate::util::key_hints;

const SCROLL_MARGIN: usize = 3;

pub struct TreeRenderCtx<'a> {
    pub tree_lines: &'a [TreeLine],
    pub filtered_indices: &'a [usize],
    pub cursor: usize,
    pub scroll_offset: usize,
    pub view_root_path: &'a Path,
    pub search_word: &'a str,
    pub search_mode: SearchMode,
    pub match_indices: &'a [SearchMatch],
    pub current_match: usize,
    pub cursor_visible: bool,
    pub focus: bool,
    pub delta_cache: Option<&'a HashMap<Vec<String>, i64>>,
    pub root_total_size: u64,
}

pub fn render(f: &mut Frame, area: Rect, ctx: TreeRenderCtx) {
    let title = format!("{} ", ctx.view_root_path.display());
    let title_style = Style::default().fg(if ctx.focus {
        Color::Magenta
    } else {
        Color::Gray
    });
    let border_style = Style::default().fg(if ctx.focus {
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

    let is_active_match =
        ctx.search_mode == SearchMode::Active && ctx.current_match < ctx.match_indices.len();
    let available_height = inner.height.saturating_sub(1).max(1) as usize;

    let total_filtered = ctx.filtered_indices.len();

    let scroll_offset =
        if ctx.cursor >= ctx.scroll_offset + available_height.saturating_sub(SCROLL_MARGIN) {
            ctx.cursor
                .saturating_sub(available_height)
                .saturating_add(SCROLL_MARGIN + 1)
        } else if ctx.cursor < ctx.scroll_offset + SCROLL_MARGIN {
            ctx.cursor.saturating_sub(SCROLL_MARGIN)
        } else {
            ctx.scroll_offset
        };

    let end = (scroll_offset + available_height).min(total_filtered);
    let visible_indices = &ctx.filtered_indices[scroll_offset..end];

    let has_delta = ctx.delta_cache.is_some();
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
        ctx.search_mode,
        ctx.search_word,
        ctx.cursor_visible,
        ctx.match_indices,
        ctx.filtered_indices.len(),
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
        let Some(line) = ctx.tree_lines.get(tree_idx) else {
            continue;
        };
        let global_idx = scroll_offset + display_offset;
        let is_selected = global_idx == ctx.cursor;
        let is_current_match =
            is_active_match && ctx.match_indices[ctx.current_match].tree_idx == Some(tree_idx);

        let row_bg = if is_current_match {
            Color::Blue
        } else if is_selected {
            Color::from_str("#2c284b").unwrap_or_else(|_| Color::Green)
        } else {
            Color::Reset
        };

        let delta = ctx.delta_cache.and_then(|c| c.get(&line.path).copied());

        let (left_spans, right_spans) = render_tree_line(
            line,
            is_selected,
            is_current_match,
            ctx.search_mode != SearchMode::Inactive && !ctx.search_word.is_empty(),
            ctx.search_word,
            has_delta,
            delta,
            row_bg,
            ctx.root_total_size,
        );

        let row_y = inner.y + 1 + display_offset as u16;
        let row_area = Rect {
            x: inner.x,
            y: row_y,
            width: content_width,
            height: 1,
        };

        if row_bg != Color::Reset {
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    " ".repeat(content_width as usize),
                    Style::default().bg(row_bg),
                )])),
                row_area,
            );
        }

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

#[allow(clippy::too_many_arguments)]
fn render_tree_line<'a>(
    line: &'a TreeLine,
    is_selected: bool,
    is_current_match: bool,
    has_search: bool,
    search_word: &'a str,
    has_delta: bool,
    delta: Option<i64>,
    row_bg: Color,
    root_total_size: u64,
) -> (Vec<Span<'a>>, Vec<Span<'a>>) {
    let fg = if is_selected {
        Color::Black
    } else {
        Color::White
    };
    let base = Style::default().fg(fg).bg(row_bg);
    let row_hl = row_bg != Color::Reset;

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
            bg: row_bg,
            is_selected,
            is_current_match,
            has_search,
            search_word,
        },
    );

    let mut right = Vec::new();

    if has_delta {
        let (delta_str, delta_style) = match delta {
            Some(d) if d > 0 => {
                let s = format!("+{}", util::format_size(d as u64));
                let color = util::delta_unit_color(util::extract_unit(&s));
                (s, base.fg(if row_hl { Color::White } else { color }))
            }
            Some(d) if d < 0 => (
                format!("-{}", util::format_size(d.unsigned_abs())),
                base.fg(if row_hl { Color::White } else { Color::Green }),
            ),
            Some(_) | None => (
                "-".to_string(),
                base.fg(if row_hl {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ),
        };
        right.push(Span::styled(delta_str, delta_style));
        right.push(Span::raw(" "));
    }

    if line.node.is_dir() && !line.has_scan_data {
        right.push(Span::styled(
            size_str.clone(),
            base.fg(if row_hl {
                Color::White
            } else {
                Color::DarkGray
            }),
        ));
    } else {
        let trimmed = size_str.trim().to_string();
        if let Some(space_idx) = trimmed.rfind(' ') {
            let leading = size_str.len() - size_str.trim_start().len();
            let num = format!("{}{} ", &size_str[..leading], &trimmed[..space_idx]);
            right.push(Span::styled(
                num,
                base.fg(if row_hl { Color::White } else { Color::Gray }),
            ));
            right.push(Span::styled(
                trimmed[space_idx + 1..].to_string(),
                base.fg(if row_hl {
                    Color::White
                } else {
                    util::filesize_unit_color(&trimmed[space_idx + 1..])
                }),
            ));
        } else {
            right.push(Span::styled(
                size_str.clone(),
                base.fg(if row_hl { Color::White } else { Color::Gray }),
            ));
        }
    }

    if root_total_size > 0 && line.has_scan_data {
        let pct = (line.node.current_size() as f64 / root_total_size as f64) * 100.0;
        right.push(Span::styled(
            format!("{:>6.1}%", pct),
            base.fg(if row_hl {
                Color::White
            } else {
                Color::DarkGray
            }),
        ));
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
            Style::default().bg(ctx.bg).add_modifier(Modifier::BOLD)
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
                ctx.bg,
            ));
        } else {
            let (fg, actual_bg) = if ctx.is_current_match {
                (Color::Green, ctx.bg)
            } else if ctx.is_selected {
                (Color::Black, ctx.bg)
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
            Style::default().bg(ctx.bg).add_modifier(Modifier::BOLD)
        } else if ctx.is_selected {
            Style::default()
                .fg(Color::White)
                .bg(ctx.bg)
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
            Span::styled(ctx.name_prefix, Style::default().bg(ctx.bg)),
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
    row_bg: Color,
) -> Vec<Span<'a>> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut prev_end = 0;
    let match_set: std::collections::HashSet<usize> = match_indices.iter().copied().collect();

    let (matched_fg, matched_bg, normal_fg, normal_bg) = if is_current_match || is_selected {
        (Color::Black, Color::Green, Color::Black, row_bg)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{SearchMatch, TreeNode};
    use argus_core::FileType;
    use std::sync::Arc;

    fn make_file_node(name: &str, is_dir: bool, file_type: FileType) -> argus_core::FileNode {
        argus_core::FileNode {
            name: name.into(),
            parent: None,
            is_dir,
            file_type,
            size: 1024,
            children: vec![],
        }
    }

    fn make_treeline(
        name: &str,
        depth: usize,
        is_dir: bool,
        file_type: FileType,
        expanded: bool,
        has_scan_data: bool,
    ) -> TreeLine {
        let arena = vec![make_file_node(name, is_dir, file_type)];
        let snap = Arc::new(argus_core::Snapshot::new(
            std::path::PathBuf::from("/"),
            arena,
            1024,
        ));
        TreeLine {
            depth,
            node: TreeNode::Snapshot(snap, 0),
            expanded,
            has_scan_data,
            path: vec![name.into()],
        }
    }

    #[test]
    fn test_line_indent_zero() {
        assert_eq!(line_indent(0), "");
    }

    #[test]
    fn test_line_indent_one() {
        assert_eq!(line_indent(1), "  ");
    }

    #[test]
    fn test_line_indent_two() {
        assert_eq!(line_indent(2), "    ");
    }

    #[test]
    fn test_line_indent_deep() {
        assert_eq!(line_indent(3), "      ");
    }

    #[test]
    fn test_branch_marker_dir_expanded() {
        let line = make_treeline("d", 0, true, FileType::Directory, true, false);
        assert_eq!(branch_marker(&line), "- ");
    }

    #[test]
    fn test_branch_marker_dir_collapsed() {
        let line = make_treeline("d", 0, true, FileType::Directory, false, false);
        assert_eq!(branch_marker(&line), "+ ");
    }

    #[test]
    fn test_branch_marker_file() {
        let line = make_treeline("f", 0, false, FileType::File, false, false);
        assert_eq!(branch_marker(&line), "  ");
    }

    #[test]
    fn test_is_symlink_true() {
        let line = make_treeline("link", 0, false, FileType::Symlink, false, false);
        assert!(is_symlink(&line));
    }

    #[test]
    fn test_is_symlink_false() {
        let line = make_treeline("f", 0, false, FileType::File, false, false);
        assert!(!is_symlink(&line));
    }

    #[test]
    fn test_search_status_inactive() {
        let (left, right) = search_status_line(SearchMode::Inactive, "", true, &[], 10);
        assert!(right.is_empty());
        assert!(left[0].content.contains("type / to search"));
    }

    #[test]
    fn test_search_status_input_cursor_visible() {
        let (left, right) = search_status_line(SearchMode::Input, "hello", true, &[], 10);
        assert!(right.is_empty());
        assert!(left[0].content.contains("hello"));
    }

    #[test]
    fn test_search_status_active_shows_hints() {
        let matches = vec![SearchMatch {
            path: vec!["a".into()],
            tree_idx: Some(0),
            walk_idx: 0,
        }];
        let (left, right) = search_status_line(SearchMode::Active, "hello", true, &matches, 10);
        assert!(!right.is_empty());
        assert!(left[0].content.contains("hello"));
    }

    #[test]
    fn test_match_highlight_spans_no_matches() {
        let spans = match_highlight_spans("hello", &[], false, false, false, false, Color::Reset);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "hello");
    }

    #[test]
    fn test_match_highlight_spans_some_matches() {
        let spans =
            match_highlight_spans("hello", &[0, 1], false, false, false, false, Color::Reset);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "h");
        assert_eq!(spans[1].content, "e");
        assert_eq!(spans[2].content, "llo");
    }

    #[test]
    fn test_render_tree_line_normal() {
        let line = make_treeline("test.txt", 0, false, FileType::File, false, true);
        let (left, right) =
            render_tree_line(&line, false, false, false, "", false, None, Color::Reset, 0);
        assert!(!left.is_empty());
        assert!(!right.is_empty());
    }

    #[test]
    fn test_render_tree_line_selected() {
        let line = make_treeline("test.txt", 0, false, FileType::File, false, true);
        let (left, _) =
            render_tree_line(&line, true, false, false, "", false, None, Color::Green, 0);
        assert!(!left.is_empty());
    }

    #[test]
    fn test_render_tree_line_with_delta_positive() {
        let line = make_treeline("f", 0, false, FileType::File, false, true);
        let (_, right) = render_tree_line(
            &line,
            false,
            false,
            false,
            "",
            true,
            Some(2048),
            Color::Reset,
            0,
        );
        let right_text: String = right.iter().map(|s| s.content.as_ref()).collect();
        assert!(right_text.contains('+'));
    }

    #[test]
    fn test_render_tree_line_with_delta_negative() {
        let line = make_treeline("f", 0, false, FileType::File, false, true);
        let (_, right) = render_tree_line(
            &line,
            false,
            false,
            false,
            "",
            true,
            Some(-512),
            Color::Reset,
            0,
        );
        let right_text: String = right.iter().map(|s| s.content.as_ref()).collect();
        assert!(right_text.contains('-'));
    }

    #[test]
    fn test_render_tree_line_with_search() {
        let line = make_treeline("hello.txt", 0, false, FileType::File, false, true);
        let (left, _) = render_tree_line(
            &line,
            false,
            false,
            true,
            "hello",
            false,
            None,
            Color::Reset,
            0,
        );
        let left_text: String = left.iter().map(|s| s.content.as_ref()).collect();
        assert!(left_text.contains("hello"));
    }

    #[test]
    fn test_render_tree_line_percentage_shown() {
        let line = make_treeline("f", 0, false, FileType::File, false, true);
        let (_, right) = render_tree_line(
            &line,
            false,
            false,
            false,
            "",
            false,
            None,
            Color::Reset,
            2048, // root_total_size = 2048, file size = 1024 (from make_file_node)
        );
        let right_text: String = right.iter().map(|s| s.content.as_ref()).collect();
        assert!(right_text.contains("50.0%"));
    }

    #[test]
    fn test_render_tree_line_percentage_hidden_when_no_scan() {
        // unscanned dir: has_scan_data=false
        let line = make_treeline("d", 0, true, FileType::Directory, false, false);
        let (_, right) = render_tree_line(
            &line,
            false,
            false,
            false,
            "",
            false,
            None,
            Color::Reset,
            2048,
        );
        let right_text: String = right.iter().map(|s| s.content.as_ref()).collect();
        assert!(!right_text.contains('%'));
    }

    #[test]
    fn test_render_tree_line_percentage_hidden_when_root_zero() {
        let line = make_treeline("f", 0, false, FileType::File, false, true);
        let (_, right) = render_tree_line(
            &line,
            false,
            false,
            false,
            "",
            false,
            None,
            Color::Reset,
            0, // root_total_size = 0
        );
        let right_text: String = right.iter().map(|s| s.content.as_ref()).collect();
        assert!(!right_text.contains('%'));
    }
}
