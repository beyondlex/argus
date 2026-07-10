use ratatui::{
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::{FilterFocus, FilterState, SnapshotInfo};
use crate::util;

/// Render the filter bar
pub fn render(
    f: &mut Frame,
    area: Rect,
    filter: &FilterState,
    snapshots: &[SnapshotInfo],
    focus: bool,
    sub_focus: FilterFocus,
    has_enough_snapshots: bool,
) {
    let border_color = if focus { Color::Cyan } else { Color::DarkGray };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
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

    let threshold_str = filter.threshold.map(util::format_size).unwrap_or_default();

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

    let active_style = Style::default().fg(Color::Yellow).bg(Color::DarkGray);
    let idle_style = Style::default().fg(Color::Cyan);
    let threshold_idle = Style::default().fg(Color::Magenta);

    let from_style = if focus && sub_focus == FilterFocus::From {
        active_style
    } else {
        idle_style
    };
    let to_style = if focus && sub_focus == FilterFocus::To {
        active_style
    } else {
        idle_style
    };
    let threshold_style = if focus && sub_focus == FilterFocus::Threshold {
        active_style
    } else {
        threshold_idle
    };

    let spans = vec![
        Span::styled("Time:", Style::default().fg(Color::White).bold()),
        Span::raw(" ["),
        Span::styled(from_str, from_style),
        Span::raw(" → "),
        Span::styled(to_str, to_style),
        Span::raw("]"),
        Span::raw("  Δ≥ ["),
        Span::styled(threshold_str, threshold_style),
        Span::raw("]"),
        Span::raw("  "),
        Span::styled("[Clear]", Style::default().fg(Color::Red)),
        Span::styled(diff_hint, Style::default().fg(Color::DarkGray)),
    ];

    let text = Paragraph::new(Line::from(spans)).block(block);
    f.render_widget(text, area);
}
