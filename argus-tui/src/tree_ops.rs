use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use argus_core::{NodeIndex, Snapshot, ROOT_NODE};

use crate::app::{App, TreeNode};

/// Convert a filesystem path relative to a base into string components.
fn path_to_components(base: &Path, full: &Path) -> Option<Vec<String>> {
    let relative = full.strip_prefix(base).ok()?;
    let components: Vec<String> = relative
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(String::from))
        .collect();
    if components.is_empty() {
        None
    } else {
        Some(components)
    }
}

/// Get the size of a node at `deleted_path` within a snapshot, without modifying it.
fn node_size_in_snapshot(snapshot: &Snapshot, deleted_path: &Path) -> u64 {
    let components = match path_to_components(&snapshot.root_path, deleted_path) {
        Some(c) => c,
        None => return 0,
    };
    let mut idx = ROOT_NODE;
    for (step, comp) in components.iter().enumerate() {
        if step + 1 == components.len() {
            return snapshot
                .children(idx)
                .iter()
                .find(|&&child_idx| snapshot.name(child_idx) == *comp)
                .map(|&child_idx| snapshot.node(child_idx).size())
                .unwrap_or(0);
        }
        match snapshot.child_idx(idx, comp) {
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
            if let Some(arc) = app.scan_cache.get_mut(&key) {
                let snapshot = Arc::make_mut(arc);
                let freed = node_size_in_snapshot(snapshot, deleted_path);
                remove_path_from_snapshot(snapshot, deleted_path);
                total_freed = total_freed.saturating_add(freed);
            }
        } else if let Some(snapshot) = app.scan_cache.remove(&key) {
            total_freed = total_freed.saturating_add(snapshot.total_size);
        }
    }

    if let Some(TreeNode::Snapshot(snap_arc, _)) = &mut app.tree_root {
        let snap = Arc::make_mut(snap_arc);
        let _ = remove_path_from_tree(snap, &app.view_root_path, deleted_path);
    }

    total_freed
}

fn remove_path_from_snapshot(snapshot: &mut Snapshot, deleted_path: &Path) -> bool {
    let components = match path_to_components(&snapshot.root_path, deleted_path) {
        Some(c) => c,
        None => return false,
    };
    let removed = prune_file_node(snapshot, ROOT_NODE, &components, 0);
    if removed {
        snapshot.total_size = snapshot.node(ROOT_NODE).size();
    }
    removed
}

fn remove_path_from_tree(snap: &mut Snapshot, root_path: &Path, deleted_path: &Path) -> bool {
    let components = match path_to_components(root_path, deleted_path) {
        Some(c) => c,
        None => return false,
    };
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
        let pos = snap
            .children(current_idx)
            .iter()
            .position(|&child_idx| snap.name(child_idx) == components[index]);
        if let Some(p) = pos {
            snap.swap_remove_child(current_idx, p);
            true
        } else {
            false
        }
    } else if let Some(child_idx) = snap.child_idx(current_idx, &components[index]) {
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
    if snap.children_is_empty(idx) {
        // Leaf or emptied dir: keep file size; dirs with no children become 0 if they were dirs
        if snap.node(idx).is_dir() {
            snap.node_mut(idx).set_size(0);
            return 0;
        }
        return snap.node(idx).size();
    }

    let children: Vec<NodeIndex> = snap.children_clone(idx);
    let mut total = 0u64;
    for child_idx in children {
        total = total.saturating_add(recompute_file_node_size(snap, child_idx));
    }
    snap.node_mut(idx).set_size(total);
    total
}

// ── Size enrichment / sorting ────────────────────────────────────────────────

pub(crate) fn enrich_snapshot_sizes(
    snap: &mut Snapshot,
    idx: NodeIndex,
    scan_cache: &HashMap<PathBuf, Arc<Snapshot>>,
    view_root_path: &Path,
    root_scan_tree: Option<(&Snapshot, NodeIndex)>,
    path: &mut Vec<String>,
) {
    let name = snap.name(idx).to_string();
    path.push(name);

    if let Some(size) = size_for_path(scan_cache, view_root_path, root_scan_tree, path) {
        snap.node_mut(idx).set_size(size);
    }

    if snap.node(idx).is_dir() {
        let children: Vec<NodeIndex> = snap.children_clone(idx);
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
        return Some(snapshot.node(ROOT_NODE).size());
    }

    root_scan_tree.and_then(|(snap, idx)| {
        snap.find_node(idx, path_key)
            .map(|found_idx| snap.node(found_idx).size())
    })
}

/// Find the best-available scan tree node for a view path.
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
                idx = snapshot.child_idx(idx, name)?;
            }
            return Some((snapshot.as_ref(), idx));
        }
        parent = parent.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argus_core::{FileType, SnapshotBuilder, ROOT_NODE};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn make_flat_app(snap: Snapshot, scan_snap: Snapshot) -> App {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/test");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.scan_cache
            .insert(PathBuf::from("/tmp/test"), Arc::new(scan_snap));
        app.current_dir_path = vec!["test".into()];
        app.load_current_children();
        app
    }

    #[test]
    fn test_enrich_snapshot_sizes_recurses_into_deep_children() {
        let root_path = PathBuf::from("/tmp/test");

        let mut live = SnapshotBuilder::new("test");
        let t = live.push_dir(ROOT_NODE, "target");
        let d = live.push_dir(t, "debug");
        let b = live.push_dir(d, "build");
        live.push_file(b, "build-script-build", FileType::File, 475_880, 475_880);
        let mut live_snap = live.finish(root_path.clone(), 0, 0);

        let mut scan = SnapshotBuilder::new("test");
        let t = scan.push_dir(ROOT_NODE, "target");
        let d = scan.push_dir(t, "debug");
        let b = scan.push_dir(d, "build");
        scan.push_file(b, "build-script-build", FileType::File, 475_880, 475_880);
        // set rolled sizes on scan tree
        for i in (1..scan.nodes.len()).rev() {
            let size = scan.nodes[i].size();
            if let Some(p) = scan.nodes[i].parent() {
                let tot = scan.nodes[p as usize].size().saturating_add(size);
                scan.nodes[p as usize].set_size(tot);
            }
        }
        let total = scan.nodes[0].size();
        let scan_snap = scan.finish(root_path.clone(), total, total);

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

        // build dir is index 3
        assert_eq!(live_snap.node(3).size(), 475_880);
    }

    #[test]
    fn test_size_for_path_cache_hit() {
        let root_path = PathBuf::from("/tmp/test");
        let sub_path = root_path.join("src");
        let mut scan_cache = HashMap::new();
        let mut b = SnapshotBuilder::new("src");
        b.nodes[0].set_size(100);
        scan_cache.insert(sub_path.clone(), Arc::new(b.finish(sub_path, 100, 0)));
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
        let mut b = SnapshotBuilder::new("test");
        let src = b.push_dir(ROOT_NODE, "src");
        b.nodes[src as usize].set_size(200);
        b.nodes[0].set_size(200);
        let snap = b.finish(root_path.clone(), 200, 0);
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
        let snap = SnapshotBuilder::new("test").finish(root_path.clone(), 0, 0);
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
    fn test_delete_updates_parent_sizes_and_scan_cache() {
        let mut b = SnapshotBuilder::new("test");
        let ignore = b.push_dir(ROOT_NODE, "ignore");
        b.push_file(ignore, "keep.bin", FileType::File, 12, 12);
        b.push_file(ignore, "delete.bin", FileType::File, 10, 10);
        for i in (1..b.nodes.len()).rev() {
            let size = b.nodes[i].size();
            if let Some(p) = b.nodes[i].parent() {
                let t = b.nodes[p as usize].size().saturating_add(size);
                b.nodes[p as usize].set_size(t);
            }
        }
        let total = b.nodes[0].size();
        let arena_snap = b.finish(PathBuf::from("/tmp/test"), total, total);
        let root_snapshot = arena_snap.clone();
        let snap = arena_snap;

        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/test");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.scan_cache
            .insert(PathBuf::from("/tmp/test"), Arc::new(root_snapshot));
        app.current_dir_path = vec!["test".into()];
        app.load_current_children();

        apply_deletion_to_state(&mut app, Path::new("/tmp/test/ignore/delete.bin"));
        app.current_dir_path = vec!["test".into()];
        app.load_current_children();

        let TreeNode::Snapshot(snap_arc, _) = app.tree_root.as_ref().unwrap();
        let ignore = snap_arc.node(1);
        assert_eq!(ignore.size(), 12);
        assert_eq!(snap_arc.node(ROOT_NODE).size(), 12);

        let cached = app.scan_cache.get(&PathBuf::from("/tmp/test")).unwrap();
        assert_eq!(cached.node(ROOT_NODE).size(), 12);
        let cached_ignore = cached.node(1);
        assert_eq!(cached_ignore.size(), 12);
        assert!(!cached
            .children(1)
            .iter()
            .any(|&idx| cached.name(idx) == "delete.bin"));
    }

    #[test]
    fn test_delete_file_under_root_keeps_scan_data_and_percentage() {
        let mut b = SnapshotBuilder::new("test");
        b.push_file(ROOT_NODE, "keep.txt", FileType::File, 80, 80);
        b.push_file(ROOT_NODE, "delete.txt", FileType::File, 20, 20);
        for i in (1..b.nodes.len()).rev() {
            let size = b.nodes[i].size();
            if let Some(p) = b.nodes[i].parent() {
                let t = b.nodes[p as usize].size().saturating_add(size);
                b.nodes[p as usize].set_size(t);
            }
        }
        let total = b.nodes[0].size();
        let snap = b.finish(PathBuf::from("/tmp/test"), total, total);
        let root_snapshot = snap.clone();

        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/test");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.scan_cache
            .insert(PathBuf::from("/tmp/test"), Arc::new(root_snapshot));
        app.current_dir_path = vec!["test".into()];
        app.load_current_children();

        let keep_before = app
            .current_children
            .iter()
            .find(|l| l.node.name() == "keep.txt")
            .unwrap();
        assert!(keep_before.has_scan_data);
        assert_eq!(keep_before.size, 80);

        apply_deletion_to_state(&mut app, Path::new("/tmp/test/delete.txt"));
        app.current_dir_path = vec!["test".into()];
        app.load_current_children();

        let TreeNode::Snapshot(snap_arc, _) = app.tree_root.as_ref().unwrap();
        assert_eq!(snap_arc.node(ROOT_NODE).size(), 80);

        let keep = app
            .current_children
            .iter()
            .find(|l| l.node.name() == "keep.txt")
            .unwrap();
        assert!(keep.has_scan_data);
        assert_eq!(keep.size, 80);

        let root_total = snap_arc.node(ROOT_NODE).size();
        let pct = (keep.size as f64 / root_total as f64) * 100.0;
        assert!((pct - 100.0).abs() < 0.1);

        assert!(app
            .current_children
            .iter()
            .find(|l| l.node.name() == "delete.txt")
            .is_none());
    }

    #[test]
    fn test_delete_in_subdirectory_preserves_parent_scan_cache() {
        let mut parent_b = SnapshotBuilder::new("github");
        let argus = parent_b.push_dir(ROOT_NODE, "argus");
        parent_b.push_file(ROOT_NODE, "readme.md", FileType::File, 10, 10);
        parent_b.push_file(argus, "keep.txt", FileType::File, 80, 80);
        parent_b.push_file(argus, "delete.txt", FileType::File, 20, 20);
        for i in (1..parent_b.nodes.len()).rev() {
            let size = parent_b.nodes[i].size();
            if let Some(p) = parent_b.nodes[i].parent() {
                let t = parent_b.nodes[p as usize].size().saturating_add(size);
                parent_b.nodes[p as usize].set_size(t);
            }
        }
        let parent_total = parent_b.nodes[0].size();
        let parent_scan = parent_b.finish(PathBuf::from("/tmp/github"), parent_total, parent_total);

        let mut sub_b = SnapshotBuilder::new("argus");
        sub_b.push_file(ROOT_NODE, "keep.txt", FileType::File, 80, 80);
        sub_b.push_file(ROOT_NODE, "delete.txt", FileType::File, 20, 20);
        let mut sub_snap = sub_b.finish(PathBuf::from("/tmp/github/argus"), 0, 0);

        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/github/argus");
        app.scan_cache
            .insert(PathBuf::from("/tmp/github"), Arc::new(parent_scan));
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
        app.current_dir_path = vec!["argus".into()];
        app.load_current_children();

        let keep = app
            .current_children
            .iter()
            .find(|l| l.node.name() == "keep.txt")
            .unwrap();
        assert!(keep.has_scan_data);
        assert_eq!(keep.size, 80);

        apply_deletion_to_state(&mut app, Path::new("/tmp/github/argus/delete.txt"));
        app.current_dir_path = vec!["argus".into()];
        app.load_current_children();

        assert!(app.scan_cache.contains_key(&PathBuf::from("/tmp/github")));
        assert!(resolve_scan_tree(&app.scan_cache, &app.view_root_path).is_some());

        let keep = app
            .current_children
            .iter()
            .find(|l| l.node.name() == "keep.txt")
            .unwrap();
        assert!(keep.has_scan_data);

        let TreeNode::Snapshot(snap_arc, _) = app.tree_root.as_ref().unwrap();
        assert_eq!(snap_arc.node(ROOT_NODE).size(), 80);
    }

    #[test]
    fn test_make_flat_app_smoke() {
        let mut b = SnapshotBuilder::new("test");
        b.push_file(ROOT_NODE, "a.txt", FileType::File, 1, 1);
        let snap = b.finish(PathBuf::from("/tmp/test"), 1, 1);
        let scan = snap.clone();
        let app = make_flat_app(snap, scan);
        assert!(!app.current_children.is_empty());
    }
}
