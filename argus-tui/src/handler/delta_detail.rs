use crate::app::App;
use crate::app::AppMode;
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_delta_detail_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ref mut state) = app.delta_detail {
                if delta_detail_needs_scroll(state) && state.scroll + 1 < state.entries.len() {
                    state.scroll += 1;
                }
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut state) = app.delta_detail {
                if delta_detail_needs_scroll(state) {
                    state.scroll = state.scroll.saturating_sub(1);
                }
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.delta_detail = None;
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

fn delta_detail_needs_scroll(state: &crate::types::DeltaDetailState) -> bool {
    let visible_rows = crossterm::terminal::size()
        .ok()
        .map(|(_, h)| ((h as f64 * 0.65) as usize).saturating_sub(4))
        .unwrap_or(10);
    state.entries.len() > visible_rows
}
