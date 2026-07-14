use crate::FinderState;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

/// Render the finder popup centered in the given area.
///
/// Layout:
/// ```text
/// ┌─ Go to Path ──────────────────────┐
/// │ ~/ai/projects/cu                   │  ← input line
/// ├────────────────────────────────────┤
/// │ ~/ai/projects                     │  ← self-item
/// │ ~/ai/projects/cursor/             │
/// │ ~/ai/projects/curl/               │  ← results list
/// │ ~/ai/projects/custom/             │
/// └────────────────────────────────────┘
/// ```
pub fn render_finder_popup(f: &mut Frame, area: Rect, state: &mut FinderState) {
    // 1. Calculate popup dimensions
    let popup_width = area.width.min(80).min((area.width as f64 * 0.65) as u16);
    let popup_height = area.height.min(14).min((area.height as f64 * 0.35) as u16).max(5);

    // 2. Center position
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    };

    // 3. Clear background
    f.render_widget(Clear, popup_area);

    // 4. Draw border block
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Go to Path ")
        .style(Style::default().fg(Color::White).bg(Color::Black));
    let inner = block.inner(popup_area);

    // 5. Split into input line and results list
    let chunks = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    let input_area = chunks[0];
    let results_area = chunks[1];

    // 6. Render input line
    render_input_line(f, input_area, state);

    // 7. Render results list
    render_results_list(f, results_area, state, popup_width);

    // 8. Render block (after content)
    f.render_widget(block, popup_area);

    // 9. Set cursor position
    let cursor_x = x + 1 + state.cursor as u16;
    let cursor_x = cursor_x.min(x + popup_width - 2);
    f.set_cursor_position(ratatui::layout::Position::new(cursor_x, y + 1));
}

/// Render the input line with optional virtual text hint.
fn render_input_line(f: &mut Frame, area: Rect, state: &mut FinderState) {
    let input_text = &state.input;

    // Determine virtual hint text (tab completion hint)
    let hint = if !state.items.is_empty() && state.selected < state.items.len() {
        let selected_item = &state.items[state.selected];
        if !selected_item.is_self {
            let hint_name = &selected_item.name;
            // Only show hint if input doesn't already end with it
            if !input_text.ends_with(hint_name) {
                Some(hint_name.clone())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut spans = Vec::new();
    spans.push(Span::styled(
        input_text.clone(),
        Style::default().fg(Color::White),
    ));

    if let Some(hint) = hint {
        spans.push(Span::styled(
            format!(" → {hint}"),
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC),
        ));
    }

    let paragraph = Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Black));
    f.render_widget(paragraph, area);
}

/// Render the scrollable results list.
fn render_results_list(
    f: &mut Frame,
    area: Rect,
    state: &mut FinderState,
    _popup_width: u16,
) {
    if area.height == 0 || state.items.is_empty() {
        return;
    }

    let visible_count = area.height as usize;
    let total_items = state.items.len();

    // Calculate scroll offset to keep selection visible
    let scroll_offset = if state.selected >= visible_count {
        state.selected - visible_count + 1
    } else {
        0
    };

    // Render visible items
    for i in 0..visible_count {
        let item_idx = scroll_offset + i;
        if item_idx >= total_items {
            break;
        }

        let item = &state.items[item_idx];
        let is_selected = item_idx == state.selected;

        let line_y = area.y + i as u16;
        let line_area = Rect {
            x: area.x,
            y: line_y,
            width: area.width,
            height: 1,
        };

        // Determine style
        let base_style = if is_selected {
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
        } else {
            Style::default().bg(Color::Black).fg(Color::White)
        };

        // Build spans with match highlighting
        let mut spans = Vec::new();
        let display_text = &item.display;

        if item.match_positions.is_empty() {
            // No match highlighting, just render plain text
            spans.push(Span::styled(display_text.clone(), base_style));
        } else {
            // Render with match highlighting
            let mut last_end = 0;
            let sorted_positions = {
                let mut p = item.match_positions.clone();
                p.sort();
                p.dedup();
                p
            };

            for &pos in &sorted_positions {
                // Add text before this match position
                if pos > last_end && last_end < display_text.len() {
                    spans.push(Span::styled(
                        display_text[last_end..pos].to_string(),
                        base_style,
                    ));
                }

                // Add the matched character highlighted
                if pos < display_text.len() {
                    let ch = display_text[pos..].chars().next().unwrap_or_default();
                    let ch_len = ch.len_utf8();
                    let end = pos + ch_len;
                    spans.push(Span::styled(
                        ch.to_string(),
                        base_style
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ));
                    last_end = end;
                }
            }

            // Add remaining text after last match
            if last_end < display_text.len() {
                spans.push(Span::styled(
                    display_text[last_end..].to_string(),
                    base_style,
                ));
            }
        }

        let paragraph = Paragraph::new(Line::from(spans));
        f.render_widget(paragraph, line_area);
    }
}