use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc;

use crate::app::{App, AppMessage, AppMode};

pub(crate) fn handle_delete_prompt_key(key: KeyEvent, app: &mut App) {
    handle_delete_common(key, app, |path| {
        trash::delete(path).map_err(|e| e.to_string())?;
        Ok(format!("deleted: {}", path.display()))
    });
}

pub(crate) fn handle_delete_permanent_prompt_key(key: KeyEvent, app: &mut App) {
    handle_delete_common(key, app, |path| {
        if path.is_dir() {
            std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        } else {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(format!("permanently deleted: {}", path.display()))
    });
}

pub(crate) fn handle_delete_common<F>(key: KeyEvent, app: &mut App, delete_fn: F)
where
    F: Fn(&Path) -> Result<String, String> + Send + 'static,
{
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            let is_batch = !app.delete_target_paths.is_empty();
            if is_batch {
                let paths: Vec<PathBuf> = std::mem::take(&mut app.delete_target_paths);
                let total = paths.len() as u64;
                let tx = app.tx.clone();
                app.deleting = true;
                app.delete_progress = Some((0, total));
                app.delete_permanent = matches!(app.mode, AppMode::DeletePermanentPrompt);
                app.mode = AppMode::Deleting;

                tokio::task::spawn_blocking(move || {
                    let mut errors: Vec<String> = Vec::new();
                    for (i, path) in paths.iter().enumerate() {
                        if let Err(e) = delete_fn(path) {
                            errors.push(format!("{}: {}", path.display(), e));
                        }
                        let _ = tx.blocking_send(AppMessage::DeleteProgress {
                            current: (i + 1) as u64,
                            total,
                        });
                    }
                    let _ = tx.blocking_send(AppMessage::DeleteComplete { errors, paths });
                });
            } else if let Some(path) = app.delete_target_path.clone() {
                if path.is_dir() {
                    let tx = app.tx.clone();
                    let permanent = matches!(app.mode, AppMode::DeletePermanentPrompt);
                    app.deleting = true;
                    app.delete_progress = Some((0, 1));
                    app.delete_permanent = permanent;
                    app.mode = AppMode::Deleting;

                    tokio::task::spawn_blocking(move || {
                        let errors = delete_dir_progressive(&path, permanent, &tx);
                        let _ = tx.blocking_send(AppMessage::DeleteComplete {
                            errors,
                            paths: vec![path],
                        });
                    });
                } else {
                    match delete_fn(&path) {
                        Ok(msg) => {
                            app.set_error(msg, 3);
                            let freed = crate::tree_ops::apply_deletion_to_state(app, &path);
                            app.deleted_bytes = app.deleted_bytes.saturating_add(freed);
                            app.update_tree_lines();
                        }
                        Err(e) => {
                            app.set_error(format!("delete failed: {}", e), 5);
                        }
                    }
                    app.delete_target_path = None;
                    app.mode = AppMode::Browsing;
                }
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.delete_target_path = None;
            app.delete_target_paths.clear();
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

fn collect_items(path: &Path, items: &mut Vec<PathBuf>) {
    items.push(path.to_path_buf());
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            collect_items(&entry.path(), items);
        }
    }
}

fn delete_dir_progressive(
    path: &Path,
    permanent: bool,
    tx: &mpsc::Sender<AppMessage>,
) -> Vec<String> {
    let mut items = Vec::new();
    collect_items(path, &mut items);

    items.sort_by(|a, b| {
        let depth_cmp = b.components().count().cmp(&a.components().count());
        if depth_cmp != std::cmp::Ordering::Equal {
            return depth_cmp;
        }
        let a_is_dir = a.is_dir();
        let b_is_dir = b.is_dir();
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Equal,
        }
    });

    let total = items.len() as u64;
    let _ = tx.blocking_send(AppMessage::DeleteProgress { current: 0, total });

    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let result = if permanent {
            if item.is_dir() {
                std::fs::remove_dir(item)
            } else {
                std::fs::remove_file(item)
            }
            .map_err(|e| e.to_string())
        } else {
            trash::delete(item).map_err(|e| e.to_string())
        };
        if let Err(e) = result {
            errors.push(format!("{}: {}", item.display(), e));
        }
        let _ = tx.blocking_send(AppMessage::DeleteProgress {
            current: (i + 1) as u64,
            total,
        });
    }
    errors
}

pub(crate) fn handle_help_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('?') | KeyCode::Esc => {
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

pub(crate) fn handle_time_help_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Browsing;
            app.time_help_scroll = 0;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.time_help_scroll = app.time_help_scroll.saturating_add(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.time_help_scroll = app.time_help_scroll.saturating_sub(1);
        }
        _ => {}
    }
}
