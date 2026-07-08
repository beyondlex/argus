use crossterm::event::{KeyCode, KeyEvent};
use std::sync::atomic::Ordering;

use crate::app::{AppMode, App, Focus};

/// Handle keyboard events
pub fn handle_key(key: KeyEvent, app: &mut App) {
    match app.mode {
        AppMode::Browsing => handle_browsing_key(key, app),
        AppMode::DeletePrompt => handle_delete_prompt_key(key, app),
        AppMode::Help => handle_help_key(key, app),
    }
}

fn handle_browsing_key(key: KeyEvent, app: &mut App) {
    // Handle scan input prompt first
    if app.scan_prompt_open {
        match key.code {
            KeyCode::Enter => {
                let path = if app.scan_path_input.trim().is_empty() {
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
                } else {
                    std::path::PathBuf::from(app.scan_path_input.trim())
                };
                app.scan_path_input.clear();
                app.scan_prompt_open = false;
                start_scan(app, path);
            }
            KeyCode::Esc => {
                app.scan_path_input.clear();
                app.scan_prompt_open = false;
            }
            KeyCode::Char(c) => {
                app.scan_path_input.push(c);
            }
            KeyCode::Backspace => {
                app.scan_path_input.pop();
            }
            _ => {}
        }
        return;
    }

    // Handle scanning state
    if app.scanning {
        if key.code == KeyCode::Esc {
            app.cancel_scan.store(true, Ordering::Relaxed);
        }
        return;
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            move_cursor(app, 1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            move_cursor(app, -1);
        }
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
            expand_node(app);
        }
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
            collapse_node(app);
        }
        KeyCode::Char('s') => {
            app.scan_prompt_open = true;
            app.scan_path_input = String::new();
        }
        KeyCode::Char('o') => {
            app.sort_mode = app.sort_mode.toggle();
            app.update_tree_lines();
        }
        KeyCode::Char('d') => {
            if let Some(line) = app.selected_line() {
                if line.node.is_dir() && line.node.name() == app.current_root_path.as_ref().and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string())).as_deref().unwrap_or("") {
                    // Root node - cannot delete root
                    app.last_error = Some("cannot delete root directory".into());
                    app.error_clear_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                } else if let Some(full_path) = app.selected_node_full_path() {
                    if crate::util::is_protected_path(&full_path) {
                        app.last_error = Some("protected path, cannot delete".into());
                        app.error_clear_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                    } else {
                        app.delete_target_path = Some(full_path);
                        app.mode = AppMode::DeletePrompt;
                    }
                }
            }
        }
        KeyCode::Tab => {
            app.focus = match app.focus {
                Focus::Tree => Focus::FilterBar,
                Focus::FilterBar => Focus::Tree,
            };
        }
        KeyCode::Char('?') => {
            app.mode = AppMode::Help;
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            app.should_quit = true;
        }
        KeyCode::Char('1') => {
            // Quick select first snapshot as from
            if !app.available_snapshots.is_empty() && app.focus == Focus::FilterBar {
                app.filter_state.from_idx = Some(0);
                trigger_diff_if_ready(app);
            }
        }
        KeyCode::Char('2') => {
            // Quick select last snapshot as to
            if app.available_snapshots.len() > 1 && app.focus == Focus::FilterBar {
                app.filter_state.to_idx = Some(app.available_snapshots.len() - 1);
                trigger_diff_if_ready(app);
            }
        }
        _ => {}
    }
}

fn handle_delete_prompt_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(path) = app.delete_target_path.clone() {
                match trash::delete(&path) {
                    Ok(_) => {
                        app.last_error = Some(format!("deleted: {}", path.display()));
                        app.error_clear_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                    }
                    Err(e) => {
                        app.last_error = Some(format!("delete failed: {}", e));
                        app.error_clear_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
                    }
                }
            }
            app.delete_target_path = None;
            app.mode = AppMode::Browsing;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.delete_target_path = None;
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

fn handle_help_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('?') | KeyCode::Esc => {
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

fn move_cursor(app: &mut App, delta: isize) {
    if app.tree_lines.is_empty() {
        return;
    }
    let new_cursor = app.cursor as isize + delta;
    if new_cursor < 0 {
        app.cursor = 0;
    } else if new_cursor >= app.tree_lines.len() as isize {
        app.cursor = app.tree_lines.len() - 1;
    } else {
        app.cursor = new_cursor as usize;
    }
}

fn expand_node(app: &mut App) {
    let Some(line) = app.selected_line().cloned() else { return };
    if !line.node.is_dir() {
        return;
    }

    let path_key = line.node.name().to_string();

    if app.expanded.contains(&path_key) {
        // Already expanded, try to enter first child
        if app.cursor + 1 < app.tree_lines.len() {
            app.cursor += 1;
        }
    } else {
        // Expand this node
        app.expanded.insert(path_key);
        app.update_tree_lines();
    }
}

fn collapse_node(app: &mut App) {
    let Some(line) = app.selected_line().cloned() else { return };
    let path_key = line.node.name().to_string();

    if line.node.is_dir() && line.expanded {
        // Collapse this node
        app.expanded.remove(&path_key);
        app.update_tree_lines();
    } else if line.depth > 0 {
        // Go to parent: find the first line with depth-1 before cursor
        if app.cursor > 0 {
            let target_depth = line.depth.saturating_sub(1);
            for i in (0..app.cursor).rev() {
                if app.tree_lines[i].depth == target_depth {
                    app.cursor = i;
                    return;
                }
            }
        }
    }
}

fn start_scan(app: &mut App, path: std::path::PathBuf) {
    app.scanning = true;
    app.scan_progress = None;
    app.cancel_scan = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel = app.cancel_scan.clone();
    let tx = app.tx.clone();

    tokio::spawn(async move {
        let (progress_tx, progress_rx) = std::sync::mpsc::channel::<argus_core::scanner::ProgressUpdate>();

        // Forward progress from std channel to tokio channel
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            while let Ok(update) = progress_rx.recv() {
                let _ = tx_clone
                    .send(crate::app::AppMessage::ScanProgress {
                        file_count: update.file_count,
                        total_bytes: update.total_bytes,
                    })
                    .await;
            }
        });

        match argus_core::scan_path(&path, &cancel, Some(progress_tx)) {
            Ok(snapshot) => {
                // Save snapshot to disk
                let snapshots_dir = crate::util::default_snapshots_dir();
                let _ = std::fs::create_dir_all(&snapshots_dir);
                let filename = format!(
                    "{}_{}.json",
                    argus_core::hash_root_path(&path),
                    snapshot.timestamp.format("%Y-%m-%dT%H:%M:%SZ")
                );
                let filepath = snapshots_dir.join(&filename);
                if let Ok(json) = serde_json::to_string_pretty(&snapshot) {
                    let _ = std::fs::write(&filepath, &json);
                }

                let _ = tx.send(crate::app::AppMessage::ScanComplete(snapshot)).await;
            }
            Err(e) => {
                let _ = tx
                    .send(crate::app::AppMessage::Error(format!("scan failed: {}", e)))
                    .await;
            }
        }
    });
}

fn trigger_diff_if_ready(app: &mut App) {
    if !app.filter_state.should_diff() {
        return;
    }

    let from_idx = match app.filter_state.from_idx {
        Some(i) => i,
        None => return,
    };
    let to_idx = match app.filter_state.to_idx {
        Some(i) => i,
        None => return,
    };

    if from_idx >= app.available_snapshots.len() || to_idx >= app.available_snapshots.len() {
        return;
    }

    let from_info = &app.available_snapshots[from_idx];
    let to_info = &app.available_snapshots[to_idx];

    let from_content = match std::fs::read_to_string(&from_info.path) {
        Ok(c) => c,
        Err(e) => {
            app.last_error = Some(format!("failed to read snapshot: {}", e));
            return;
        }
    };
    let to_content = match std::fs::read_to_string(&to_info.path) {
        Ok(c) => c,
        Err(e) => {
            app.last_error = Some(format!("failed to read snapshot: {}", e));
            return;
        }
    };

    let old_snap: argus_core::Snapshot = match serde_json::from_str(&from_content) {
        Ok(s) => s,
        Err(e) => {
            app.last_error = Some(format!("failed to parse snapshot: {}", e));
            return;
        }
    };
    let new_snap: argus_core::Snapshot = match serde_json::from_str(&to_content) {
        Ok(s) => s,
        Err(e) => {
            app.last_error = Some(format!("failed to parse snapshot: {}", e));
            return;
        }
    };

    // Run diff in background
    let tx = app.tx.clone();
    tokio::spawn(async move {
        match argus_core::compare_trees(&old_snap, &new_snap) {
            Ok(diff) => {
                let _ = tx.send(crate::app::AppMessage::DiffComplete(diff)).await;
            }
            Err(e) => {
                let _ = tx
                    .send(crate::app::AppMessage::Error(format!("diff failed: {}", e)))
                    .await;
            }
        }
    });
}
