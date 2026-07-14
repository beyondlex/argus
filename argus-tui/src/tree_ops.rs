use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use argus_core::{FileNode, NodeIndex, Snapshot, ROOT_NODE};

use crate::app::{App, SortMode, TreeLine, TreeNode};

/// Directories with more children than this won't have their subtree matched or expanded
/// during search navigation. Prevents n/N from hanging when jumping into massive directories.
pub(crate) const MAX_DIR_CHILDREN: usize = 500;

pub fn expand_node(app: &mut App) {
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
        if app.cursor + 1 < app.tree_lines.len() {
            app.cursor += 1;
        }
        return;
    }

    let needs_listing = match &app.tree_root {
        Some(TreeNode::Snapshot(snap_arc, root_idx)) => snap_arc
            .find_node(*root_idx, &path_key)
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
                    let mut enrich: HashMap<String, u64> = HashMap::new();
                    if let Some(TreeNode::Snapshot(snap_arc, root_idx)) = &app.tree_root {
                        let root_scan_tree =
                            resolve_scan_tree(&app.scan_cache, &app.view_root_path);
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
                                    .and_then(|(tree, idx)| tree.find_node(idx, &child_path))
                                {
                                    let (tree, _) = root_scan_tree.unwrap();
                                    enrich.insert(name.clone(), tree.node(scanned_idx).size);
                                } else if let Some(found_idx) =
                                    snap_arc.find_node(*root_idx, &child_path)
                                {
                                    enrich.insert(name.clone(), snap_arc.node(found_idx).size);
                                }
                            }
                        }
                    }

                    if let Some(TreeNode::Snapshot(snap_arc, root_idx)) = &mut app.tree_root {
                        let snap = Arc::make_mut(snap_arc);
                        if let Some(target_idx) = snap.find_node(*root_idx, &path_key) {
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

    app.expanded.insert(path_key.clone());
    if !app.expand_path_in_tree(&path_key) {
        app.update_tree_lines();
    }
}

pub fn collapse_or_navigate_up(app: &mut App) {
    let Some(line) = app.selected_line().cloned() else {
        return;
    };

    if line.depth == 0 {
        return;
    }

    let path_key = match app.tree_line_relative_path(app.cursor) {
        Some(path) => path,
        None => return,
    };

    if line.node.is_dir() && line.expanded {
        app.expanded.remove(&path_key);
        app.update_tree_lines();
    } else {
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

pub fn collapse_all_children(app: &mut App) {
    app.expanded.retain(|p| p.len() <= 1);
    app.update_tree_lines();

    if app.cursor >= app.tree_lines.len() {
        app.cursor = 0;
    }
}

pub fn navigate_up_root(app: &mut App) {
    if let Some(parent) = app.view_root_path.parent() {
        if parent != app.view_root_path {
            app.view_root_path = parent.to_path_buf();
            app.rebuild_tree();
        }
    }
}

/// Get the size of a node at `deleted_path` within a snapshot, without modifying it.
fn node_size_in_snapshot(snapshot: &Snapshot, deleted_path: &Path) -> u64 {
    let Ok(relative) = deleted_path.strip_prefix(&snapshot.root_path) else {
        return 0;
    };
    let components: Vec<String> = relative
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(String::from))
        .collect();
    if components.is_empty() {
        return 0;
    }
    let mut idx = ROOT_NODE;
    for (step, comp) in components.iter().enumerate() {
        let node = snapshot.node(idx);
        if step + 1 == components.len() {
            return node
                .children
                .iter()
                .find(|(n, _)| n == comp)
                .map(|(_, child_idx)| snapshot.node(*child_idx).size)
                .unwrap_or(0);
        }
        match node.child_idx(comp) {
            Some(child_idx) => idx = child_idx,
            None => return 0,
        }
    }
    0
}

pub fn apply_deletion_to_state(app: &mut App, deleted_path: &Path) -> u64 {
    let mut total_freed = 0u64;
    let mut keys_to_remove = Vec::new();

    for key in app.scan_cache.keys() {
        if deleted_path.starts_with(key) || key.starts_with(deleted_path) {
            keys_to_remove.push(key.clone());
        }
    }

    for key in keys_to_remove {
        if key == app.view_root_path || deleted_path.starts_with(&key) {
            // Prune in-place: remove the deleted path from this snapshot.
            // This preserves parent/ancestor scans needed by resolve_scan_tree
            // when the view root is a subdirectory (not the scanned root).
            if let Some(arc) = app.scan_cache.get_mut(&key) {
                // COW: clone only if another Arc still shares this snapshot
                let snapshot = Arc::make_mut(arc);
                let freed = node_size_in_snapshot(snapshot, deleted_path);
                remove_path_from_snapshot(snapshot, deleted_path);
                total_freed = total_freed.saturating_add(freed);
            }
        } else {
            // key.starts_with(deleted_path) — descendant cache entry is being deleted
            if let Some(snapshot) = app.scan_cache.remove(&key) {
                total_freed = total_freed.saturating_add(snapshot.total_size);
            }
        }
    }

    if let Some(TreeNode::Snapshot(snap_arc, _)) = &mut app.tree_root {
        let snap = Arc::make_mut(snap_arc);
        let _ = remove_path_from_tree(snap, &app.view_root_path, deleted_path);
    }

    total_freed
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

// ── Tree flattening / size enrichment / sorting ──────────────────────────────

#[allow(clippy::too_many_arguments)]
pub(crate) fn flatten_snapshot_tree(
    snap_arc: &Arc<Snapshot>,
    idx: NodeIndex,
    depth: usize,
    expanded: &HashSet<Vec<String>>,
    sort_mode: SortMode,
    lines: &mut Vec<TreeLine>,
    scan_cache: &HashMap<PathBuf, Arc<Snapshot>>,
    view_root_path: &Path,
    root_scan_tree: Option<(&Snapshot, NodeIndex)>,
    delta_cache: Option<&HashMap<Vec<String>, i64>>,
    path: &mut Vec<String>,
    show_hidden: bool,
) {
    let node = snap_arc.node(idx);
    path.push(node.name.clone());
    // One clone only: into TreeLine.path. expanded / size_for_path use &path.
    let is_expanded = depth == 0 || expanded.contains(path);

    let node_has_scan = if node.is_dir {
        size_for_path(scan_cache, view_root_path, root_scan_tree, path).is_some()
    } else {
        false
    };

    lines.push(TreeLine {
        depth,
        node: TreeNode::Snapshot(Arc::clone(snap_arc), idx),
        expanded: is_expanded && node.is_dir && !node.children.is_empty(),
        has_scan_data: node_has_scan || !node.is_dir,
        path: path.clone(),
    });

    if is_expanded && node.is_dir {
        let mut children: Vec<(&String, NodeIndex)> =
            node.children.iter().map(|(n, i)| (n, *i)).collect();
        if !show_hidden {
            children.retain(|(name, _)| !name.starts_with('.'));
        }
        sort_children_snapshot(&mut children, snap_arc, sort_mode, path, delta_cache);
        for (_name, child_idx) in children {
            flatten_snapshot_tree(
                snap_arc,
                child_idx,
                depth + 1,
                expanded,
                sort_mode,
                lines,
                scan_cache,
                view_root_path,
                root_scan_tree,
                delta_cache,
                path,
                show_hidden,
            );
        }
    }

    path.pop();
}

pub(crate) fn enrich_snapshot_sizes(
    snap: &mut Snapshot,
    idx: NodeIndex,
    scan_cache: &HashMap<PathBuf, Arc<Snapshot>>,
    view_root_path: &Path,
    root_scan_tree: Option<(&Snapshot, NodeIndex)>,
    path: &mut Vec<String>,
) {
    let name = snap.node(idx).name.clone();
    path.push(name);

    if let Some(size) = size_for_path(scan_cache, view_root_path, root_scan_tree, path) {
        snap.node_mut(idx).size = size;
    }

    if snap.node(idx).is_dir {
        let children: Vec<NodeIndex> = snap
            .node(idx)
            .children
            .iter()
            .map(|(_, idx)| *idx)
            .collect();
        for child_idx in children {
            enrich_snapshot_sizes(
                snap,
                child_idx,
                scan_cache,
                view_root_path,
                root_scan_tree,
                path,
            );
        }
    }

    path.pop();
}

pub(crate) fn size_for_path(
    scan_cache: &HashMap<PathBuf, Arc<Snapshot>>,
    view_root_path: &Path,
    root_scan_tree: Option<(&Snapshot, NodeIndex)>,
    path_key: &[String],
) -> Option<u64> {
    if path_key.is_empty() {
        return None;
    }

    let mut path = view_root_path.to_path_buf();
    for component in path_key.iter().skip(1) {
        path.push(component);
    }

    if let Some(snapshot) = scan_cache.get(&path) {
        return Some(snapshot.node(ROOT_NODE).size);
    }

    root_scan_tree.and_then(|(snap, idx)| {
        snap.find_node(idx, path_key)
            .map(|found_idx| snap.node(found_idx).size)
    })
}

/// Find the best-available scan tree node for a view path.
///
/// First tries an exact match in scan_cache. If not found, walks up
/// the path hierarchy to find a parent-level scan, then walks down
/// the scan tree to find the subtree matching the view root.
pub(crate) fn resolve_scan_tree<'a>(
    scan_cache: &'a HashMap<PathBuf, Arc<Snapshot>>,
    view_root_path: &Path,
) -> Option<(&'a Snapshot, NodeIndex)> {
    if let Some(snapshot) = scan_cache.get(view_root_path) {
        return Some((snapshot.as_ref(), ROOT_NODE));
    }

    let mut parent = view_root_path.parent()?;
    loop {
        if let Some(snapshot) = scan_cache.get(parent) {
            let relative = view_root_path.strip_prefix(parent).ok()?;
            let mut idx = ROOT_NODE;
            for component in relative.components() {
                let name = component.as_os_str().to_str()?;
                idx = snapshot.node(idx).child_idx(name)?;
            }
            return Some((snapshot.as_ref(), idx));
        }
        parent = parent.parent()?;
    }
}

pub(crate) fn sort_children_snapshot(
    children: &mut Vec<(&String, NodeIndex)>,
    snap: &Snapshot,
    mode: SortMode,
    parent_path: &[String],
    delta_cache: Option<&HashMap<Vec<String>, i64>>,
) {
    match mode {
        SortMode::Name => children.sort_by(|a, b| a.0.cmp(b.0)),
        SortMode::Size => children.sort_by(|a, b| {
            let a_size = snap.node(a.1).size;
            let b_size = snap.node(b.1).size;
            b_size.cmp(&a_size)
        }),
        SortMode::Delta => {
            let mut with_delta: Vec<(i64, &String, NodeIndex)> = children
                .iter()
                .map(|(name, idx)| {
                    let mut child_path = parent_path.to_vec();
                    child_path.push((*name).clone());
                    let delta = delta_cache
                        .and_then(|c| c.get(&child_path))
                        .copied()
                        .unwrap_or(0);
                    (delta, *name, *idx)
                })
                .collect();
            with_delta.sort_unstable_by(|a, b| b.0.abs().cmp(&a.0.abs()));
            children.clear();
            for (_, name, idx) in with_delta {
                children.push((name, idx));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};
    use std::collections::HashMap;
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
        app.scan_cache
            .insert(PathBuf::from("/tmp/test"), Arc::new(scan_snap));
        app.update_tree_lines();
        app
    }

    fn node(name: &str, is_dir: bool, size: u64, children: Vec<(&str, NodeIndex)>) -> FileNode {
        FileNode {
            name: name.to_string(),
            parent: None,
            is_dir,
            file_type: if is_dir {
                FileType::Directory
            } else {
                FileType::File
            },
            size,
            children: children
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn test_enrich_snapshot_sizes_recurses_into_deep_children() {
        let root_path = PathBuf::from("/tmp/test");

        let live_arena = vec![
            node("test", true, 0, vec![("target", 1)]),
            node("target", true, 0, vec![("debug", 2)]),
            node("debug", true, 0, vec![("build", 3)]),
            node("build", true, 0, vec![("build-script-build", 4)]),
            node("build-script-build", false, 475_880, vec![]),
        ];
        let mut live_snap = Snapshot::new(root_path.clone(), live_arena, 0);

        let scan_arena = vec![
            node("test", true, 475_880, vec![("target", 1)]),
            node("target", true, 475_880, vec![("debug", 2)]),
            node("debug", true, 475_880, vec![("build", 3)]),
            node("build", true, 475_880, vec![("build-script-build", 4)]),
            node("build-script-build", false, 475_880, vec![]),
        ];
        let scan_snap = Snapshot::new(root_path.clone(), scan_arena, 475_880);
        let mut scan_cache = HashMap::new();
        scan_cache.insert(root_path.clone(), Arc::new(scan_snap));
        let root_scan_tree = resolve_scan_tree(&scan_cache, &root_path);

        enrich_snapshot_sizes(
            &mut live_snap,
            ROOT_NODE,
            &scan_cache,
            &root_path,
            root_scan_tree,
            &mut Vec::new(),
        );

        assert_eq!(live_snap.node(3).size, 475_880);
    }

    #[test]
    fn test_size_for_path_cache_hit() {
        let root_path = PathBuf::from("/tmp/test");
        let sub_path = root_path.join("src");
        let mut scan_cache = HashMap::new();
        scan_cache.insert(
            sub_path.clone(),
            Arc::new(Snapshot::new(
                sub_path,
                vec![node("src", true, 100, vec![])],
                100,
            )),
        );
        let size = size_for_path(
            &scan_cache,
            &root_path,
            None,
            &["test".into(), "src".into()],
        );
        assert_eq!(size, Some(100));
    }

    #[test]
    fn test_size_for_path_scan_tree_fallback() {
        let root_path = PathBuf::from("/tmp/test");
        let mut scan_cache = HashMap::new();
        let snap = Snapshot::new(
            root_path.clone(),
            vec![
                node("test", true, 200, vec![("src", 1)]),
                node("src", true, 200, vec![]),
            ],
            200,
        );
        scan_cache.insert(root_path.clone(), Arc::new(snap));
        let root_scan_tree = resolve_scan_tree(&scan_cache, &root_path);
        let size = size_for_path(
            &scan_cache,
            &root_path,
            root_scan_tree,
            &["test".into(), "src".into()],
        );
        assert_eq!(size, Some(200));
    }

    #[test]
    fn test_size_for_path_no_match() {
        let root_path = PathBuf::from("/tmp/test");
        let mut scan_cache = HashMap::new();
        let snap = Snapshot::new(root_path.clone(), vec![node("test", true, 0, vec![])], 0);
        scan_cache.insert(root_path.clone(), Arc::new(snap));
        let root_scan_tree = resolve_scan_tree(&scan_cache, &root_path);
        let size = size_for_path(
            &scan_cache,
            &root_path,
            root_scan_tree,
            &["test".into(), "nonexistent".into()],
        );
        assert_eq!(size, None);
    }

    #[test]
    fn test_expand_node_keeps_regular_dirs_marked_with_metadata() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("root");
        fs::create_dir_all(root_path.join("sub")).unwrap();
        fs::write(root_path.join("sub").join("file.txt"), "data").unwrap();

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
        app.scan_cache
            .insert(root_path.clone(), Arc::new(scan_snap));
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
            .insert(PathBuf::from("/tmp/test"), Arc::new(root_snapshot));
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

    #[test]
    fn test_delete_file_under_root_keeps_scan_data_and_percentage() {
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

        let arena = vec![
            dir_node("test", vec![("keep.txt", 1), ("delete.txt", 2)]),
            sized_file("keep.txt", 80),
            sized_file("delete.txt", 20),
        ];
        // simulate scanner: compute_size sets root node's size
        let mut root_snapshot = Snapshot::new(PathBuf::from("/tmp/test"), arena.clone(), 0);
        let mut snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        // set root node size as scanner would (compute_size)
        snap.node_mut(ROOT_NODE).size = 100;
        root_snapshot.node_mut(ROOT_NODE).size = 100;

        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/test");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.scan_cache
            .insert(PathBuf::from("/tmp/test"), Arc::new(root_snapshot));
        app.update_tree_lines();

        // verify initial state
        let keep_before = app
            .tree_lines
            .iter()
            .find(|l| l.node.name() == "keep.txt")
            .unwrap();
        assert!(keep_before.has_scan_data);
        assert_eq!(keep_before.node.current_size(), 80);
        let root_total = match &app.tree_root {
            Some(TreeNode::Snapshot(s, _)) => s.node(ROOT_NODE).size,
            _ => 0,
        };
        assert_eq!(root_total, 100);

        // delete a file directly under root
        apply_deletion_to_state(&mut app, Path::new("/tmp/test/delete.txt"));
        app.update_tree_lines();

        // verify tree_root sizes
        let TreeNode::Snapshot(snap_arc, _) = app.tree_root.as_ref().unwrap();
        assert_eq!(snap_arc.node(ROOT_NODE).size, 80);

        // verify remaining tree line still has scan data and correct size
        let keep = app.tree_lines.iter().find(|l| l.node.name() == "keep.txt");
        assert!(keep.is_some(), "keep.txt should still be in tree_lines");
        let keep = keep.unwrap();
        assert!(
            keep.has_scan_data,
            "keep.txt should have has_scan_data=true after deletion"
        );
        assert_eq!(
            keep.node.current_size(),
            80,
            "keep.txt size should be 80 after deletion"
        );

        // verify percentage would show correctly
        let root_total = snap_arc.node(ROOT_NODE).size;
        assert!(
            root_total > 0,
            "root_total_size should be > 0 after deletion"
        );
        let pct = (keep.node.current_size() as f64 / root_total as f64) * 100.0;
        assert!(
            (pct - 100.0).abs() < 0.1,
            "keep.txt should be 100% of remaining root"
        );

        // deleted file should not be in tree_lines
        let deleted = app
            .tree_lines
            .iter()
            .find(|l| l.node.name() == "delete.txt");
        assert!(
            deleted.is_none(),
            "delete.txt should be removed from tree_lines"
        );
    }

    #[test]
    fn test_delete_in_subdirectory_preserves_parent_scan_cache() {
        // Simulate: user scanned ~/code/github, navigated into argus/, then deleted a file.
        // The parent scan cache entry (~/code/github) must be PRUNED, not removed,
        // so resolve_scan_tree can still find sizes for remaining entries.
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

        // Parent scan: ~/code/github with argus/ and other files
        let parent_arena = vec![
            dir_node("github", vec![("argus", 1), ("readme.md", 2)]),
            dir_node("argus", vec![("keep.txt", 3), ("delete.txt", 4)]),
            sized_file("readme.md", 10),
            sized_file("keep.txt", 80),
            sized_file("delete.txt", 20),
        ];
        let mut parent_scan = Snapshot::new(PathBuf::from("/tmp/github"), parent_arena.clone(), 0);
        parent_scan.node_mut(ROOT_NODE).size = 110; // scanner compute_size
        parent_scan.node_mut(1).size = 100; // argus total

        // Subdirectory view: argus/ (shallow, like list_dir result)
        let sub_arena = vec![
            dir_node("argus", vec![("keep.txt", 1), ("delete.txt", 2)]),
            sized_file("keep.txt", 80),
            sized_file("delete.txt", 20),
        ];
        let mut sub_snap = Snapshot::new(PathBuf::from("/tmp/github/argus"), sub_arena, 0);

        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/github/argus");
        // Store parent scan in cache (subdirectory is NOT in cache)
        app.scan_cache
            .insert(PathBuf::from("/tmp/github"), Arc::new(parent_scan));
        // Enrich subdirectory snapshot (simulating build_current_tree)
        let root_scan_tree = resolve_scan_tree(&app.scan_cache, &app.view_root_path);
        enrich_snapshot_sizes(
            &mut sub_snap,
            ROOT_NODE,
            &app.scan_cache,
            &app.view_root_path,
            root_scan_tree,
            &mut Vec::new(),
        );
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(sub_snap), ROOT_NODE));
        app.update_tree_lines();

        // Verify initial state
        let keep = app
            .tree_lines
            .iter()
            .find(|l| l.node.name() == "keep.txt")
            .unwrap();
        assert!(keep.has_scan_data);
        assert_eq!(keep.node.current_size(), 80);

        // Delete a file inside the subdirectory
        apply_deletion_to_state(&mut app, Path::new("/tmp/github/argus/delete.txt"));
        app.update_tree_lines();

        // Verify parent scan still exists in cache
        assert!(
            app.scan_cache.contains_key(&PathBuf::from("/tmp/github")),
            "parent scan cache entry should be preserved (pruned, not removed)"
        );

        // Verify resolve_scan_tree still works
        let rst = resolve_scan_tree(&app.scan_cache, &app.view_root_path);
        assert!(
            rst.is_some(),
            "resolve_scan_tree should still find parent scan"
        );

        // Verify remaining entry has scan data
        let keep = app.tree_lines.iter().find(|l| l.node.name() == "keep.txt");
        assert!(keep.is_some(), "keep.txt should still be in tree_lines");
        let keep = keep.unwrap();
        assert!(
            keep.has_scan_data,
            "keep.txt should have has_scan_data=true after deletion in subdirectory view"
        );

        // Verify tree_root root node size is recomputed
        let TreeNode::Snapshot(snap_arc, _) = app.tree_root.as_ref().unwrap();
        assert_eq!(snap_arc.node(ROOT_NODE).size, 80);
    }

    #[test]
    fn test_collapse_or_navigate_up_at_root() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        let snap = Snapshot::new(
            PathBuf::from("/tmp/test"),
            vec![dir_node("test", vec![])],
            0,
        );
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.update_tree_lines();
        collapse_or_navigate_up(&mut app);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_collapse_or_navigate_up_collapse_expanded_dir() {
        let arena = vec![
            dir_node("test", vec![("sub", 1)]),
            dir_node("sub", vec![("nested", 2)]),
            dir_node("nested", vec![]),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["test".to_string(), "sub".to_string()]);
        app.update_tree_lines();
        app.cursor = 1;
        let line = app.selected_line().unwrap();
        assert!(line.expanded);

        collapse_or_navigate_up(&mut app);
        assert!(!app
            .expanded
            .contains(&vec!["test".to_string(), "sub".to_string()]));
    }

    #[test]
    fn test_collapse_or_navigate_up_to_parent() {
        let arena = vec![
            dir_node("test", vec![("sub", 1), ("other", 2)]),
            dir_node("sub", vec![]),
            dir_node("other", vec![]),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["test".to_string(), "sub".to_string()]);
        app.update_tree_lines();
        app.cursor = 1;
        collapse_or_navigate_up(&mut app);
        collapse_or_navigate_up(&mut app);
        let selected = app.selected_line().unwrap();
        assert_eq!(selected.depth, 0);
    }

    #[test]
    fn test_collapse_all_children_basic() {
        let arena = vec![
            dir_node("test", vec![("a", 1)]),
            dir_node("a", vec![("deep", 2)]),
            dir_node("deep", vec![]),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["test".to_string(), "a".to_string()]);
        app.expanded.insert(vec![
            "test".to_string(),
            "a".to_string(),
            "deep".to_string(),
        ]);
        app.update_tree_lines();

        assert_eq!(app.tree_lines.len(), 3);

        collapse_all_children(&mut app);

        assert!(app.expanded.iter().all(|p| p.len() <= 1));
        assert_eq!(app.tree_lines.len(), 2);
    }

    #[test]
    fn test_collapse_all_children_cursor_adjustment() {
        let arena = vec![
            dir_node("test", vec![("a", 1)]),
            dir_node("a", vec![("deep", 2)]),
            dir_node("deep", vec![]),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["test".to_string(), "a".to_string()]);
        app.expanded.insert(vec![
            "test".to_string(),
            "a".to_string(),
            "deep".to_string(),
        ]);
        app.update_tree_lines();
        app.cursor = 2;

        collapse_all_children(&mut app);

        assert_eq!(app.cursor, 1);
        assert!(app.cursor < app.tree_lines.len());
    }

    #[test]
    fn test_collapse_all_children_snaps_to_root_when_way_out_of_bounds() {
        let arena = vec![dir_node("test", vec![])];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.update_tree_lines();
        app.cursor = 100;

        collapse_all_children(&mut app);

        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_navigate_up_root_basic() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let snap = Snapshot::new(sub.clone(), vec![file_node("sub", 0)], 0);
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = sub;
        app.scan_cache
            .insert(tmp.path().to_path_buf(), Arc::new(snap));

        navigate_up_root(&mut app);

        assert_eq!(app.view_root_path, tmp.path());
    }

    #[test]
    fn test_navigate_up_root_at_filesystem_root() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/");

        navigate_up_root(&mut app);

        assert_eq!(app.view_root_path, PathBuf::from("/"));
    }
}
