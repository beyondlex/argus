use crate::app::App;
use crate::app::AppMode;
use crate::components::delta_detail::detail_popup_visible_rows;
use crossterm::event::{KeyCode, KeyEvent};

/// Fallback frame height when the terminal size is unavailable (essentially
/// only in tests): a ~24-row terminal is the smallest sane popup host.
const FALLBACK_FRAME_HEIGHT: u16 = 24;

pub fn handle_delta_detail_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ref mut state) = app.delta_detail {
                let visible = visible_rows();
                state.scroll = scroll_down(state.scroll, state.entries.len(), visible);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut state) = app.delta_detail {
                state.scroll = state.scroll.saturating_sub(1);
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.delta_detail = None;
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

/// Furthest scroll offset that still keeps the viewport filled: the last full
/// page, not the last row. `j` used to scroll until the bottom entry reached
/// the popup's top, stranding blank rows under it (review #67).
fn scroll_down(scroll: usize, entry_count: usize, visible_rows: usize) -> usize {
    scroll
        .saturating_add(1)
        .min(entry_count.saturating_sub(visible_rows))
}

fn visible_rows() -> usize {
    let frame_height = crossterm::terminal::size()
        .map(|(_, h)| h)
        .unwrap_or(FALLBACK_FRAME_HEIGHT);
    detail_popup_visible_rows(frame_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scrolling must stop at the last full page: with 10 entries and 4
    /// visible rows the deepest offset is 6, where rows 6..10 fill the view.
    #[test]
    fn test_scroll_down_clamps_to_last_full_page() {
        assert_eq!(scroll_down(0, 10, 4), 1);
        assert_eq!(scroll_down(5, 10, 4), 6);
        assert_eq!(scroll_down(6, 10, 4), 6, "past the last full page");
        assert_eq!(scroll_down(9, 10, 4), 6, "used to reach 9 (last row on top)");
    }

    /// A list shorter than one page never scrolls.
    #[test]
    fn test_scroll_down_no_scroll_when_list_fits() {
        assert_eq!(scroll_down(0, 4, 4), 0);
        assert_eq!(scroll_down(0, 3, 4), 0);
        assert_eq!(scroll_down(0, 0, 4), 0);
    }

    /// The shared geometry helper must yield a sane, monotonic row count.
    #[test]
    fn test_visible_rows_grow_with_frame_height() {
        let small = detail_popup_visible_rows(20);
        let large = detail_popup_visible_rows(60);
        assert!(small < large);
        assert!(large >= small + 10);
    }
}
