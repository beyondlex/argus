use crate::app::{App, AppMessage, AppMode};
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
            let (paths, permanent) = {
                let s = app.ai_state.as_ref().unwrap();
                let (paths, permanent) = s.delete_confirm.as_ref().unwrap().clone();
                (paths, permanent)
            };
            {
                let s = app.ai_state.as_mut().unwrap();
                s.delete_confirm = None;
                // Deleting runs on a background thread (same as the browsing
                // prompt): the inline loop here used to freeze the UI for the
                // whole permanent removal of a large tree. [`AiStatus::Ready`]
                // is restored by the completion message, which also applies
                // the tree/state updates.
                s.status = AiStatus::Deleting;
            }

            let tx = app.tx.clone();
            std::thread::spawn(move || {
                let (errors, deleted_paths) = delete_marked(&paths, permanent);
                let _ = tx.blocking_send(AppMessage::AiDeleteComplete {
                    errors,
                    paths: deleted_paths,
                });
            });
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
        KeyCode::Char('j') | KeyCode::Down if state.cursor + 1 < state.results.len() => {
            let visible = review_visible_rows(crossterm::terminal::size().ok().map(|(_, h)| h));
            if state.cursor >= state.scroll_offset + visible - 1 {
                state.scroll_offset = state.cursor + 2 - visible;
            }
            state.cursor += 1;
        }
        KeyCode::Char('k') | KeyCode::Up if state.cursor > 0 => {
            state.cursor -= 1;
            if state.cursor < state.scroll_offset {
                state.scroll_offset = state.cursor;
            }
        }
        KeyCode::Char(' ') => {
            app.ai_review_toggle_mark();
        }
        KeyCode::Char('i') if state.status == AiStatus::Ready && !state.results.is_empty() => {
            state.info_item = Some(state.cursor);
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
    // Sorted: mark_for_delete is a HashSet, and arbitrary iteration order made
    // both the confirm dialog's row order and the deletion order nondeterministic.
    let mut paths: Vec<PathBuf> = state
        .mark_for_delete
        .iter()
        .filter_map(|&i| state.results.get(i))
        .map(|r| r.path.clone())
        .collect();
    paths.sort();
    paths
}

/// Delete the confirmed paths (runs on a background thread). Browsing-mode
/// deletes refuse protected paths before the prompt; this must not become a
/// side door around that gate. Returns the per-path errors (including
/// protected-path skips) and the paths that were actually removed — callers
/// may only prune state and count freed bytes for the removed ones.
fn delete_marked(paths: &[PathBuf], permanent: bool) -> (Vec<String>, Vec<PathBuf>) {
    let mut errors: Vec<String> = Vec::new();
    let mut deleted: Vec<PathBuf> = Vec::new();
    for path in paths {
        if crate::util::is_protected_path(path) {
            errors.push(format!("{}: protected path, skipped", path.display()));
            continue;
        }
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
        match result {
            Ok(()) => deleted.push(path.clone()),
            Err(e) => errors.push(format!("{}: {}", path.display(), e)),
        }
    }
    (errors, deleted)
}

/// How many result rows fit on screen. `.max(1)`: a four-row terminal yields
/// 0 and the `visible - 1` scroll math would underflow.
fn review_visible_rows(terminal_height: Option<u16>) -> usize {
    terminal_height
        .map(|h| (h as usize).saturating_sub(4) / ITEM_LINES)
        .unwrap_or(6)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny terminal (height ≤ 4) must clamp to one visible row: the raw
    /// computation yields 0 and `visible - 1` underflowed in the scroll math.
    #[test]
    fn test_review_visible_rows_never_zero() {
        assert_eq!(review_visible_rows(Some(4)), 1);
        assert_eq!(review_visible_rows(Some(0)), 1);
        assert_eq!(review_visible_rows(None), 6);
        assert_eq!(review_visible_rows(Some(24)), 5);
        assert_eq!(review_visible_rows(Some(u16::MAX)), 16_382);
    }

    /// Permanent deletion removes files and dirs and reports only the removed
    /// paths; a failure (missing path) lands in errors instead.
    #[test]
    fn test_delete_marked_permanent_removes_and_reports() {
        let tmp = tempfile::TempDir::new().unwrap();
        let file = tmp.path().join("gone.txt");
        std::fs::write(&file, "x").unwrap();
        let dir = tmp.path().join("gone_dir");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested").join("b.bin"), "y").unwrap();
        let missing = tmp.path().join("missing.bin");

        let (errors, deleted) = delete_marked(&[file.clone(), dir.clone(), missing], true);

        assert!(!file.exists());
        assert!(!dir.exists());
        assert_eq!(deleted, vec![file, dir]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("missing.bin"));
    }
}
