use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::app::{App, AppMessage, AppMode, FilterMode, TreeNode};

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

    // Lazy-load children from disk when the tree node has none, or when
    // all existing children are structural placeholders from a shallow
    // scan (they are the only nodes with has_metadata == false).
    let needs_listing = match &app.tree_root {
        Some(TreeNode::Snapshot(root)) => find_node(root, &path_key)
            .map(|n| n.children.is_empty() || n.children.values().all(|c| !c.has_metadata))
            .unwrap_or(false),
        _ => false,
    };

    if needs_listing {
        if let Some(dir_path) = app.selected_node_full_path() {
            match argus_core::list_dir(&dir_path) {
                Ok(listed) => {
                    // Pre-compute scanned sizes from root snapshot for dir
                    // children before mutating the tree.
                    let mut enrich: HashMap<String, (u64, bool)> = HashMap::new();
                    if let Some(TreeNode::Snapshot(ref root)) = app.tree_root {
                        let root_scan_tree =
                            crate::app::resolve_scan_tree(&app.scan_cache, &app.view_root_path);
                        for (name, child) in &listed.children {
                            if child.is_dir {
                                let mut child_path = path_key.clone();
                                child_path.push(name.clone());
                                let scan_full_path = dir_path.join(name);
                                let from_cache = app
                                    .scan_cache
                                    .get(&scan_full_path)
                                    .map(|s| (s.root_node.size, s.root_node.has_metadata));
                                if let Some(val) = from_cache {
                                    enrich.insert(name.clone(), val);
                                } else if let Some(scanned) =
                                    root_scan_tree.and_then(|tree| find_node(tree, &child_path))
                                {
                                    enrich
                                        .insert(name.clone(), (scanned.size, scanned.has_metadata));
                                } else if let Some(scanned) = find_node(root, &child_path) {
                                    enrich
                                        .insert(name.clone(), (scanned.size, scanned.has_metadata));
                                }
                            }
                        }
                    }
                    // Now apply the listing and enrichment
                    if let Some(TreeNode::Snapshot(ref mut file_node)) = app.tree_root {
                        if let Some(target) = find_node_mut(file_node, &path_key) {
                            target.children = listed.children;
                            target.has_metadata = true;
                            for child in target.children.values_mut() {
                                if let Some(&(size, meta)) = enrich.get(&child.name) {
                                    child.size = size;
                                    child.has_metadata = meta;
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

fn find_node<'a>(
    node: &'a argus_core::FileNode,
    target_path: &[String],
) -> Option<&'a argus_core::FileNode> {
    let (head, tail) = target_path.split_first()?;
    if node.name != *head {
        return None;
    }
    if tail.is_empty() {
        return Some(node);
    }
    let child = node.children.get(&tail[0])?;
    find_node(child, tail)
}

fn find_node_mut<'a>(
    node: &'a mut argus_core::FileNode,
    target_path: &[String],
) -> Option<&'a mut argus_core::FileNode> {
    let (head, tail) = target_path.split_first()?;
    if node.name != *head {
        return None;
    }
    if tail.is_empty() {
        return Some(node);
    }

    let child = node.children.get_mut(&tail[0])?;
    find_node_mut(child, tail)
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

    if let Some(crate::app::TreeNode::Snapshot(ref mut root)) = app.tree_root {
        let _ = remove_path_from_tree(root, &app.view_root_path, deleted_path);
    }
}

fn remove_path_from_snapshot(snapshot: &mut argus_core::Snapshot, deleted_path: &Path) -> bool {
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
    let removed = prune_file_node(&mut snapshot.root_node, &components, 0);
    if removed {
        snapshot.total_size = snapshot.root_node.size;
    }
    removed
}

fn remove_path_from_tree(
    root: &mut argus_core::FileNode,
    root_path: &Path,
    deleted_path: &Path,
) -> bool {
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
    prune_file_node(root, &components, 0)
}

fn prune_file_node(
    current: &mut argus_core::FileNode,
    components: &[String],
    index: usize,
) -> bool {
    if index >= components.len() {
        return false;
    }

    let removed = if index + 1 == components.len() {
        current.children.remove(&components[index]).is_some()
    } else if let Some(child) = current.children.get_mut(&components[index]) {
        let removed = prune_file_node(child, components, index + 1);
        if removed {
            recompute_file_node_size(child);
        }
        removed
    } else {
        false
    };

    if removed {
        recompute_file_node_size(current);
    }
    removed
}

fn recompute_file_node_size(node: &mut argus_core::FileNode) -> u64 {
    if node.children.is_empty() {
        return node.size;
    }

    if node.children.values().all(|c| !c.has_metadata) {
        return node.size;
    }

    let mut total = 0u64;
    for child in node.children.values_mut() {
        total = total.saturating_add(recompute_file_node_size(child));
    }
    node.size = total;
    total
}

fn jump_to_next_match(app: &mut App, delta: isize) {
    if app.match_indices.is_empty() {
        return;
    }
    let Some(current_path) = app.tree_line_relative_path(app.cursor) else {
        return;
    };
    let Some(anchor_walk_idx) = current_cursor_walk_index(app, &current_path) else {
        return;
    };

    let new_idx = if delta >= 0 {
        next_match_index(&app.match_indices, anchor_walk_idx).unwrap_or(0)
    } else {
        prev_match_index(&app.match_indices, anchor_walk_idx).unwrap_or(app.match_indices.len() - 1)
    };

    app.current_match = new_idx;
    let sm = app.match_indices[new_idx].clone();
    let target_path = sm.path.clone();

    if target_path.len() > 1 {
        expand_ancestor_prefixes(&mut app.expanded, &target_path[..target_path.len() - 1]);
    }

    app.update_tree_lines();

    // Find the exact match by relative path, not just name.
    if let Some(pos) = app
        .tree_lines
        .iter()
        .enumerate()
        .find(|(idx, _)| app.tree_line_relative_path(*idx) == Some(target_path.clone()))
        .map(|(idx, _)| idx)
    {
        app.cursor = pos;
    }
}

fn next_match_index(matches: &[crate::app::SearchMatch], anchor_walk_idx: usize) -> Option<usize> {
    matches
        .iter()
        .position(|m| m.walk_idx > anchor_walk_idx)
        .or_else(|| (!matches.is_empty()).then_some(0))
}

fn prev_match_index(matches: &[crate::app::SearchMatch], anchor_walk_idx: usize) -> Option<usize> {
    matches
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| m.walk_idx < anchor_walk_idx)
        .map(|(idx, _)| idx)
        .or_else(|| (!matches.is_empty()).then_some(matches.len() - 1))
}

fn current_cursor_walk_index(app: &App, target_path: &[String]) -> Option<usize> {
    let root = match &app.tree_root {
        Some(crate::app::TreeNode::Snapshot(root)) => root,
        _ => return None,
    };

    let mut path = vec![root.name.clone()];
    let mut walk_idx = 0usize;
    walk_index_for_path(root, &mut path, target_path, app.sort_mode, &mut walk_idx)
}

fn walk_index_for_path(
    node: &argus_core::FileNode,
    path: &mut Vec<String>,
    target_path: &[String],
    sort_mode: crate::app::SortMode,
    walk_idx: &mut usize,
) -> Option<usize> {
    if path.as_slice() == target_path {
        return Some(*walk_idx);
    }

    *walk_idx += 1;

    if !node.is_dir {
        return None;
    }

    let mut children: Vec<&argus_core::FileNode> = node.children.values().collect();
    match sort_mode {
        crate::app::SortMode::Name => children.sort_by(|a, b| a.name.cmp(&b.name)),
        crate::app::SortMode::Size => children.sort_by_key(|b| std::cmp::Reverse(b.size)),
    }

    for child in children {
        path.push(child.name.clone());
        if let Some(idx) = walk_index_for_path(child, path, target_path, sort_mode, walk_idx) {
            return Some(idx);
        }
        path.pop();
    }

    None
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
                if app.tree_lines[i].depth == target_depth {
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
    let db_path = app.db_path.clone();
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
                // Write to SQLite
                if let Ok(mut conn) = argus_core::open_db(&db_path) {
                    let _ = argus_core::write_scan(&mut conn, &snapshot);
                }

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
) {
    if path.len() <= 1 {
        return;
    }

    for len in 2..=path.len() {
        expanded.insert(path[..len].to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TreeNode;
    use argus_core::{FileNode, FileType, Snapshot};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn make_file(name: &str) -> FileNode {
        FileNode {
            name: name.to_string(),
            is_dir: false,
            file_type: FileType::File,
            size: 1,
            modified: None,
            created: None,
            inode: None,
            device: None,
            has_metadata: true,
            children: HashMap::new(),
        }
    }

    fn make_dir(name: &str, children: Vec<FileNode>) -> FileNode {
        let mut map = HashMap::new();
        for child in children {
            map.insert(child.name.clone(), child);
        }

        FileNode {
            name: name.to_string(),
            is_dir: true,
            file_type: FileType::Directory,
            size: map.values().map(|child| child.size).sum(),
            modified: None,
            created: None,
            inode: None,
            device: None,
            has_metadata: true,
            children: map,
        }
    }

    fn make_app(root: FileNode) -> App {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(
            crate::config::TuiConfig::default(),
            PathBuf::from("/tmp/argus_test.db"),
            tx,
            rx,
        );
        app.view_root_path = std::path::PathBuf::from("/tmp/test");
        let snapshot = Snapshot::new(std::path::PathBuf::from("/tmp/test"), root, 1);
        app.tree_root = Some(TreeNode::Snapshot(snapshot.root_node.clone()));
        app.scan_cache
            .insert(std::path::PathBuf::from("/tmp/test"), snapshot);
        app.update_tree_lines();
        app
    }

    #[test]
    fn test_jump_to_next_match_uses_full_path() {
        // Root node name must match the last component of view_root_path (/tmp/test)
        let root = make_dir(
            "test",
            vec![
                make_dir("a", vec![make_file("target")]),
                make_dir("b", vec![make_file("target")]),
            ],
        );

        let mut app = make_app(root);
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
        let root = make_dir(
            "root",
            vec![
                make_dir("left", vec![make_dir("common", vec![make_file("l.txt")])]),
                make_dir("right", vec![make_dir("common", vec![make_file("r.txt")])]),
            ],
        );

        let mut app = make_app(root);
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

        let mut children = HashMap::new();
        children.insert(
            "sub".to_string(),
            FileNode {
                name: "sub".to_string(),
                is_dir: true,
                file_type: FileType::Directory,
                size: 0,
                modified: None,
                created: None,
                inode: None,
                device: None,
                has_metadata: false,
                children: HashMap::new(),
            },
        );

        let root = FileNode {
            name: "root".to_string(),
            is_dir: true,
            file_type: FileType::Directory,
            size: 0,
            modified: None,
            created: None,
            inode: None,
            device: None,
            has_metadata: true,
            children,
        };

        let mut app = make_app(root);
        app.view_root_path = root_path;
        app.update_tree_lines();
        app.cursor = 1;

        expand_node(&mut app);

        let TreeNode::Snapshot(root_node) = app.tree_root.as_ref().unwrap() else {
            unreachable!();
        };
        let sub = root_node.children.get("sub").unwrap();
        assert!(sub.has_metadata);
        assert!(sub.is_dir);
    }

    #[test]
    fn test_delete_updates_parent_sizes_and_scan_cache() {
        fn sized_file(name: &str, size: u64) -> FileNode {
            FileNode {
                name: name.to_string(),
                is_dir: false,
                file_type: FileType::File,
                size,
                modified: None,
                created: None,
                inode: None,
                device: None,
                has_metadata: true,
                children: HashMap::new(),
            }
        }

        let ignore_dir = make_dir(
            "ignore",
            vec![sized_file("keep.bin", 12), sized_file("delete.bin", 10)],
        );
        let root = make_dir("test", vec![ignore_dir]);
        let root_snapshot = Snapshot::new(PathBuf::from("/tmp/test"), root.clone(), 22);

        let mut app = make_app(root);
        app.tree_root = Some(TreeNode::Snapshot(root_snapshot.root_node.clone()));
        app.scan_cache
            .insert(PathBuf::from("/tmp/test"), root_snapshot);
        app.update_tree_lines();

        apply_deletion_to_state(&mut app, Path::new("/tmp/test/ignore/delete.bin"));
        app.update_tree_lines();

        let TreeNode::Snapshot(root_node) = app.tree_root.as_ref().unwrap() else {
            unreachable!();
        };
        let ignore = root_node.children.get("ignore").unwrap();
        assert_eq!(ignore.size, 12);
        assert_eq!(root_node.size, 12);

        let cached = app.scan_cache.get(&PathBuf::from("/tmp/test")).unwrap();
        assert_eq!(cached.root_node.size, 12);
        let cached_ignore = cached.root_node.children.get("ignore").unwrap();
        assert_eq!(cached_ignore.size, 12);
        assert!(!cached_ignore.children.contains_key("delete.bin"));
    }
}
