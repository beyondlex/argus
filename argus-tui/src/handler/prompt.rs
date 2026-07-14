use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, AppMode};

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
    F: Fn(&Path) -> Result<String, String>,
{
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            let is_batch = !app.delete_target_paths.is_empty();
            if is_batch {
                // Batch delete
                let paths = std::mem::take(&mut app.delete_target_paths);
                let mut total_freed = 0u64;
                let mut errors: Vec<String> = Vec::new();
                for path in &paths {
                    match delete_fn(path) {
                        Ok(_msg) => {
                            total_freed = total_freed
                                .saturating_add(crate::tree_ops::apply_deletion_to_state(app, path));
                        }
                        Err(e) => {
                            errors.push(format!("{}: {}", path.display(), e));
                        }
                    }
                }
                if !errors.is_empty() {
                    app.set_error(
                        format!("{} delete(s) failed: {}", errors.len(), errors.join("; ")),
                        5,
                    );
                } else {
                    app.set_error(format!("deleted {} item(s)", paths.len()), 3);
                }
                app.deleted_bytes = app.deleted_bytes.saturating_add(total_freed);
                app.update_tree_lines();
                app.exit_multi_select();
            } else if let Some(path) = app.delete_target_path.clone() {
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
            }
            app.delete_target_path = None;
            app.delete_target_paths.clear();
            app.mode = AppMode::Browsing;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.delete_target_path = None;
            app.delete_target_paths.clear();
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
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
