use crate::app::{App, AppMessage};
use crate::types::UninstallPhase;
use crossterm::event::{KeyCode, KeyEvent};

pub(crate) fn handle_cleanup_key(key: KeyEvent, app: &mut App) {
    if app.cleanup_state.as_ref().is_some_and(|s| s.confirm_pending) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let (items, selected, dry_run) = {
                    let s = app.cleanup_state.as_ref().unwrap();
                    (s.items.clone(), s.selected.clone(), s.dry_run)
                };
                let to_delete: Vec<argus_core::CleanItem> = items
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| selected.contains(i))
                    .map(|(_, item)| item)
                    .collect();
                let tx = app.tx.clone();
                app.cleanup_state.as_mut().unwrap().confirm_pending = false;
                std::thread::spawn(move || {
                    let report = argus_core::exec_clean(&to_delete, dry_run);
                    match report {
                        Ok(r) => {
                            let _ = tx.blocking_send(AppMessage::CleanupExecComplete(r));
                        }
                        Err(e) => {
                            let _ = tx.blocking_send(AppMessage::Error(format!("clean failed: {e}")));
                        }
                    }
                });
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                if let Some(ref mut s) = app.cleanup_state {
                    s.confirm_pending = false;
                }
            }
            _ => {}
        }
        return;
    }

    let Some(ref state) = app.cleanup_state else { return };

    if state.scanning {
        return;
    }

    let item_count = state.items.len();

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ref mut s) = app.cleanup_state {
                s.dry_run = false;
            }
            let cursor = app.cursor.saturating_add(1).min(item_count.saturating_sub(1));
            app.cursor = cursor;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut s) = app.cleanup_state {
                s.dry_run = false;
            }
            app.cursor = app.cursor.saturating_sub(1);
        }
        KeyCode::Char('g') => {
            if app.pending_gg {
                app.cursor = 0;
                app.pending_gg = false;
            } else {
                app.pending_gg = true;
            }
        }
        KeyCode::Char('G') => {
            app.cursor = item_count.saturating_sub(1);
            app.pending_gg = false;
        }
        KeyCode::Char(' ') => {
            if let Some(ref mut s) = app.cleanup_state {
                if s.selected.contains(&app.cursor) {
                    s.selected.remove(&app.cursor);
                } else {
                    s.selected.insert(app.cursor);
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(ref mut s) = app.cleanup_state {
                s.dry_run = !s.dry_run;
            }
        }
        KeyCode::Enter => {
            if let Some(ref mut s) = app.cleanup_state {
                if !s.selected.is_empty() {
                    s.confirm_pending = true;
                }
            }
        }
        KeyCode::Esc => {
            app.exit_cleanup();
        }
        _ => {}
    }
}

pub(crate) fn handle_uninstall_key(key: KeyEvent, app: &mut App) {
    if app.uninstall_state.as_ref().is_some_and(|s| s.confirm_pending) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let (app_info, remove_leftovers) = {
                    let s = app.uninstall_state.as_ref().unwrap();
                    let app_idx = s.selected_app.unwrap_or(0);
                    let app_info = s.apps.get(app_idx).cloned();
                    (app_info, s.remove_leftovers)
                };
                let Some(app_info) = app_info else { return };
                let tx = app.tx.clone();
                app.uninstall_state.as_mut().unwrap().confirm_pending = false;
                std::thread::spawn(move || {
                    let report = argus_core::uninstall_app(&app_info, remove_leftovers);
                    match report {
                        Ok(r) => {
                            let _ = tx.blocking_send(AppMessage::UninstallComplete(r));
                        }
                        Err(e) => {
                            let _ = tx.blocking_send(AppMessage::Error(format!("uninstall failed: {e}")));
                        }
                    }
                });
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                if let Some(ref mut s) = app.uninstall_state {
                    s.confirm_pending = false;
                }
            }
            _ => {}
        }
        return;
    }

    let Some(ref state) = app.uninstall_state else { return };

    if state.scanning {
        return;
    }

    match state.phase {
        UninstallPhase::SelectApp => handle_uninstall_select_app(key, app),
        UninstallPhase::Confirm => handle_uninstall_confirm(key, app),
    }
}

fn handle_uninstall_select_app(key: KeyEvent, app: &mut App) {
    let item_count = {
        let s = app.uninstall_state.as_ref().unwrap();
        s.filtered.len()
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.cursor = app.cursor.saturating_add(1).min(item_count.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.cursor = app.cursor.saturating_sub(1);
        }
        KeyCode::Char('g') => {
            if app.pending_gg {
                app.cursor = 0;
                app.pending_gg = false;
            } else {
                app.pending_gg = true;
            }
        }
        KeyCode::Char('G') => {
            app.cursor = item_count.saturating_sub(1);
            app.pending_gg = false;
        }
        KeyCode::Char('/') => {
            // Search input mode - could add, but for now simple filter
        }
        KeyCode::Char(c) => {
            // Simple incremental filter
            if let Some(ref mut s) = app.uninstall_state {
                s.search_word.push(c);
                s.filtered = s
                    .apps
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| {
                        a.name.to_lowercase().contains(&s.search_word.to_lowercase())
                    })
                    .map(|(i, _)| i)
                    .collect();
                s.cursor = 0;
                app.cursor = 0;
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut s) = app.uninstall_state {
                s.search_word.pop();
                s.filtered = s
                    .apps
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| {
                        a.name.to_lowercase().contains(&s.search_word.to_lowercase())
                    })
                    .map(|(i, _)| i)
                    .collect();
                s.cursor = 0;
                app.cursor = 0;
            }
        }
        KeyCode::Enter => {
            let selected_idx = {
                let s = app.uninstall_state.as_ref().unwrap();
                s.filtered.get(app.cursor).copied()
            };
            if let Some(idx) = selected_idx {
                if let Some(ref mut s) = app.uninstall_state {
                    s.selected_app = Some(idx);
                    s.phase = UninstallPhase::Confirm;
                    s.scanning = true;
                    s.cursor = 0;
                    app.cursor = 0;
                }
                // Spawn leftover scan
                let app_info = app.uninstall_state.as_ref().unwrap().apps[idx].clone();
                let tx = app.tx.clone();
                std::thread::spawn(move || {
                    match argus_core::find_leftovers(&app_info) {
                        Ok(leftovers) => {
                            let _ = tx.blocking_send(AppMessage::UninstallLeftoversReady(leftovers));
                        }
                        Err(e) => {
                            let _ = tx.blocking_send(AppMessage::Error(format!("leftover scan failed: {e}")));
                        }
                    }
                });
            }
        }
        KeyCode::Esc => {
            app.exit_uninstall();
        }
        _ => {}
    }
}

fn handle_uninstall_confirm(key: KeyEvent, app: &mut App) {
    let leftover_count = {
        let s = app.uninstall_state.as_ref().unwrap();
        s.leftovers.as_ref().map(|l| l.leftover_paths.len()).unwrap_or(0)
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.cursor = app.cursor.saturating_add(1).min(leftover_count.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.cursor = app.cursor.saturating_sub(1);
        }
        KeyCode::Char(' ') => {
            if let Some(ref mut s) = app.uninstall_state {
                if s.selected_leftovers.contains(&app.cursor) {
                    s.selected_leftovers.remove(&app.cursor);
                } else {
                    s.selected_leftovers.insert(app.cursor);
                }
            }
        }
        KeyCode::Char('t') | KeyCode::Tab => {
            if let Some(ref mut s) = app.uninstall_state {
                s.remove_leftovers = !s.remove_leftovers;
            }
        }
        KeyCode::Enter => {
            if let Some(ref mut s) = app.uninstall_state {
                s.confirm_pending = true;
            }
        }
        KeyCode::Esc => {
            if let Some(ref mut s) = app.uninstall_state {
                s.phase = UninstallPhase::SelectApp;
                s.leftovers = None;
                s.cursor = 0;
                app.cursor = 0;
            }
        }
        _ => {}
    }
}
