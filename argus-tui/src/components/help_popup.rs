use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
    Frame,
};

use crate::components::popup::{popup_block, PopupStyle};
use crate::theme::ColorTheme;
use crate::util::key_hints;

/// Render the help popup overlay
pub fn render(f: &mut Frame, area: Rect, theme: &ColorTheme) {
    let popup_area = crate::components::popup::centered_rect(60, 80, area);

    f.render_widget(Clear, popup_area);

    let block = popup_block(" Help ", PopupStyle::Normal, theme);

    let lines = vec![
        Line::from(vec![Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(key_hints(&[("j/k", "Move cursor up/down")], theme)),
        Line::from(key_hints(&[("h", "Collapse directory")], theme)),
        Line::from(key_hints(&[("H", "Collapse all directories")], theme)),
        Line::from(key_hints(&[("u", "Go to parent directory")], theme)),
        Line::from(key_hints(
            &[("l / Right", "Expand directory / enter child")],
            theme,
        )),
        Line::from(key_hints(&[("Enter", "Edit filter word")], theme)),
        Line::from(key_hints(&[("w", "Set cursor dir as tree root")], theme)),
        Line::from(key_hints(&[(".", "Toggle hidden files")], theme)),
        Line::from(key_hints(&[("s", "Scan directory")], theme)),
        Line::from(key_hints(&[("o", "Toggle sort mode")], theme)),
        Line::from(key_hints(&[("d", "Delete selected item")], theme)),
        Line::from(key_hints(&[("Tab", "Focus next panel")], theme)),
        Line::from(key_hints(&[("/", "Search items in tree")], theme)),
        Line::from(key_hints(
            &[("n/N", "Next/prev match (with filter)")],
            theme,
        )),
        Line::from(key_hints(&[("?", "Toggle this help")], theme)),
        Line::from(key_hints(&[("q / Ctrl+C", "Quit")], theme)),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Sort Modes:",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("  Name: Alphabetical by name")]),
        Line::from(vec![Span::raw("  Size: By total size (desc)")]),
        Line::from(vec![Span::raw("  Delta: By absolute change (desc)")]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            "Filter Bar:",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw(
            "  :Time <N>[m|h|d|w] | <from> to <to>  Set time range",
        )]),
        Line::from(vec![Span::raw("  :Delta <N>[k|m|g]  Set delta threshold")]),
        Line::from(vec![Span::raw("  Press 'Clear' to reset filters")]),
        Line::from(vec![Span::raw("")]),
        Line::from(key_hints(&[("? / Esc", "Close")], theme)),
    ];

    let text = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(text, popup_area);
}
