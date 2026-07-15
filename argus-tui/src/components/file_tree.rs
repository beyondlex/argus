use std::collections::HashMap;
use std::path::Path;

use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::app::{SearchMatch, SearchMode, TreeLine};
use crate::search::fuzzy_match_indices;
use crate::theme::ColorTheme;
use crate::util;
use crate::util::key_hints;

const SCROLL_MARGIN: usize = 3;
const NAME_FIXED_WIDTH: u16 = 30;
const DELTA_WIDTH: usize = 12;
const SIZE_WIDTH: usize = 14;

use std::collections::HashSet;

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
    pub multi_select: bool,
    pub selected_paths: &'a HashSet<Vec<String>>,
    pub theme: &'a ColorTheme,
}

pub fn render(f: &mut Frame, area: Rect, ctx: TreeRenderCtx) {
    let title = format!("{} ", ctx.view_root_path.display());
    let title_style = Style::default().fg(if ctx.focus {
        ctx.theme.accent
    } else {
        ctx.theme.text_secondary
    });
    let border_style = Style::default().fg(if ctx.focus {
        ctx.theme.accent
    } else {
        ctx.theme.border_unfocused
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
        ctx.theme,
    );

    let status_area = Rect {
        x: inner.x + 2,
        y: inner.y,
        width: content_width.saturating_sub(2),
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
            is_active_match && ctx.match_indices[ctx.current_match].tree_line_idx == Some(tree_idx);

        let row_bg = if is_current_match {
            ctx.theme.match_bg
        } else if is_selected {
            ctx.theme.selected_bg
        } else {
            Color::Reset
        };

        let delta = ctx.delta_cache.and_then(|c| c.get(&line.path).copied());

        let is_selected_item = ctx.multi_select && ctx.selected_paths.contains(&line.path);

        let (mut left_spans, right_spans) = render_tree_line(
            line,
            is_selected,
            is_current_match,
            ctx.search_mode != SearchMode::Inactive && !ctx.search_word.is_empty(),
            ctx.search_word,
            has_delta,
            delta,
            row_bg,
            ctx.root_total_size,
            ctx.multi_select,
            is_selected_item,
            ctx.theme,
        );

        let row_y = inner.y + 1 + display_offset as u16;
        let row_area = Rect {
            x: inner.x + 2,
            y: row_y,
            width: content_width.saturating_sub(2),
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
        truncate_name_spans(&mut left_spans, NAME_FIXED_WIDTH as usize);
        let [name_area, info_area] = Layout::horizontal([
            Constraint::Length(NAME_FIXED_WIDTH),
            Constraint::Length(right_width),
        ])
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
    theme: &ColorTheme,
) -> (Vec<Span<'a>>, Vec<Span<'a>>) {
    match search_mode {
        SearchMode::Inactive => (
            vec![Span::styled(
                "  [type / to search]",
                Style::default().fg(theme.text_tertiary),
            )],
            vec![],
        ),
        SearchMode::Input => {
            let mut display = search_word.to_string();
            display.push(if cursor_visible { '▎' } else { ' ' });
            let count = format!(" ({}/{})", match_indices.len(), total_visible);
            (
                vec![
                    Span::styled(
                        format!("  {display}"),
                        Style::default().fg(theme.text_highlight),
                    ),
                    Span::styled(count, Style::default().fg(theme.text_tertiary)),
                ],
                vec![],
            )
        }
        SearchMode::Active => {
            let count = format!(" ({}/{}) ", match_indices.len(), total_visible);
            let left = vec![
                Span::styled(
                    format!("  {}", search_word.to_string()),
                    Style::default().fg(theme.success),
                ),
                Span::styled(count, Style::default().fg(theme.text_tertiary)),
            ];
            let right = key_hints(
                &[
                    ("n", "next"),
                    ("N", "prev"),
                    ("Esc", "clear"),
                    ("Enter", "edit"),
                ],
                theme,
            );
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

struct RowStyle {
    row_bg: Color,
    highlighted: bool,
    // Owned colors from theme to avoid lifetime
    text: Color,
    selection_fg: Color,
    text_tertiary: Color,
    success: Color,
    focus_fg: Color,
    search_match_selected_bg: Color,
    match_bg: Color,
}

impl RowStyle {
    fn new(row_bg: Color, highlighted: bool, theme: &ColorTheme) -> Self {
        Self {
            row_bg,
            highlighted,
            text: theme.text,
            selection_fg: theme.selection_fg,
            text_tertiary: theme.text_tertiary,
            success: theme.success,
            focus_fg: theme.focus_fg,
            search_match_selected_bg: theme.search_match_selected_bg,
            match_bg: theme.match_bg,
        }
    }

    fn base(&self) -> Style {
        Style::default().fg(self.text)
    }

    fn sel(&self) -> Style {
        if self.highlighted {
            Style::default()
                .bg(self.row_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    }

    fn prefix(&self) -> Style {
        self.base().patch(self.sel())
    }

    fn delta(&self, delta_fg: Color) -> Style {
        self.base().patch(self.sel()).fg(if self.highlighted {
            self.selection_fg
        } else {
            delta_fg
        })
    }

    fn filesize(&self, fg: Color) -> Style {
        self.base().patch(self.sel()).fg(if self.highlighted {
            self.selection_fg
        } else {
            fg
        })
    }

    fn percent(&self) -> Style {
        self.base().patch(self.sel()).fg(if self.highlighted {
            self.selection_fg
        } else {
            self.text_tertiary
        })
    }

    fn name(&self, entry_fg: Color) -> Style {
        self.base().fg(entry_fg).patch(self.sel())
    }

    fn name_match(&self) -> Style {
        self.base().fg(self.success).add_modifier(Modifier::BOLD)
    }

    fn name_match_selected_filename(&self) -> Style {
        Style::default()
            .fg(self.focus_fg)
            .bg(self.search_match_selected_bg)
            .add_modifier(Modifier::BOLD)
    }

    fn name_match_selected_matchword(&self) -> Style {
        Style::default()
            .fg(self.focus_fg)
            .bg(self.match_bg)
            .add_modifier(Modifier::BOLD)
    }
}

fn entry_fg(line: &TreeLine, theme: &ColorTheme) -> Color {
    let name = line.node.name();
    if name.starts_with('.') {
        return theme.hidden;
    }
    if line.node.is_dir() {
        theme.accent
    } else if is_symlink(line) {
        theme.symlink
    } else {
        theme.text
    }
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
    multi_select: bool,
    is_selected_item: bool,
    theme: &ColorTheme,
) -> (Vec<Span<'a>>, Vec<Span<'a>>) {
    let highlighted = is_selected || is_current_match;
    let row = RowStyle::new(row_bg, highlighted, theme);

    let multi_indicator = if multi_select {
        if is_selected_item {
            "●"
        } else {
            "○"
        }
    } else {
        ""
    };
    let name_prefix = format!(
        "{}{}{}",
        line_indent(line.depth),
        multi_indicator,
        if multi_indicator.is_empty() {
            branch_marker(line).to_string()
        } else {
            format!(" {}", branch_marker(line))
        },
    );

    let size_str = util::display_size_label(
        line.node.is_dir(),
        line.has_scan_data,
        line.node.current_size(),
    );

    let left = name_spans(line, name_prefix, has_search, search_word, &row, theme);

    let mut right = Vec::new();

    // Percent column
    if root_total_size > 0 && line.has_scan_data {
        let pct = (line.node.current_size() as f64 / root_total_size as f64) * 100.0;
        right.push(Span::styled(format!("{:>6.1}%", pct), row.percent()));
        right.push(Span::raw(" "));
    } else {
        right.push(Span::styled(format!("{:>7}", "?"), row.percent()));
        right.push(Span::raw(" "));
    }

    // Size column
    if line.node.is_dir() && !line.has_scan_data {
        let real_size = util::format_size(line.node.current_size());
        let padded = format!("{:>width$}", real_size, width = SIZE_WIDTH);
        right.push(Span::styled(padded, row.filesize(theme.text_tertiary)));
        right.push(Span::raw(" "));
    } else {
        let trimmed = size_str.trim().to_string();
        if let Some(space_idx) = trimmed.rfind(' ') {
            let size_total = size_str.len();
            if size_total < SIZE_WIDTH {
                right.push(Span::raw(" ".repeat(SIZE_WIDTH - size_total)));
            }
            let leading = size_str.len() - size_str.trim_start().len();
            let num = format!("{}{} ", &size_str[..leading], &trimmed[..space_idx]);
            right.push(Span::styled(num, row.filesize(theme.text_secondary)));
            let unit = &trimmed[space_idx + 1..];
            right.push(Span::styled(
                unit.to_string(),
                row.filesize(util::filesize_unit_color(unit, theme)),
            ));
            right.push(Span::raw(" "));
        } else {
            let padded = format!("{:>width$}", size_str.clone(), width = SIZE_WIDTH);
            right.push(Span::styled(padded, row.filesize(theme.text_secondary)));
            right.push(Span::raw(" "));
        }
    }

    // Delta column
    if has_delta {
        let (delta_str, delta_fg) = match delta {
            Some(d) if d > 0 => {
                let s = format!("+{}", util::format_size(d as u64));
                let color = util::delta_unit_color(util::extract_unit(&s), theme);
                (s, color)
            }
            Some(d) if d < 0 => (
                format!("-{}", util::format_size(d.unsigned_abs())),
                theme.success,
            ),
            Some(_) | None => ("-".to_string(), theme.text_tertiary),
        };
        let padded = format!("{:>width$}", delta_str, width = DELTA_WIDTH);
        right.push(Span::styled(padded, row.delta(delta_fg)));
    }

    (left, right)
}

/// Truncate name spans so they fit within `max_width` characters.
fn truncate_name_spans<'a>(spans: &mut Vec<Span<'a>>, max_width: usize) {
    let total: usize = spans.iter().map(|s| s.content.len()).sum();
    if total <= max_width {
        return;
    }

    let prefix_len = spans.first().map(|s| s.content.len()).unwrap_or(0);
    let avail = max_width.saturating_sub(prefix_len).saturating_sub(3);

    if avail < 1 {
        let prefix = spans
            .first()
            .map(|s| s.content.to_string())
            .unwrap_or_default();
        let style = spans.first().map(|s| s.style).unwrap_or_default();
        let truncated: String = prefix.chars().take(max_width.saturating_sub(3)).collect();
        *spans = vec![Span::styled(format!("{}...", truncated), style)];
        return;
    }

    let prefix = spans
        .first()
        .map(|s| s.content.to_string())
        .unwrap_or_default();
    let prefix_style = spans.first().map(|s| s.style).unwrap_or_default();
    let name_style = spans.last().map(|s| s.style).unwrap_or_default();
    let name_text: String = spans[1..].iter().map(|s| s.content.to_string()).collect();
    let truncated_name: String = name_text.chars().take(avail).collect();

    *spans = vec![
        Span::styled(prefix, prefix_style),
        Span::styled(format!("{}...", truncated_name), name_style),
    ];
}

fn name_spans<'a>(
    line: &'a TreeLine,
    name_prefix: String,
    has_search: bool,
    search_word: &'a str,
    row: &RowStyle,
    theme: &ColorTheme,
) -> Vec<Span<'a>> {
    let name_text = line.node.name();

    if has_search && !search_word.is_empty() {
        let mut spans = vec![Span::styled(name_prefix, row.prefix())];

        if let Some(indices) = fuzzy_match_indices(search_word, name_text) {
            if row.highlighted {
                spans.extend(match_highlight_spans(name_text, &indices, row));
            } else {
                spans.push(Span::styled(name_text, row.name_match()));
            }
        } else {
            spans.push(Span::styled(name_text, row.name(entry_fg(line, theme))));
        }
        spans
    } else {
        let name_style = row.name(entry_fg(line, theme));
        let mut spans = vec![Span::styled(name_prefix, row.prefix())];
        spans.push(Span::styled(name_text, name_style));
        spans
    }
}

fn match_highlight_spans<'a>(
    text: &'a str,
    match_indices: &[usize],
    row: &RowStyle,
) -> Vec<Span<'a>> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut prev_end = 0;
    let match_set: std::collections::HashSet<usize> = match_indices.iter().copied().collect();

    for (ci, _ch) in chars.iter().enumerate() {
        if match_set.contains(&ci) {
            if ci > prev_end {
                let normal: String = chars[prev_end..ci].iter().collect();
                if row.highlighted {
                    spans.push(Span::styled(normal, row.name_match_selected_filename()));
                } else {
                    spans.push(Span::styled(normal, row.name_match()));
                }
            }
            if row.highlighted {
                spans.push(Span::styled(
                    chars[ci].to_string(),
                    row.name_match_selected_matchword(),
                ));
            } else {
                spans.push(Span::styled(chars[ci].to_string(), row.name_match()));
            }
            prev_end = ci + 1;
        }
    }
    if prev_end < chars.len() {
        let rest: String = chars[prev_end..].iter().collect();
        if row.highlighted {
            spans.push(Span::styled(rest, row.name_match_selected_filename()));
        } else {
            spans.push(Span::styled(rest, row.name_match()));
        }
    }

    spans
}
