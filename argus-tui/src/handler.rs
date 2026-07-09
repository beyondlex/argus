use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::atomic::Ordering;

use crate::app::{App, AppMode, FilterMode, Focus};

/// Handle keyboard events
pub fn handle_key(key: KeyEvent, app: &mut App) {
    match app.mode {
        AppMode::Browsing => handle_browsing_key(key, app),
        AppMode::DeletePrompt => handle_delete_prompt_key(key, app),
        AppMode::Help => handle_help_key(key, app),
    }
}

fn handle_browsing_key(key: KeyEvent, app: &mut App) {
    if app.scanning {
        if key.code == KeyCode::Esc {
            app.cancel_scan.store(true, Ordering::Relaxed);
        }
        return;
    }

    match app.filter_mode {
        FilterMode::Input => {
            match key.code {
                KeyCode::Char(c) => {
                    app.filter_word.push(c);
                    app.recompute_matches();
                }
                KeyCode::Backspace => {
                    app.filter_word.pop();
                    app.recompute_matches();
                }
                KeyCode::Enter => {
                    if app.filter_word.is_empty() {
                        app.recompute_matches();
                        app.filter_mode = FilterMode::Inactive;
                    } else {
                        app.filter_mode = FilterMode::Active;
                    }
                }
                KeyCode::Esc => {
                    app.filter_word.clear();
                    app.recompute_matches();
                    app.filter_mode = FilterMode::Inactive;
                }
                _ => {}
            }
            return;
        }
        FilterMode::Active => {
            match key.code {
                KeyCode::Char('n') => {
                    jump_to_next_match(app, 1);
                }
                KeyCode::Char('N') => {
                    jump_to_next_match(app, -1);
                }
                KeyCode::Char('/') => {
                    app.filter_word.clear();
                    app.recompute_matches();
                    app.filter_mode = FilterMode::Input;
                }
                KeyCode::Esc => {
                    app.filter_word.clear();
                    app.recompute_matches();
                    app.filter_mode = FilterMode::Inactive;
                    return;
                }
                _ => {}
            }
            // Don't return for other keys — let navigation keys pass through
        }
        FilterMode::Inactive => {}
    }

    // Only '/' triggers filter from Inactive; other keys ignored if already handled above
    if app.filter_mode == FilterMode::Inactive {
        if let KeyCode::Char('/') = key.code {
            app.filter_mode = FilterMode::Input;
            return;
        }
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            move_cursor(app, 1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            move_cursor(app, -1);
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
            if !app.tree_lines.is_empty() {
                app.cursor = app.tree_lines.len() - 1;
            }
            app.pending_gg = false;
        }
        KeyCode::Char('l') | KeyCode::Right => {
            expand_node(app);
        }
        KeyCode::Char('h') | KeyCode::Left => {
            collapse_or_navigate_up(app);
        }
        KeyCode::Enter => {
            if !app.filter_word.is_empty() {
                app.filter_mode = FilterMode::Input;
            }
        }
        KeyCode::Char('s') => {
            start_scan(app);
        }
        KeyCode::Char('.') => {
            set_root_to_selected(app);
        }
        KeyCode::Char('o') => {
            app.sort_mode = app.sort_mode.toggle();
            app.update_tree_lines();
        }
        KeyCode::Char('d') => {
            if let Some(line) = app.selected_line() {
                let root_name = app
                    .view_root_path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if line.node.is_dir() && line.node.name() == root_name {
                    app.last_error = Some("cannot delete root directory".into());
                    app.error_clear_at =
                        Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                } else if let Some(full_path) = app.selected_node_full_path() {
                    if crate::util::is_protected_path(&full_path) {
                        app.last_error = Some("protected path, cannot delete".into());
                        app.error_clear_at =
                            Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
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
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
            app.should_quit = true;
        }
        KeyCode::Esc => {
            // Esc is used by filter/delete/help modes, not for quit
        }
        KeyCode::Char('1') => {
            if !app.available_snapshots.is_empty() && app.focus == Focus::FilterBar {
                app.filter_state.from_idx = Some(0);
                trigger_diff_if_ready(app);
            }
        }
        KeyCode::Char('2') => {
            if app.available_snapshots.len() > 1 && app.focus == Focus::FilterBar {
                app.filter_state.to_idx = Some(app.available_snapshots.len() - 1);
                trigger_diff_if_ready(app);
            }
        }
        _ => {}
    }
    // Reset gg double-tap on any key other than g
    if key.code != KeyCode::Char('g') {
        app.pending_gg = false;
    }
}

fn handle_delete_prompt_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(path) = app.delete_target_path.clone() {
                match trash::delete(&path) {
                    Ok(_) => {
                        app.last_error = Some(format!("deleted: {}", path.display()));
                        app.error_clear_at =
                            Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                    }
                    Err(e) => {
                        app.last_error = Some(format!("delete failed: {}", e));
                        app.error_clear_at =
                            Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
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
    let Some(line) = app.selected_line().cloned() else {
        return;
    };
    if !line.node.is_dir() {
        return;
    }

    let path_key = line.node.name().to_string();

    if app.expanded.contains(&path_key) {
        // Already expanded, move to first child
        if app.cursor + 1 < app.tree_lines.len() {
            app.cursor += 1;
        }
        return;
    }

    // If node lacks scan data and has no children, fetch from disk
    if !line.has_scan_data {
        if let Some(dir_path) = app.selected_node_full_path() {
            match argus_core::list_dir(&dir_path) {
                Ok(listed) => {
                    if let Some(crate::app::TreeNode::Snapshot(ref mut file_node)) = app.tree_root {
                        if let Some(target) = find_node_mut(file_node, &path_key) {
                            target.children = listed.children;
                        }
                    }
                }
                Err(e) => {
                    app.last_error = Some(format!("cannot list directory: {}", e));
                    app.error_clear_at =
                        Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                }
            }
        }
    }

    app.expanded.insert(path_key);
    app.update_tree_lines();
}

fn find_node_mut<'a>(
    node: &'a mut argus_core::FileNode,
    target_name: &str,
) -> Option<&'a mut argus_core::FileNode> {
    if node.name == target_name {
        return Some(node);
    }
    for child in node.children.values_mut() {
        if child.is_dir {
            if let Some(found) = find_node_mut(child, target_name) {
                return Some(found);
            }
        }
    }
    None
}

fn jump_to_next_match(app: &mut App, delta: isize) {
    if app.match_indices.is_empty() {
        return;
    }
    let len = app.match_indices.len();
    let new_idx = (app.current_match as isize + delta).rem_euclid(len as isize) as usize;
    app.current_match = new_idx;
    let sm = app.match_indices[new_idx].clone();

    // Expand ancestors from ancestor_path (for collapsed matches)
    for name in &sm.ancestor_path {
        app.expanded.insert(name.clone());
    }

    // Expand ancestors from tree_lines (for visible matches)
    if let Some(ti) = sm.tree_idx {
        if let Some(line) = app.tree_lines.get(ti).cloned() {
            let mut ancestors: Vec<String> = Vec::new();
            let mut target_depth = line.depth;
            for i in (0..ti).rev() {
                if let Some(l) = app.tree_lines.get(i) {
                    if l.depth < target_depth && l.depth > 0 {
                        ancestors.push(l.node.name().to_string());
                        target_depth = l.depth;
                    }
                }
            }
            for name in &ancestors {
                app.expanded.insert(name.clone());
            }
        }
    }

    app.update_tree_lines();

    // Find the match by name in rebuilt tree_lines (immune to stale indices)
    let target_name = sm.name;
    if let Some(pos) = app
        .tree_lines
        .iter()
        .position(|l| l.node.name() == target_name)
    {
        app.cursor = pos;
    }
}

fn collapse_or_navigate_up(app: &mut App) {
    let Some(line) = app.selected_line().cloned() else {
        return;
    };

    // Root node at depth 0: navigate to parent directory
    if line.depth == 0 {
        if let Some(parent) = app.view_root_path.parent() {
            if parent != app.view_root_path {
                app.view_root_path = parent.to_path_buf();
                app.rebuild_tree();
            }
        }
        return;
    }

    let path_key = line.node.name().to_string();

    if line.node.is_dir() && line.expanded {
        // Collapse this node
        app.expanded.remove(&path_key);
        app.update_tree_lines();
    } else {
        // Go to parent: find first line with depth-1 before cursor
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

pub fn set_root_to_selected(app: &mut App) {
    let Some(line) = app.selected_line().cloned() else {
        return;
    };
    if !line.node.is_dir() {
        return;
    }
    if let Some(full_path) = app.selected_node_full_path() {
        if full_path == app.view_root_path {
            return;
        }
        app.view_root_path = full_path;
        app.rebuild_tree();
    }
}

pub fn start_scan(app: &mut App) {
    app.scanning = true;
    app.scan_progress = None;
    app.cancel_scan = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel = app.cancel_scan.clone();
    let tx = app.tx.clone();
    let path = app.view_root_path.clone();

    tokio::spawn(async move {
        let (progress_tx, progress_rx) =
            std::sync::mpsc::channel::<argus_core::scanner::ProgressUpdate>();

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

                let _ = tx
                    .send(crate::app::AppMessage::ScanComplete(snapshot))
                    .await;
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
