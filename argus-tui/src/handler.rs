use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use argus_core::{FileNode, NodeIndex, Snapshot, ROOT_NODE};

use crate::app::{App, AppMessage, AppMode, FilterFocus, FilterMode, Focus, TreeNode};
use crate::event::SHOULD_QUIT;
use crate::ipc_client::IpcClient;

/// Handle keyboard events
pub fn handle_key(key: KeyEvent, app: &mut App) {
    match app.mode {
        AppMode::Browsing => handle_browsing_key(key, app),
        AppMode::DeletePrompt => handle_delete_prompt_key(key, app),
        AppMode::DeletePermanentPrompt => handle_delete_permanent_prompt_key(key, app),
        AppMode::Help => handle_help_key(key, app),
        AppMode::Command => handle_command_key(key, app),
    }
}

fn handle_browsing_key(key: KeyEvent, app: &mut App) {
    if app.scanning {
        if key.code == KeyCode::Esc {
            app.cancel_scan.store(true, Ordering::Relaxed);
        }
        return;
    }

    // Filter pane has focus — handle its keys
    if app.focus == Focus::FilterPane {
        handle_filter_pane_key(key, app);
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
        KeyCode::Char('c') => {
            app.clear_filter_pane();
        }
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
            if !app.filtered_tree_lines.is_empty() {
                app.cursor = app.filtered_tree_lines.len() - 1;
            }
            app.pending_gg = false;
        }
        KeyCode::Char('l') | KeyCode::Right => {
            expand_node(app);
        }
        KeyCode::Char('h') | KeyCode::Left => {
            collapse_or_navigate_up(app);
        }
        KeyCode::Char('H') => {
            collapse_all_children(app);
        }
        KeyCode::Char('u') => {
            navigate_up_root(app);
        }
        KeyCode::Enter if !app.filter_word.is_empty() => {
            app.filter_mode = FilterMode::Input;
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
                    app.set_error("cannot delete root directory".into(), 3);
                } else if let Some(full_path) = app.selected_node_full_path() {
                    if crate::util::is_protected_path(&full_path) {
                        app.set_error("protected path, cannot delete".into(), 3);
                    } else {
                        app.delete_target_path = Some(full_path);
                        app.mode = AppMode::DeletePrompt;
                    }
                }
            }
        }
        KeyCode::Char('D') => {
            if let Some(line) = app.selected_line() {
                let root_name = app
                    .view_root_path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if line.node.is_dir() && line.node.name() == root_name {
                    app.set_error("cannot delete root directory".into(), 3);
                } else if let Some(full_path) = app.selected_node_full_path() {
                    if crate::util::is_protected_path(&full_path) {
                        app.set_error("protected path, cannot delete".into(), 3);
                    } else {
                        app.delete_target_path = Some(full_path);
                        app.mode = AppMode::DeletePermanentPrompt;
                    }
                }
            }
        }

        KeyCode::Char('?') => {
            app.mode = AppMode::Help;
        }
        KeyCode::Char(':') => {
            app.mode = AppMode::Command;
            app.command_input.clear();
            app.command_selected = 0;
            app.update_command_matches();
        }
        KeyCode::Char('t') if app.server_mode => {
            let next = (app.time_preset + 1) % crate::app::TIME_PRESET_COUNT;
            app.set_time_preset(next);
            app.set_error(format!("time range: {}", App::time_preset_label(next)), 2);
            app.request_delta_refresh();
        }
        KeyCode::Char('R') if !app.server_mode => {
            let path = crate::config::TuiConfig::default().daemon.uds_path.clone();
            let path_clone = path.clone();
            let tx = app.tx.clone();
            tokio::spawn(async move {
                if let Ok(mut client) = IpcClient::connect(&path_clone).await {
                    if client.ping().await.is_ok() {
                        let _ = tx.send(AppMessage::DaemonConnected(client)).await;
                        return;
                    }
                }
                let _ = tx
                    .send(AppMessage::Error("daemon reconnect failed".into()))
                    .await;
            });
        }
        KeyCode::Char('i') => {
            if let Some(path) = app.selected_node_full_path() {
                match std::fs::metadata(&path) {
                    Ok(meta) => {
                        app.info_data = Some((path, meta));
                    }
                    Err(e) => {
                        app.set_error(format!("stat failed: {}", e), 3);
                    }
                }
            }
        }
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Char('f') if app.server_mode => {
            app.focus = Focus::FilterPane;
            app.filter_focus = FilterFocus::TimePreset;
        }
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
            app.should_quit = true;
        }
        KeyCode::Esc => {
            app.info_data = None;
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
                        app.set_error(format!("deleted: {}", path.display()), 3);
                        apply_deletion_to_state(app, &path);
                        app.update_tree_lines();
                    }
                    Err(e) => {
                        app.set_error(format!("delete failed: {}", e), 5);
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

fn handle_delete_permanent_prompt_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(path) = app.delete_target_path.clone() {
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                match result {
                    Ok(_) => {
                        app.set_error(format!("permanently deleted: {}", path.display()), 3);
                        apply_deletion_to_state(app, &path);
                        app.update_tree_lines();
                    }
                    Err(e) => {
                        app.set_error(format!("delete failed: {}", e), 5);
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

fn handle_command_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char(c) if app.command_input.len() < 200 => {
            app.command_input.push(c);
            app.update_command_matches();
        }
        KeyCode::Backspace => {
            app.command_input.pop();
            app.update_command_matches();
        }
        KeyCode::Tab if !app.command_matches.is_empty() => {
            app.command_selected = (app.command_selected + 1) % app.command_matches.len();
        }
        KeyCode::BackTab if !app.command_matches.is_empty() => {
            app.command_selected = if app.command_selected == 0 {
                app.command_matches.len() - 1
            } else {
                app.command_selected - 1
            };
        }
        KeyCode::Enter => {
            let cmd = if !app.command_matches.is_empty() && app.command_input.is_empty() {
                app.command_matches[app.command_selected].to_string()
            } else {
                app.command_input.clone()
            };
            app.mode = AppMode::Browsing;
            execute_command(app, &cmd);
        }
        KeyCode::Esc => {
            app.command_input.clear();
            app.command_matches.clear();
            app.command_selected = 0;
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

fn execute_command(app: &mut App, cmd: &str) {
    let cmd = cmd.trim();

    if cmd.eq_ignore_ascii_case("Scan") {
        app.command_input.clear();
        app.command_matches.clear();
        app.command_selected = 0;
        start_scan(app);
        return;
    }

    if cmd.eq_ignore_ascii_case("Consolidate") {
        app.command_input.clear();
        app.command_matches.clear();
        app.command_selected = 0;
        if app.server_mode {
            let uds_path = app
                .daemon_client
                .as_ref()
                .map(|_| crate::config::TuiConfig::default().daemon.uds_path.clone())
                .unwrap_or_default();
            let tx = app.tx.clone();
            tokio::spawn(async move {
                match IpcClient::connect(&uds_path).await {
                    Ok(mut client) => match client.request_consolidation().await {
                        Ok(count) => {
                            let _ = tx
                                .send(AppMessage::Info(format!("consolidated {count} events")))
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(AppMessage::Info(format!("consolidation failed: {e}")))
                                .await;
                        }
                    },
                    Err(e) => {
                        let _ = tx
                            .send(AppMessage::Info(format!("daemon connect failed: {e}")))
                            .await;
                    }
                }
            });
        } else {
            app.set_error("not in server mode".into(), 3);
        }
        return;
    }

    match app.execute_command(cmd) {
        Ok(msg) => {
            app.set_error(msg, 3);
        }
        Err(e) => {
            app.set_error(e, 4);
        }
    }
    app.command_input.clear();
    app.command_matches.clear();
    app.command_selected = 0;
}

// ── Filter pane key handling ─────────────────────────────────────────────────

fn handle_filter_pane_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Tab | KeyCode::Char('\t') => {
            let next = match app.filter_focus {
                FilterFocus::TimePreset => FilterFocus::DeltaValue,
                FilterFocus::DeltaValue => FilterFocus::DeltaUnit,
                FilterFocus::DeltaUnit => FilterFocus::TimePreset,
            };
            app.filter_focus = next;
        }
        KeyCode::BackTab => {
            let next = match app.filter_focus {
                FilterFocus::TimePreset => FilterFocus::DeltaUnit,
                FilterFocus::DeltaValue => FilterFocus::TimePreset,
                FilterFocus::DeltaUnit => FilterFocus::DeltaValue,
            };
            app.filter_focus = next;
        }
        KeyCode::Char(ch) if ch.is_ascii_digit() && app.filter_focus == FilterFocus::DeltaValue => {
            let digit = (ch as u8 - b'0') as u64;
            if app.delta_filter_active {
                app.delta_filter_value = app
                    .delta_filter_value
                    .saturating_mul(10)
                    .saturating_add(digit);
            } else {
                app.delta_filter_active = true;
                app.delta_filter_value = digit;
            }
            app.refresh_filtered_lines();
        }
        KeyCode::Backspace
            if app.filter_focus == FilterFocus::DeltaValue && app.delta_filter_active =>
        {
            app.delta_filter_value /= 10;
            if app.delta_filter_value == 0 {
                app.delta_filter_active = false;
            }
            app.refresh_filtered_lines();
        }
        KeyCode::Char('j') | KeyCode::Down => match app.filter_focus {
            FilterFocus::TimePreset => {
                let next = (app.time_preset + 1) % crate::app::TIME_PRESET_COUNT;
                let label = App::time_preset_label(next);
                crate::app::log_msg(
                    &app.log_path,
                    &format!("filter: j key, preset={next} ({label})"),
                );
                app.set_time_preset(next);
                app.request_delta_refresh();
            }
            FilterFocus::DeltaValue => {
                app.delta_filter_active = true;
                app.delta_filter_inc();
                app.refresh_filtered_lines();
            }
            FilterFocus::DeltaUnit => {
                app.delta_filter_active = true;
                app.delta_filter_cycle_unit();
                app.refresh_filtered_lines();
            }
        },
        KeyCode::Char('k') | KeyCode::Up => match app.filter_focus {
            FilterFocus::TimePreset => {
                let count = crate::app::TIME_PRESET_COUNT;
                let next = (app.time_preset + count - 1) % count;
                let label = App::time_preset_label(next);
                crate::app::log_msg(
                    &app.log_path,
                    &format!("filter: k key, preset={next} ({label})"),
                );
                app.set_time_preset(next);
                app.request_delta_refresh();
            }
            FilterFocus::DeltaValue => {
                app.delta_filter_active = true;
                app.delta_filter_dec();
                app.refresh_filtered_lines();
            }
            FilterFocus::DeltaUnit => {
                app.delta_filter_active = true;
                app.delta_filter_unit = (app.delta_filter_unit + 2) % 3;
                app.refresh_filtered_lines();
            }
        },
        KeyCode::Char('c') => {
            app.clear_filter_pane();
        }
        KeyCode::Enter => {
            if app.filter_focus == FilterFocus::DeltaValue && app.delta_filter_active {
                app.refresh_filtered_lines();
            }
            app.focus = Focus::Tree;
        }
        KeyCode::Esc => {
            app.focus = Focus::Tree;
        }
        _ => {}
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

fn move_cursor(app: &mut App, delta: isize) {
    if app.filtered_tree_lines.is_empty() {
        return;
    }
    let new_cursor = app.cursor as isize + delta;
    if new_cursor < 0 {
        app.cursor = 0;
    } else if new_cursor >= app.filtered_tree_lines.len() as isize {
        app.cursor = app.filtered_tree_lines.len() - 1;
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

    let path_key = match app.tree_line_relative_path(app.cursor) {
        Some(path) => path,
        None => return,
    };

    if line.expanded {
        // Already expanded, move to first child
        if app.cursor + 1 < app.tree_lines.len() {
            app.cursor += 1;
        }
        return;
    }

    // Lazy-load children from disk when the tree node has no children.
    let needs_listing = match &app.tree_root {
        Some(TreeNode::Snapshot(snap_arc, root_idx)) => find_node(snap_arc, *root_idx, &path_key)
            .map(|found_idx| {
                let node = snap_arc.node(found_idx);
                node.children.is_empty()
            })
            .unwrap_or(false),
        _ => false,
    };

    if needs_listing {
        if let Some(dir_path) = app.selected_node_full_path() {
            match argus_core::list_dir(&dir_path) {
                Ok(listed) => {
                    // Pre-compute scanned sizes before mutating the tree
                    let mut enrich: HashMap<String, u64> = HashMap::new();
                    if let Some(TreeNode::Snapshot(snap_arc, root_idx)) = &app.tree_root {
                        let root_scan_tree =
                            crate::app::resolve_scan_tree(&app.scan_cache, &app.view_root_path);
                        for (name, child_idx) in &listed.node(ROOT_NODE).children {
                            if listed.node(*child_idx).is_dir {
                                let mut child_path = path_key.clone();
                                child_path.push(name.clone());
                                let scan_full_path = dir_path.join(name);
                                let from_cache = app
                                    .scan_cache
                                    .get(&scan_full_path)
                                    .map(|s| s.node(ROOT_NODE).size);
                                if let Some(val) = from_cache {
                                    enrich.insert(name.clone(), val);
                                } else if let Some(scanned_idx) = root_scan_tree
                                    .and_then(|(tree, idx)| find_node(tree, idx, &child_path))
                                {
                                    let (tree, _) = root_scan_tree.unwrap();
                                    enrich.insert(name.clone(), tree.node(scanned_idx).size);
                                } else if let Some(found_idx) =
                                    find_node(snap_arc, *root_idx, &child_path)
                                {
                                    enrich.insert(name.clone(), snap_arc.node(found_idx).size);
                                }
                            }
                        }
                    }

                    // Merge listed children into the tree root's arena
                    if let Some(TreeNode::Snapshot(snap_arc, root_idx)) = &mut app.tree_root {
                        let snap = Arc::make_mut(snap_arc);
                        if let Some(target_idx) = find_node(snap, *root_idx, &path_key) {
                            let child_nodes: Vec<(String, FileNode)> = listed
                                .node(ROOT_NODE)
                                .children
                                .iter()
                                .map(|(name, idx)| (name.clone(), listed.node(*idx).clone()))
                                .collect();

                            for (name, node) in child_nodes {
                                let new_idx = snap.arena.len() as NodeIndex;
                                snap.arena.push(node);
                                snap.node_mut(target_idx)
                                    .children
                                    .push((name.clone(), new_idx));
                                if let Some(&size) = enrich.get(&name) {
                                    snap.node_mut(new_idx).size = size;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    app.set_error(format!("cannot list directory: {}", e), 3);
                }
            }
        }
    }

    app.expanded.insert(path_key);
    app.update_tree_lines();
}

fn find_node(snap: &Snapshot, idx: NodeIndex, target_path: &[String]) -> Option<NodeIndex> {
    let node = snap.node(idx);
    let (head, tail) = target_path.split_first()?;
    if node.name != *head {
        return None;
    }
    if tail.is_empty() {
        return Some(idx);
    }
    let child_idx = node.child_idx(&tail[0])?;
    find_node(snap, child_idx, tail)
}

fn apply_deletion_to_state(app: &mut App, deleted_path: &Path) {
    let mut keys_to_remove = Vec::new();

    for key in app.scan_cache.keys() {
        if deleted_path.starts_with(key) || key.starts_with(deleted_path) {
            keys_to_remove.push(key.clone());
        }
    }

    for key in keys_to_remove {
        if key == app.view_root_path {
            if let Some(snapshot) = app.scan_cache.get_mut(&key) {
                remove_path_from_snapshot(snapshot, deleted_path);
            }
        } else {
            app.scan_cache.remove(&key);
        }
    }

    if let Some(crate::app::TreeNode::Snapshot(snap_arc, _)) = &mut app.tree_root {
        let snap = Arc::make_mut(snap_arc);
        let _ = remove_path_from_tree(snap, &app.view_root_path, deleted_path);
    }
}

fn remove_path_from_snapshot(snapshot: &mut Snapshot, deleted_path: &Path) -> bool {
    let Ok(relative) = deleted_path.strip_prefix(&snapshot.root_path) else {
        return false;
    };
    let components: Vec<String> = relative
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(String::from))
        .collect();
    if components.is_empty() {
        return false;
    }
    let removed = prune_file_node(snapshot, ROOT_NODE, &components, 0);
    if removed {
        snapshot.total_size = snapshot.node(ROOT_NODE).size;
    }
    removed
}

fn remove_path_from_tree(snap: &mut Snapshot, root_path: &Path, deleted_path: &Path) -> bool {
    let Ok(relative) = deleted_path.strip_prefix(root_path) else {
        return false;
    };
    let components: Vec<String> = relative
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(String::from))
        .collect();
    if components.is_empty() {
        return false;
    }
    prune_file_node(snap, ROOT_NODE, &components, 0)
}

fn prune_file_node(
    snap: &mut Snapshot,
    current_idx: NodeIndex,
    components: &[String],
    index: usize,
) -> bool {
    if index >= components.len() {
        return false;
    }

    let removed = if index + 1 == components.len() {
        let node = snap.node_mut(current_idx);
        let pos = node
            .children
            .iter()
            .position(|(n, _)| n == &components[index]);
        pos.map(|p| node.children.swap_remove(p)).is_some()
    } else if let Some(child_idx) = snap.node(current_idx).child_idx(&components[index]) {
        let removed = prune_file_node(snap, child_idx, components, index + 1);
        if removed {
            recompute_file_node_size(snap, current_idx);
        }
        removed
    } else {
        false
    };

    if removed {
        recompute_file_node_size(snap, current_idx);
    }
    removed
}

fn recompute_file_node_size(snap: &mut Snapshot, idx: NodeIndex) -> u64 {
    if snap.node(idx).children.is_empty() {
        return snap.node(idx).size;
    }

    let children: Vec<NodeIndex> = snap
        .node(idx)
        .children
        .iter()
        .map(|(_, idx)| *idx)
        .collect();
    let mut total = 0u64;
    for child_idx in children {
        total = total.saturating_add(recompute_file_node_size(snap, child_idx));
    }
    snap.node_mut(idx).size = total;
    total
}

fn jump_to_next_match(app: &mut App, delta: isize) {
    if SHOULD_QUIT.load(Ordering::Relaxed) {
        app.should_quit = true;
        return;
    }
    if app.match_indices.is_empty() {
        return;
    }
    let Some(current_path) = app.tree_line_relative_path(app.cursor) else {
        return;
    };
    let Some(anchor_walk_idx) = app.get_walk_idx(&current_path) else {
        return;
    };

    let new_idx = if delta >= 0 {
        next_match_index(&app.match_indices, anchor_walk_idx).unwrap_or(0)
    } else {
        prev_match_index(&app.match_indices, anchor_walk_idx).unwrap_or(app.match_indices.len() - 1)
    };

    app.current_match = new_idx;
    let target_path = app.match_indices[new_idx].path.clone();

    let newly_expanded = if target_path.len() > 1 {
        expand_ancestor_prefixes(&mut app.expanded, &target_path[..target_path.len() - 1])
    } else {
        Vec::new()
    };

    if !newly_expanded.is_empty() {
        // Incrementally expand each newly visible directory (shallowest first)
        for path in &newly_expanded {
            app.expand_path_in_tree(path);
        }
    }

    // Find the exact match by relative path, not just name.
    if let Some(pos) = app
        .tree_lines
        .iter()
        .position(|line| line.path == target_path)
    {
        // Map tree_lines position to filtered view position
        app.cursor = app
            .filtered_tree_lines
            .iter()
            .position(|&i| i == pos)
            .unwrap_or(0);
    }
}

fn next_match_index(matches: &[crate::app::SearchMatch], anchor_walk_idx: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let idx = matches.binary_search_by_key(&anchor_walk_idx, |m| m.walk_idx);
    let start = match idx {
        Ok(i) => i + 1,
        Err(i) => i,
    };
    if start < matches.len() {
        Some(start)
    } else {
        Some(0)
    }
}

fn prev_match_index(matches: &[crate::app::SearchMatch], anchor_walk_idx: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let idx = matches.binary_search_by_key(&anchor_walk_idx, |m| m.walk_idx);
    let end = match idx {
        Ok(i) => i,
        Err(i) => i,
    };
    if end > 0 {
        Some(end - 1)
    } else {
        Some(matches.len() - 1)
    }
}

fn collapse_or_navigate_up(app: &mut App) {
    let Some(line) = app.selected_line().cloned() else {
        return;
    };

    // At root (depth 0): nothing to collapse, no parent within tree
    if line.depth == 0 {
        return;
    }

    let path_key = match app.tree_line_relative_path(app.cursor) {
        Some(path) => path,
        None => return,
    };

    if line.node.is_dir() && line.expanded {
        // Collapse this node
        app.expanded.remove(&path_key);
        app.update_tree_lines();
    } else {
        // Go to parent: find first line with depth-1 before cursor
        if app.cursor > 0 {
            let target_depth = line.depth.saturating_sub(1);
            for i in (0..app.cursor).rev() {
                let actual_idx = app.filtered_tree_lines.get(i).copied().unwrap_or(0);
                if app.tree_lines.get(actual_idx).map(|l| l.depth) == Some(target_depth) {
                    app.cursor = i;
                    return;
                }
            }
        }
    }
}

fn collapse_all_children(app: &mut App) {
    // Remove all expanded paths deeper than root (length > 1).
    // Root (depth 0) is always expanded by the flatten logic.
    app.expanded.retain(|p| p.len() <= 1);
    app.update_tree_lines();

    // Snap cursor to root if it was on a now-hidden child
    if app.cursor >= app.tree_lines.len() {
        app.cursor = 0;
    }
}

fn navigate_up_root(app: &mut App) {
    if let Some(parent) = app.view_root_path.parent() {
        if parent != app.view_root_path {
            app.view_root_path = parent.to_path_buf();
            app.rebuild_tree();
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
    app.scan_spinner = 0;
    app.scan_spinner_tick = Instant::now();
    app.scan_started_at = Some(Instant::now());
    app.cancel_scan = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel = app.cancel_scan.clone();
    let tx = app.tx.clone();
    let path = app.view_root_path.clone();
    let scan_skip_dirs: Vec<String> = app.config.browsing.skip_dirs.clone();

    tokio::task::spawn_blocking(move || {
        let (progress_tx, progress_rx) =
            std::sync::mpsc::channel::<argus_core::scanner::ProgressUpdate>();

        // Forward progress updates from blocking scan to UI via dedicated thread
        let tx_clone = tx.clone();
        std::thread::spawn(move || {
            while let Ok(update) = progress_rx.recv() {
                let _ = tx_clone.blocking_send(AppMessage::ScanProgress {
                    file_count: update.file_count,
                    total_bytes: update.total_bytes,
                });
            }
        });

        match argus_core::scan_path(&path, &cancel, Some(progress_tx), &scan_skip_dirs) {
            Ok(snapshot) => {
                let _ = tx.blocking_send(AppMessage::ScanComplete(snapshot));
            }
            Err(e) => {
                let _ = tx.blocking_send(AppMessage::Error(format!("scan failed: {}", e)));
            }
        }
    });
}

fn expand_ancestor_prefixes(
    expanded: &mut std::collections::HashSet<Vec<String>>,
    path: &[String],
) -> Vec<Vec<String>> {
    let mut expanded_paths = Vec::new();
    if path.len() <= 1 {
        return expanded_paths;
    }

    for len in 2..=path.len() {
        let ancestor = path[..len].to_vec();
        if expanded.insert(ancestor.clone()) {
            expanded_paths.push(ancestor);
        }
    }
    expanded_paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TreeNode;
    use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn file_node(name: &str, size: u64) -> FileNode {
        FileNode {
            name: name.to_string(),
            parent: None,
            is_dir: false,
            file_type: FileType::File,
            size,
            children: Vec::new(),
        }
    }

    fn dir_node(name: &str, children: Vec<(&str, NodeIndex)>) -> FileNode {
        FileNode {
            name: name.to_string(),
            parent: None,
            is_dir: true,
            file_type: FileType::Directory,
            size: 0,
            children: children
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    fn make_app(snap: Snapshot, scan_snap: Snapshot) -> App {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/test");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.scan_cache.insert(PathBuf::from("/tmp/test"), scan_snap);
        app.update_tree_lines();
        app
    }

    #[test]
    fn test_jump_to_next_match_uses_full_path() {
        let root_arena = vec![
            dir_node("test", vec![("a", 1), ("b", 2)]),
            dir_node("a", vec![("target", 3)]),
            dir_node("b", vec![("target", 4)]),
            file_node("target", 1),
            file_node("target", 1),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), root_arena, 2);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);

        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["test".to_string(), "a".to_string()]);
        app.expanded
            .insert(vec!["test".to_string(), "b".to_string()]);
        app.update_tree_lines();
        app.filter_word = "target".to_string();
        app.recompute_matches();

        assert_eq!(app.match_indices.len(), 2);
        app.cursor = 2;
        app.current_match = 1;

        jump_to_next_match(&mut app, 1);

        let selected = app.selected_line().expect("cursor should point to a line");
        assert_eq!(selected.node.name(), "target");
        assert_eq!(app.cursor, 4);
        assert_eq!(
            app.match_indices[0].path,
            vec!["test".to_string(), "a".to_string(), "target".to_string()]
        );
        assert_eq!(
            app.match_indices[1].path,
            vec!["test".to_string(), "b".to_string(), "target".to_string()]
        );
        assert_eq!(
            app.selected_node_full_path().expect("selected path"),
            std::path::PathBuf::from("/tmp/test/b/target")
        );
    }

    #[test]
    fn test_expanded_is_path_scoped() {
        let root_arena = vec![
            dir_node("root", vec![("left", 1), ("right", 2)]),
            dir_node("left", vec![("common", 3)]),
            dir_node("right", vec![("common", 4)]),
            dir_node("common", vec![("l.txt", 5)]),
            dir_node("common", vec![("r.txt", 6)]),
            file_node("l.txt", 1),
            file_node("r.txt", 1),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/root"), root_arena, 2);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/root"), vec![file_node("root", 0)], 0);

        let mut app = make_app(snap, scan_snap);
        app.view_root_path = PathBuf::from("/tmp/root");
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["root".to_string(), "left".to_string()]);
        app.expanded.insert(vec![
            "root".to_string(),
            "left".to_string(),
            "common".to_string(),
        ]);
        app.update_tree_lines();

        let visible_paths: Vec<Vec<String>> = app
            .tree_lines
            .iter()
            .enumerate()
            .filter_map(|(idx, _)| app.tree_line_relative_path(idx))
            .collect();

        assert!(visible_paths.contains(&vec![
            "root".to_string(),
            "left".to_string(),
            "common".to_string()
        ]));
        assert!(!visible_paths.contains(&vec![
            "root".to_string(),
            "right".to_string(),
            "common".to_string()
        ]));
    }

    #[test]
    fn test_expand_node_keeps_regular_dirs_marked_with_metadata() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("root");
        fs::create_dir_all(root_path.join("sub")).unwrap();
        fs::write(root_path.join("sub").join("file.txt"), "data").unwrap();

        // Root node in arena with a metadata-less child
        let root_arena = vec![
            FileNode {
                name: "root".to_string(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 0,
                children: [("sub".to_string(), 1)].into_iter().collect(),
            },
            FileNode {
                name: "sub".to_string(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 0,
                children: Vec::new(),
            },
        ];
        let snap = Snapshot::new(root_path.clone(), root_arena, 0);
        let scan_snap = Snapshot::new(root_path.clone(), vec![dir_node("root", vec![])], 0);

        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = root_path.clone();
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.scan_cache.insert(root_path.clone(), scan_snap);
        app.update_tree_lines();
        app.cursor = 1;

        expand_node(&mut app);

        let TreeNode::Snapshot(snap_arc, _) = app.tree_root.as_ref().unwrap();
        let sub = snap_arc.node(1);
        assert!(sub.is_dir);
    }

    #[test]
    fn test_delete_updates_parent_sizes_and_scan_cache() {
        fn sized_file(name: &str, size: u64) -> FileNode {
            FileNode {
                name: name.to_string(),
                parent: None,
                is_dir: false,
                file_type: FileType::File,
                size,
                children: Vec::new(),
            }
        }

        // Arena: test/ignore/{keep.bin, delete.bin}
        let arena = vec![
            dir_node("test", vec![("ignore", 1)]),
            dir_node("ignore", vec![("keep.bin", 2), ("delete.bin", 3)]),
            sized_file("keep.bin", 12),
            sized_file("delete.bin", 10),
        ];
        let root_snapshot = Snapshot::new(PathBuf::from("/tmp/test"), arena.clone(), 22);

        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 22);
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/test");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.scan_cache
            .insert(PathBuf::from("/tmp/test"), root_snapshot);
        app.update_tree_lines();

        apply_deletion_to_state(&mut app, Path::new("/tmp/test/ignore/delete.bin"));
        app.update_tree_lines();

        let TreeNode::Snapshot(snap_arc, _) = app.tree_root.as_ref().unwrap();
        let ignore = snap_arc.node(1);
        assert_eq!(ignore.size, 12);
        assert_eq!(snap_arc.node(ROOT_NODE).size, 12);

        let cached = app.scan_cache.get(&PathBuf::from("/tmp/test")).unwrap();
        assert_eq!(cached.node(ROOT_NODE).size, 12);
        let cached_ignore = cached.node(1);
        assert_eq!(cached_ignore.size, 12);
        assert!(!cached_ignore
            .children
            .iter()
            .any(|(n, _)| n == "delete.bin"));
    }
}
