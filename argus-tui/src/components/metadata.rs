use ratatui::{
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::TreeLine;
use crate::util;

/// Render the metadata panel
pub fn render(f: &mut Frame, area: Rect, selected: Option<&TreeLine>, has_delta: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Metadata ")
        .title_alignment(ratatui::layout::Alignment::Left);

    let _inner = block.inner(area);

    let mut lines: Vec<Line> = Vec::new();

    if let Some(line) = selected {
        let node = &line.node;

        // Path
        lines.push(Line::from(vec![
            Span::styled("Path:", Style::default().fg(Color::Gray).bold()),
        ]));
        lines.push(Line::from(vec![
            Span::styled(node.name(), Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![Span::raw("")]));

        // Current Size
        lines.push(Line::from(vec![
            Span::styled("Size:", Style::default().fg(Color::Gray).bold()),
            Span::raw(" "),
            Span::styled(util::format_size(node.current_size()), Style::default().fg(Color::Yellow)),
        ]));

        // Size Delta
        if has_delta {
            let delta = node.size_delta();
            let delta_str = util::format_delta(delta);
            let delta_color = if delta > 0 { Color::Red } else if delta < 0 { Color::Green } else { Color::Gray };
            lines.push(Line::from(vec![
                Span::styled("Delta:", Style::default().fg(Color::Gray).bold()),
                Span::raw(" "),
                Span::styled(delta_str, Style::default().fg(delta_color)),
            ]));
        }

        // File Count
        if node.is_dir() {
            let count = match node {
                crate::app::TreeNode::Snapshot(n) => util::count_file_nodes(n),
                crate::app::TreeNode::Diff(n) => util::count_diff_nodes(n),
            };
            lines.push(Line::from(vec![
                Span::styled("Files:", Style::default().fg(Color::Gray).bold()),
                Span::raw(" "),
                Span::styled(count.to_string(), Style::default().fg(Color::White)),
            ]));
        }

        // Modified Time
        if let Some(modified) = node.modified() {
            lines.push(Line::from(vec![
                Span::styled("Modified:", Style::default().fg(Color::Gray).bold()),
                Span::raw(" "),
                Span::styled(modified.format("%Y-%m-%d %H:%M:%S").to_string(), Style::default().fg(Color::White)),
            ]));
        }

        // Type
        let type_str = if node.is_dir() { "Directory" } else { "File" };
        lines.push(Line::from(vec![
            Span::styled("Type:", Style::default().fg(Color::Gray).bold()),
            Span::raw(" "),
            Span::styled(type_str, Style::default().fg(Color::White)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("No selection", Style::default().fg(Color::Gray)),
        ]));
    }

    let text = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(text, area);
}
