use crate::app::{App, AppMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn handle_ai_review_key(key: KeyEvent, app: &mut App) {
    let Some(ref mut state) = app.ai_state else {
        app.mode = AppMode::Browsing;
        return;
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if state.cursor + 1 < state.results.len() {
                state.cursor += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        KeyCode::Char(' ') => {
            app.ai_review_toggle_mark();
        }
        KeyCode::Enter => {
            let marked_count = state.mark_for_delete.len();
            let total_size: u64 = state
                .mark_for_delete
                .iter()
                .filter_map(|&i| state.results.get(i))
                .map(|r| r.size)
                .sum();
            app.exit_ai_review();
            app.set_info(
                format!(
                    "[Phase 1] Would delete {} item(s) ({} total) — no-op in UI preview",
                    marked_count,
                    crate::util::format_size(total_size),
                ),
                4,
            );
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.exit_ai_review();
        }
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
            app.should_quit = true;
        }
        _ => {}
    }
}
