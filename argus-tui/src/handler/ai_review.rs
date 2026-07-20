use crate::app::{App, AppMode};
use crate::types::AiStatus;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

const ITEM_LINES: usize = 4;

pub(crate) fn handle_ai_review_key(key: KeyEvent, app: &mut App) {
    // Check if delete confirm is active and handle it first (before state borrow)
    if app
        .ai_state
        .as_ref()
        .is_some_and(|s| s.delete_confirm.is_some())
    {
        let confirmed = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
        let cancelled = matches!(
            key.code,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q')
        );

        if confirmed {
            let (paths, permanent, total_size) = {
                let s = app.ai_state.as_ref().unwrap();
                let total_size: u64 = s
                    .mark_for_delete
                    .iter()
                    .filter_map(|&i| s.results.get(i))
                    .map(|r| r.size)
                    .sum();
                let (paths, permanent) = s.delete_confirm.as_ref().unwrap().clone();
                (paths, permanent, total_size)
            };
            {
                let s = app.ai_state.as_mut().unwrap();
                s.delete_confirm = None;
            }

            let mut errors: Vec<String> = Vec::new();
            for path in &paths {
                let result = if permanent {
                    if path.is_dir() {
                        std::fs::remove_dir_all(path)
                    } else {
                        std::fs::remove_file(path)
                    }
                    .map_err(|e| e.to_string())
                } else {
                    trash::delete(path).map_err(|e| e.to_string())
                };
                if let Err(e) = result {
                    errors.push(format!("{}: {}", path.display(), e));
                }
                let _ = crate::tree_ops::apply_deletion_to_state(app, path);
            }

            app.deleted_bytes = app.deleted_bytes.saturating_add(total_size);
            app.load_current_children();

            // Remove deleted items from results
            if let Some(ref mut s) = app.ai_state {
                s.results.retain(|r| !paths.iter().any(|p| p == &r.path));
                s.mark_for_delete.clear();
                if s.cursor >= s.results.len() {
                    s.cursor = s.results.len().saturating_sub(1);
                }
                if s.results.is_empty() {
                    app.exit_ai_review();
                }
            }

            if !errors.is_empty() {
                app.set_error(
                    format!("{} delete(s) failed: {}", errors.len(), errors.join("; ")),
                    5,
                );
            } else {
                app.set_info(format!("deleted {} item(s)", paths.len()), 3);
            }
        } else if cancelled {
            if let Some(ref mut s) = app.ai_state {
                s.delete_confirm = None;
            }
        }
        return;
    }

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
            if state.status != AiStatus::Ready {
                return;
            }
            let paths = collect_marked_paths(state);
            if paths.is_empty() {
                return;
            }
            state.delete_confirm = Some((paths, false));
        }
        KeyCode::Char('x') => {
            if state.status != AiStatus::Ready || state.results.is_empty() {
                return;
            }
            let (path, path_str, should_exit) = {
                let path = state.results[state.cursor].path.clone();
                let path_str = path.to_string_lossy().to_string();
                state.results.remove(state.cursor);
                if state.cursor >= state.results.len() && !state.results.is_empty() {
                    state.cursor = state.results.len() - 1;
                }
                (path, path_str, state.results.is_empty())
            };

            app.ai_analyzed.remove(&path);
            app.ai_cache.remove(&path);
            if let Ok(conn) = argus_core::open_db(&argus_core::default_db_path()) {
                let _ = argus_core::delete_ai_analysis(&conn, &path_str);
            }
            app.set_info(format!("AI analysis data deleted: {}", path_str), 3);
            if should_exit {
                app.exit_ai_review();
            }
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
