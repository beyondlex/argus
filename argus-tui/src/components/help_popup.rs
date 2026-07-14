use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
    Frame,
};

use crate::components::popup::{popup_block, PopupStyle};
use crate::util::key_hints;

/// Render the help popup overlay
pub fn render(f: &mut Frame, area: Rect) {
    let popup_area = crate::components::popup::centered_rect(60, 80, area);

    f.render_widget(Clear, popup_area);

    let block = popup_block(" Help ", PopupStyle::Normal);

    let lines = vec![
        Line::from(vec![Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(key_hints(&[("j/k", "Move cursor up/down")])),
        Line::from(key_hints(&[("h", "Collapse directory")])),
        Line::from(key_hints(&[("H", "Collapse all directories")])),
        Line::from(key_hints(&[("u", "Go to parent directory")])),
        Line::from(key_hints(&[(
            "l / Right",
            "Expand directory / enter child",
        )])),
        Line::from(key_hints(&[("Enter", "Edit filter word")])),
        Line::from(key_hints(&[("w", "Set cursor dir as tree root")])),
        Line::from(key_hints(&[(".", "Toggle hidden files")])),
        Line::from(key_hints(&[("s", "Scan directory")])),
        Line::from(key_hints(&[("o", "Toggle sort mode")])),
        Line::from(key_hints(&[("d", "Delete selected item")])),
        Line::from(key_hints(&[("Tab", "Focus next panel")])),
        Line::from(key_hints(&[("/", "Search items in tree")])),
        Line::from(key_hints(&[("n/N", "Next/prev match (with filter)")])),
        Line::from(key_hints(&[("?", "Toggle this help")])),
        Line::from(key_hints(&[("q / Ctrl+C", "Quit")])),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Sort Modes:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("  Name: Alphabetical by name")]),
        Line::from(vec![Span::raw("  Size: By total size (desc)")]),
        Line::from(vec![Span::raw("  Delta: By absolute change (desc)")]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Filter Bar:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw(
            "  :Time <N>[m|h|d|w] | <from> to <to>  Set time range",
        )]),
        Line::from(vec![Span::raw("  :Delta <N>[k|m|g]  Set delta threshold")]),
        Line::from(vec![Span::raw("  Press 'Clear' to reset filters")]),
        Line::from(vec![Span::raw("")]),
        Line::from(key_hints(&[("? / Esc", "Close")])),
    ];

    let text = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(text, popup_area);
}
