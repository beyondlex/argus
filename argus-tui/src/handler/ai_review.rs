use crate::app::{App, AppMode};
use crate::types::AiStatus;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

const ITEM_LINES: usize = 4;

pub(crate) fn handle_ai_review_key(key: KeyEvent, app: &mut App) {
    let Some(ref mut state) = app.ai_state else {
        app.mode = AppMode::Browsing;
        return;
    };

    // If info popup is active, Esc/q closes it
    if state.info_item.is_some() {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            state.info_item = None;
        }
        return;
    }

    // If delete confirm is active, handle y/n before other keys
    if state.delete_confirm.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let (_, permanent) = state.delete_confirm.take().unwrap();
                let count = state.mark_for_delete.len();
                let total_size: u64 = state
                    .mark_for_delete
                    .iter()
                    .filter_map(|&i| state.results.get(i))
                    .map(|r| r.size)
                    .sum();
                let action = if permanent {
                    "permanently delete"
                } else {
                    "delete"
                };
                state.mark_for_delete.clear();
                app.set_info(
                    format!(
                        "[Phase 1] Would {} {} item(s) ({} total) — no-op in UI preview",
                        action,
                        count,
                        crate::util::format_size(total_size),
                    ),
                    4,
                );
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                state.delete_confirm = None;
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if state.cursor + 1 < state.results.len() {
                let visible = crossterm::terminal::size()
                    .ok()
                    .map(|(_, h)| ((h as usize).saturating_sub(4)) / ITEM_LINES)
                    .unwrap_or(6);
                if state.cursor >= state.scroll_offset + visible - 1 {
                    state.scroll_offset = state.cursor + 2 - visible;
                }
                state.cursor += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.cursor > 0 {
                state.cursor -= 1;
                if state.cursor < state.scroll_offset {
                    state.scroll_offset = state.cursor;
                }
            }
        }
        KeyCode::Char(' ') => {
            app.ai_review_toggle_mark();
        }
        KeyCode::Char('i') => {
            if state.status == AiStatus::Ready && !state.results.is_empty() {
                state.info_item = Some(state.cursor);
            }
        }
        KeyCode::Char('d') => {
            if state.status != AiStatus::Ready {
                return;
            }
            let paths = collect_marked_paths(state);
            if paths.is_empty() {
                return;
            }
            state.delete_confirm = Some((paths, false));
        }
        KeyCode::Char('D') => {
            if state.status != AiStatus::Ready {
                return;
            }
            let paths = collect_marked_paths(state);
            if paths.is_empty() {
                return;
            }
            state.delete_confirm = Some((paths, true));
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

fn collect_marked_paths(state: &crate::types::AiReviewState) -> Vec<PathBuf> {
    state
        .mark_for_delete
        .iter()
        .filter_map(|&i| state.results.get(i))
        .map(|r| r.path.clone())
        .collect()
}
