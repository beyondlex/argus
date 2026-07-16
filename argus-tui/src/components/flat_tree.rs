use std::collections::HashMap;
use std::path::Path;

use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::app::{DirEntry, SearchMode};
use crate::search::fuzzy_match_indices;
use crate::theme::ColorTheme;
use crate::util;
use crate::util::key_hints;

const SCROLL_MARGIN: usize = 3;
const DELTA_WIDTH: usize = 12;
const SIZE_WIDTH: usize = 14;

use std::collections::HashSet;

pub struct FlatRenderCtx<'a> {
    pub children: &'a [DirEntry],
    pub filtered_indices: &'a [usize],
    pub cursor: usize,
    pub scroll_offset: usize,
    pub view_root_path: &'a Path,
    pub current_dir_path: &'a [String],
    pub search_word: &'a str,
    pub search_mode: SearchMode,
    pub cursor_visible: bool,
    pub focus: bool,
    pub delta_cache: Option<&'a HashMap<Vec<String>, i64>>,
    pub current_dir_total: u64,
    pub multi_select: bool,
    pub selected_paths: &'a HashSet<Vec<String>>,
    pub theme: &'a ColorTheme,
}

pub fn render(f: &mut Frame, area: Rect, ctx: FlatRenderCtx) {
    // Build breadcrumb title from current_dir_path
    // current_dir_path[0] is the root node name (already in view_root_path),
    // so we skip it for the display path.
    let title = if ctx.current_dir_path.len() <= 1 {
        format!("{} ", ctx.view_root_path.display())
    } else {
        let subpath = ctx.current_dir_path[1..].join("/");
        let full = ctx.view_root_path.join(&subpath);
        format!("{} ", full.display())
    };
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

    let (status_left, status_right) = flat_search_status_line(
        ctx.search_mode,
        ctx.search_word,
        ctx.cursor_visible,
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

    for (display_offset, &child_idx) in visible_indices.iter().enumerate() {
        let Some(entry) = ctx.children.get(child_idx) else {
            continue;
        };
        let global_idx = scroll_offset + display_offset;
        let is_selected = global_idx == ctx.cursor;

        let row_bg = if is_selected {
            ctx.theme.selected_bg
        } else {
            Color::Reset
        };

        let delta = ctx.delta_cache.and_then(|c| c.get(&entry.path).copied());

        let is_selected_item = ctx.multi_select && ctx.selected_paths.contains(&entry.path);

        let (info_spans, mut name_spans) = render_flat_entry(
            entry,
            is_selected,
            ctx.search_mode != SearchMode::Inactive && !ctx.search_word.is_empty(),
            ctx.search_word,
            has_delta,
            delta,
            row_bg,
            ctx.current_dir_total,
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

        let info_width: u16 = info_spans.iter().map(|s| s.content.len() as u16).sum();
        let name_max_width = row_area.width.saturating_sub(info_width);
        truncate_name_spans(&mut name_spans, name_max_width as usize);
        let [info_area, name_area] =
            Layout::horizontal([Constraint::Length(info_width), Constraint::Fill(1)])
                .areas(row_area);

        f.render_widget(Paragraph::new(Line::from(info_spans)), info_area);
        f.render_widget(Paragraph::new(Line::from(name_spans)), name_area);
    }
}

fn flat_search_status_line<'a>(
    search_mode: SearchMode,
    search_word: &'a str,
    cursor_visible: bool,
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
            let count = format!(" ({})", total_visible);
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
            let count = format!(" ({}) ", total_visible);
            let left = vec![
                Span::styled(
                    format!("  {search_word}"),
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

#[allow(clippy::too_many_arguments)]
fn render_flat_entry(
    entry: &DirEntry,
    is_selected: bool,
    has_search: bool,
    search_word: &str,
    has_delta: bool,
    delta: Option<i64>,
    row_bg: Color,
    current_dir_total: u64,
    multi_select: bool,
    is_selected_item: bool,
    theme: &ColorTheme,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let highlighted = is_selected;
    let row = RowStyle::new(row_bg, highlighted, theme);

    let multi_indicator = if multi_select {
        if is_selected_item {
            "● "
        } else {
            "○ "
        }
    } else {
        ""
    };
    let name_prefix = multi_indicator.to_string();

    let name_text = if entry.is_dir {
        format!("{}/", entry.node.name())
    } else {
        entry.node.name().to_string()
    };

    let size_str = util::display_size_label(entry.is_dir, entry.has_scan_data, entry.size);

    let mut info = Vec::new();

    // Delta column (first)
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
        info.push(Span::styled(padded, row.delta(delta_fg)));
        info.push(Span::raw(" "));
    }

    // Percent column
    if current_dir_total > 0 && entry.has_scan_data {
        let pct = (entry.size as f64 / current_dir_total as f64) * 100.0;
        info.push(Span::styled(format!("{:>6.1}%", pct), row.percent()));
        info.push(Span::raw(" "));
    } else {
        info.push(Span::styled(format!("{:>7}", "?"), row.percent()));
        info.push(Span::raw(" "));
    }

    // Size column
    if entry.is_dir && !entry.has_scan_data {
        let real_size = util::format_size(entry.size);
        let padded = format!("{:>width$}", real_size, width = SIZE_WIDTH);
        info.push(Span::styled(padded, row.filesize(theme.text_tertiary)));
        info.push(Span::raw(" "));
    } else {
        let trimmed = size_str.trim().to_string();
        if let Some(space_idx) = trimmed.rfind(' ') {
            let size_total = size_str.len();
            if size_total < SIZE_WIDTH {
                info.push(Span::raw(" ".repeat(SIZE_WIDTH - size_total)));
            }
            let leading = size_str.len() - size_str.trim_start().len();
            let num = format!("{}{} ", &size_str[..leading], &trimmed[..space_idx]);
            info.push(Span::styled(num, row.filesize(theme.text_secondary)));
            let unit = &trimmed[space_idx + 1..];
            info.push(Span::styled(
                unit.to_string(),
                row.filesize(util::filesize_unit_color(unit, theme)),
            ));
            info.push(Span::raw(" "));
        } else {
            let padded = format!("{:>width$}", size_str.clone(), width = SIZE_WIDTH);
            info.push(Span::styled(padded, row.filesize(theme.text_secondary)));
            info.push(Span::raw(" "));
        }
    }

    let name = name_spans(
        &name_text,
        &name_prefix,
        has_search,
        search_word,
        &row,
        theme,
    );

    (info, name)
}

// ── Reusable helper types/functions (adapted from file_tree.rs) ──

struct RowStyle {
    row_bg: Color,
    highlighted: bool,
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

fn name_spans(
    name_text: &str,
    name_prefix: &str,
    has_search: bool,
    search_word: &str,
    row: &RowStyle,
    theme: &ColorTheme,
) -> Vec<Span<'static>> {
    if has_search && !search_word.is_empty() {
        let mut spans = vec![Span::styled(name_prefix.to_string(), row.prefix())];

        if let Some(indices) = fuzzy_match_indices(search_word, name_text) {
            if row.highlighted {
                spans.extend(match_highlight_spans(name_text, &indices, row));
            } else {
                spans.push(Span::styled(name_text.to_string(), row.name_match()));
            }
        } else {
            // Name doesn't match search — still show it but dimmed
            spans.push(Span::styled(
                name_text.to_string(),
                row.name(entry_fg_raw(name_text, theme)),
            ));
        }
        spans
    } else {
        let name_style = row.name(entry_fg_raw(name_text, theme));
        let mut spans = vec![Span::styled(name_prefix.to_string(), row.prefix())];
        spans.push(Span::styled(name_text.to_string(), name_style));
        spans
    }
}

/// Determine entry color from name text (handles trailing / for dirs)
fn entry_fg_raw(name: &str, theme: &ColorTheme) -> Color {
    if name.starts_with('.') {
        return theme.hidden;
    }
    if name.ends_with('/') {
        theme.accent
    } else {
        theme.text
    }
}

fn match_highlight_spans(
    text: &str,
    match_indices: &[usize],
    row: &RowStyle,
) -> Vec<Span<'static>> {
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

/// Truncate name spans so they fit within `max_width` characters.
fn truncate_name_spans(spans: &mut Vec<Span<'static>>, max_width: usize) {
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
