use crate::app::{App, AppMessage};
use crate::types::UninstallPhase;
use crossterm::event::{KeyCode, KeyEvent};
use std::path::Path;

pub(crate) fn handle_cleanup_key(key: KeyEvent, app: &mut App) {
    if app.cleanup_state.as_ref().is_some_and(|s| s.detail_pending) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('i') => {
                if let Some(ref mut s) = app.cleanup_state {
                    s.detail_pending = false;
                    s.detail_items = None;
                }
            }
            _ => {}
        }
        return;
    }

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
                s.cursor = s.cursor.saturating_add(1).min(item_count.saturating_sub(1));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut s) = app.cleanup_state {
                s.dry_run = false;
                s.cursor = s.cursor.saturating_sub(1);
            }
        }
        KeyCode::Char('g') => {
            if app.pending_gg {
                if let Some(ref mut s) = app.cleanup_state {
                    s.cursor = 0;
                }
                app.pending_gg = false;
            } else {
                app.pending_gg = true;
            }
        }
        KeyCode::Char('G') => {
            if let Some(ref mut s) = app.cleanup_state {
                s.cursor = item_count.saturating_sub(1);
            }
            app.pending_gg = false;
        }
        KeyCode::Char(' ') => {
            if let Some(ref mut s) = app.cleanup_state {
                if s.selected.contains(&s.cursor) {
                    s.selected.remove(&s.cursor);
                } else {
                    s.selected.insert(s.cursor);
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(ref mut s) = app.cleanup_state {
                s.dry_run = !s.dry_run;
            }
        }
        KeyCode::Char('i') => {
            let path = {
                let s = app.cleanup_state.as_ref().unwrap();
                s.items.get(s.cursor).map(|item| item.path.clone())
            };
            if let Some(path) = path {
                app.cleanup_state.as_mut().unwrap().detail_pending = true;
                let tx = app.tx.clone();
                std::thread::spawn(move || {
                    let details = scan_dir_details(&path);
                    let _ = tx.blocking_send(AppMessage::CleanupDetailReady(details));
                });
            }
        }
        KeyCode::Enter => {
            if let Some(ref mut s) = app.cleanup_state {
                if !s.selected.is_empty() {
                    s.confirm_pending = true;
                }
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
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
    let is_filter_mode = app.uninstall_state.as_ref().is_some_and(|s| s.filter_mode);

    if is_filter_mode {
        match key.code {
            KeyCode::Esc => {
                if let Some(ref mut s) = app.uninstall_state {
                    s.filter_mode = false;
                }
            }
            KeyCode::Char(c) => {
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
                }
            }
            _ => {}
        }
        return;
    }

    let item_count = {
        let s = app.uninstall_state.as_ref().unwrap();
        s.filtered.len()
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = s.cursor.saturating_add(1).min(item_count.saturating_sub(1));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = s.cursor.saturating_sub(1);
            }
        }
        KeyCode::Char('g') => {
            if app.pending_gg {
                if let Some(ref mut s) = app.uninstall_state {
                    s.cursor = 0;
                }
                app.pending_gg = false;
            } else {
                app.pending_gg = true;
            }
        }
        KeyCode::Char('G') => {
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = item_count.saturating_sub(1);
            }
            app.pending_gg = false;
        }
        KeyCode::Char('/') => {
            if let Some(ref mut s) = app.uninstall_state {
                s.filter_mode = true;
            }
        }
        KeyCode::Char('o') => {
            if let Some(ref mut s) = app.uninstall_state {
                s.sort_mode = (s.sort_mode + 1) % 3;
                match s.sort_mode {
                    0 => s.apps.sort_by(|a, b| b.size.cmp(&a.size)),
                    1 => s.apps.sort_by(|a, b| {
                        let a_t = a.last_used.unwrap_or_default();
                        let b_t = b.last_used.unwrap_or_default();
                        b_t.cmp(&a_t)
                    }),
                    _ => s.apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                }
                s.filtered = (0..s.apps.len()).collect();
                s.cursor = 0;
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
            }
        }
        KeyCode::Enter => {
            let (selected_idx, _search_word) = {
                let s = app.uninstall_state.as_ref().unwrap();
                (s.filtered.get(s.cursor).copied(), s.search_word.clone())
            };
            if let Some(idx) = selected_idx {
                if let Some(ref mut s) = app.uninstall_state {
                    s.selected_app = Some(idx);
                    s.phase = UninstallPhase::Confirm;
                    s.scanning = true;
                    s.cursor = 0;
                }
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
        KeyCode::Esc | KeyCode::Char('q') => {
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
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = s.cursor.saturating_add(1).min(leftover_count.saturating_sub(1));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = s.cursor.saturating_sub(1);
            }
        }
        KeyCode::Char(' ') => {
            if let Some(ref mut s) = app.uninstall_state {
                if s.selected_leftovers.contains(&s.cursor) {
                    s.selected_leftovers.remove(&s.cursor);
                } else {
                    s.selected_leftovers.insert(s.cursor);
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
            }
        }
        _ => {}
    }
}

fn scan_dir_details(path: &Path) -> Vec<(String, u64)> {
    let mut entries = Vec::new();
    if !path.is_dir() {
        if let Ok(meta) = path.metadata() {
            entries.push((
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                meta.len(),
            ));
        }
        return entries;
    }
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let p = entry.path();
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                let size = dir_total_size(&p);
                entries.push((
                    p.strip_prefix(path)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .to_string(),
                    size,
                ));
            } else if ft.is_file() {
                let size = match entry.metadata() {
                    Ok(m) => m.len(),
                    Err(_) => 0,
                };
                entries.push((
                    p.strip_prefix(path)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .to_string(),
                    size,
                ));
            }
        }
    }
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.truncate(200);
    entries
}

fn dir_total_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                dirs.push(entry.path());
            } else if ft.is_file() {
                total += match entry.metadata() {
                    Ok(m) => m.len(),
                    Err(_) => 0,
                };
            }
        }
    }
    total
}
