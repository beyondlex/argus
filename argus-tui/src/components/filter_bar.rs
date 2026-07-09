use ratatui::{
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::{FilterState, SnapshotInfo};
use crate::util;

#[allow(dead_code)]
pub enum FilterArea {
    From,
    To,
    Threshold,
    Clear,
}

/// Render the filter bar
pub fn render(
    f: &mut Frame,
    area: Rect,
    filter: &FilterState,
    snapshots: &[SnapshotInfo],
    focus: bool,
    has_enough_snapshots: bool,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Filter ")
        .title_alignment(ratatui::layout::Alignment::Left);

    let _inner = block.inner(area);

    let from_str = if let Some(idx) = filter.from_idx {
        snapshots
            .get(idx)
            .map(|s| s.timestamp.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let to_str = if let Some(idx) = filter.to_idx {
        snapshots
            .get(idx)
            .map(|s| s.timestamp.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let threshold_str = filter
        .threshold
        .map(|t| util::format_size(t))
        .unwrap_or_default();

    let _can_diff = has_enough_snapshots && filter.from_idx.is_some() && filter.to_idx.is_some();
    let diff_hint = if !has_enough_snapshots {
        " (need 2 snapshots)"
    } else if filter.from_idx.is_some()
        && filter.to_idx.is_some()
        && filter.from_idx == filter.to_idx
    {
        " (different timestamps needed)"
    } else {
        ""
    };

    let from_style = if focus {
        Style::default().fg(Color::Yellow).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let spans = vec![
        Span::styled("Time:", Style::default().fg(Color::White).bold()),
        Span::raw(" ["),
        Span::styled(from_str, from_style),
        Span::raw(" → "),
        Span::styled(
            to_str,
            if focus {
                Style::default().fg(Color::Yellow).bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Cyan)
            },
        ),
        Span::raw("]"),
        Span::raw("  Δ≥ ["),
        Span::styled(
            threshold_str,
            if focus {
                Style::default().fg(Color::Yellow).bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Magenta)
            },
        ),
        Span::raw("]"),
        Span::raw("  "),
        Span::styled("[Clear]", Style::default().fg(Color::Red)),
        Span::styled(diff_hint, Style::default().fg(Color::DarkGray)),
    ];

    let text = Paragraph::new(Line::from(spans)).block(block);
    f.render_widget(text, area);
}
