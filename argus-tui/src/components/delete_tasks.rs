use crate::render::SPINNER_FRAMES;
use crate::theme::ColorTheme;
use crate::types::{DeleteTask, DeleteTaskState};
use crate::util::key_hints;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Width of the right-hand pane, including borders.
pub const PANE_WIDTH: u16 = 34;

/// Rows one task occupies in the pane: label, gauge, blank separator.
const TASK_ROWS: u16 = 3;

/// Fit `text` into `width` display columns, keeping the tail (paths are most
/// identifiable by their last components) after a leading ellipsis.
fn truncate_head(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(3);
    let chars: Vec<char> = text.chars().collect();
    let mut tail = String::new();
    let mut used = 0;
    for c in chars.iter().rev() {
        let w = c.to_string().width();
        if used + w > keep {
            break;
        }
        tail.insert(0, *c);
        used += w;
    }
    format!("...{}", tail)
}

/// Render the right-hand delete-task pane. Every registered task keeps a row
/// until it is cleared with `T`, so completion and failure counts stay visible.
pub fn render(f: &mut Frame, area: Rect, tasks: &[DeleteTask], theme: &ColorTheme, spinner: u8) {
    f.render_widget(Clear, area);

    let running = tasks.iter().filter(|t| t.is_running()).count();
    let title = format!(" Delete tasks ({running} active) ");
    let footer = Line::from(key_hints(&[("T", "Clear done")], theme)).centered();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused))
        .style(Style::default().bg(theme.popup_bg))
        .title_style(Style::default().fg(theme.accent))
        .title_alignment(Alignment::Center)
        .title(title)
        .title_bottom(footer);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let max_label = inner.width.saturating_sub(2) as usize;
    let visible = (inner.height / TASK_ROWS) as usize;
    // Newest tasks first: the user just confirmed them and wants to see them
    // near the top rather than scroll past a long completed history.
    let shown: Vec<&DeleteTask> = tasks.iter().rev().take(visible).collect();
    let mut constraints = vec![Constraint::Length(TASK_ROWS); shown.len()];
    constraints.push(Constraint::Min(0));
    let rows = Layout::vertical(constraints).split(inner);

    for (i, task) in shown.iter().enumerate() {
        let row = rows[i];
        let [label_area, gauge_area, _] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(row);

        let (head, ratio, label, color) = match task.state {
            DeleteTaskState::Running { current, total } => {
                let r = if total > 0 {
                    current as f64 / total as f64
                } else {
                    0.0
                };
                let action = if task.permanent { "rm" } else { "trash" };
                let spin = if task.spinner && current < total {
                    format!(
                        "{} ",
                        SPINNER_FRAMES[spinner as usize % SPINNER_FRAMES.len()]
                    )
                } else {
                    String::new()
                };
                (
                    format!(
                        "{}[{}] {}",
                        spin,
                        action,
                        truncate_head(&task.label, max_label)
                    ),
                    r,
                    format!("{:.0}% ({}/{})", r * 100.0, current, total),
                    theme.danger,
                )
            }
            DeleteTaskState::Finished { failed } => {
                if failed > 0 {
                    (
                        format!("✗ {}", truncate_head(&task.label, max_label)),
                        1.0,
                        format!("{} failed", failed),
                        theme.danger,
                    )
                } else {
                    (
                        format!("✓ {}", truncate_head(&task.label, max_label)),
                        1.0,
                        "done".to_string(),
                        theme.success,
                    )
                }
            }
        };

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                head,
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ))),
            label_area,
        );

        let gauge = Gauge::default()
            .gauge_style(
                Style::default()
                    .fg(color)
                    .bg(theme.bg)
                    .add_modifier(Modifier::BOLD),
            )
            .ratio(ratio)
            .label(label);
        f.render_widget(gauge, gauge_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_head_keeps_fits_or_tail() {
        assert_eq!(truncate_head("short", 10), "short");
        assert_eq!(truncate_head("abcdefgh", 6), "...fgh");
        assert_eq!(truncate_head("abcdefgh", 3), "...");
    }

    #[test]
    fn test_truncate_head_counts_display_width() {
        // CJK glyphs are two columns wide.
        assert_eq!(truncate_head("目录abc", 6), "...abc");
    }
}
