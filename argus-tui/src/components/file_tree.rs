use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::app::{SortMode, TreeLine};
use crate::util;

/// Render the file tree panel
pub fn render(
    f: &mut Frame,
    area: Rect,
    lines: &[TreeLine],
    cursor: usize,
    scroll_offset: usize,
    sort_mode: SortMode,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" File Tree ({}) ", sort_mode.label()))
        .title_alignment(ratatui::layout::Alignment::Left);

    let inner = block.inner(area);

    if lines.is_empty() {
        let text = Paragraph::new("Empty directory")
            .style(Style::default().fg(Color::Gray))
            .block(block);
        f.render_widget(text, area);
        return;
    }

    let visible_height = inner.height.max(1) as usize;

    // Adjust scroll offset
    let scroll_offset = if cursor >= scroll_offset + visible_height {
        cursor.saturating_sub(visible_height) + 1
    } else if cursor < scroll_offset {
        cursor
    } else {
        scroll_offset
    };

    let end = (scroll_offset + visible_height).min(lines.len());
    let visible_lines = &lines[scroll_offset..end];

    let mut rendered_lines: Vec<Line> = Vec::with_capacity(visible_lines.len());

    for (i, line) in visible_lines.iter().enumerate() {
        let global_idx = scroll_offset + i;
        let is_selected = global_idx == cursor;

        let prefix = if line.depth == 0 {
            String::new()
        } else {
            let mut p = String::new();
            // Indentation
            for _ in 0..line.depth.saturating_sub(1) {
                p.push_str("   ");
            }
            if line.depth > 0 {
                p.push_str("├── ");
            }
            p
        };

        let branch = if line.node.is_dir() {
            if line.expanded {
                "▼ "
            } else {
                "▶ "
            }
        } else {
            "  "
        };

        let name_str = format!("{}{}{}", prefix, branch, line.node.name());

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

        let name_style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else if line.node.is_dir() {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let size_style = if line.node.is_dir() && !line.has_scan_data {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Yellow)
        };
        let mut spans = vec![
            Span::styled(name_str, name_style),
            Span::raw("  "),
            Span::styled(size_str, size_style),
        ];

        if !delta_str.is_empty() {
            let delta_color = if line.node.size_delta() > 0 {
                Color::Red
            } else {
                Color::Green
            };
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                delta_str,
                Style::default()
                    .fg(delta_color)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        rendered_lines.push(Line::from(spans));
    }

    let total_lines = lines.len();
    let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll_offset);

    // Render list
    let list = Paragraph::new(rendered_lines).block(block);
    f.render_widget(list, area);

    // Render scrollbar on the right side
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
