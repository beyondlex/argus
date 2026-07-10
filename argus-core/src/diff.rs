use std::collections::HashMap;

use crate::model::{DiffError, DiffNode, FileNode, Snapshot};

pub fn compare_trees(old: &Snapshot, new: &Snapshot) -> Result<DiffNode, DiffError> {
    if old.root_path != new.root_path {
        return Err(DiffError::RootPathMismatch(
            old.root_path.clone(),
            new.root_path.clone(),
        ));
    }

    let root = compare_nodes(&old.root_node, &new.root_node);
    Ok(root)
}

fn compare_nodes(old: &FileNode, new: &FileNode) -> DiffNode {
    let name = if !new.name.is_empty() {
        new.name.clone()
    } else {
        old.name.clone()
    };

    let is_dir = new.is_dir || old.is_dir;

    let size_delta = if old.size == 0 && new.size == 0 {
        0
    } else {
        let delta = new.size as i128 - old.size as i128;
        delta.clamp(i64::MIN as i128, i64::MAX as i128) as i64
    };
    let current_size = new.size;

    let mut children: HashMap<String, DiffNode> = HashMap::new();

    if is_dir {
        let all_keys: std::collections::HashSet<&str> = old
            .children
            .keys()
            .chain(new.children.keys())
            .map(|s| s.as_str())
            .collect();

        for key in all_keys {
            let old_child = old.children.get(key);
            let new_child = new.children.get(key);

            match (old_child, new_child) {
                (Some(a), Some(b)) => {
                    let diff = compare_nodes(a, b);
                    children.insert(key.to_string(), diff);
                }
                (Some(a), None) => {
                    children.insert(
                        key.to_string(),
                        DiffNode {
                            name: key.to_string(),
                            is_dir: a.is_dir,
                            current_size: 0,
                            size_delta: -(a.size as i64),
                            children: delete_subtree_children(a),
                        },
                    );
                }
                (None, Some(b)) => {
                    children.insert(
                        key.to_string(),
                        DiffNode {
                            name: key.to_string(),
                            is_dir: b.is_dir,
                            current_size: b.size,
                            size_delta: b.size as i64,
                            children: add_subtree_children(b),
                        },
                    );
                }
                (None, None) => {}
            }
        }
    }

    let mut result = DiffNode {
        name,
        is_dir,
        current_size,
        size_delta,
        children,
    };

    if is_dir {
        aggregate_deltas(&mut result);
    }

    result
}

fn aggregate_deltas(node: &mut DiffNode) {
    for child in node.children.values_mut() {
        if child.is_dir {
            aggregate_deltas(child);
        }
    }

    let child_delta: i64 = node.children.values().map(|c| c.size_delta).sum();
    if !node.children.is_empty() {
        node.size_delta = child_delta;
    }
}

fn delete_subtree_children(node: &FileNode) -> HashMap<String, DiffNode> {
    node.children
        .iter()
        .map(|(name, child)| {
            let diff_child = DiffNode {
                name: name.clone(),
                is_dir: child.is_dir,
                current_size: 0,
                size_delta: -(child.size as i64),
                children: if child.is_dir {
                    delete_subtree_children(child)
                } else {
                    HashMap::new()
                },
            };
            (name.clone(), diff_child)
        })
        .collect()
}

fn add_subtree_children(node: &FileNode) -> HashMap<String, DiffNode> {
    node.children
        .iter()
        .map(|(name, child)| {
            let diff_child = DiffNode {
                name: name.clone(),
                is_dir: child.is_dir,
                current_size: child.size,
                size_delta: child.size as i64,
                children: if child.is_dir {
                    add_subtree_children(child)
                } else {
                    HashMap::new()
                },
            };
            (name.clone(), diff_child)
        })
        .collect()
}

pub fn filter_by_threshold(node: &DiffNode, threshold: u64) -> Option<DiffNode> {
    let threshold_i64 = threshold as i64;

    let mut filtered_children: HashMap<String, DiffNode> = HashMap::new();
    for (name, child) in &node.children {
        if let Some(filtered) = filter_by_threshold(child, threshold) {
            filtered_children.insert(name.clone(), filtered);
        }
    }

    let has_visible_children = !filtered_children.is_empty();
    let has_real_change = node.size_delta != 0;
    let above_threshold = threshold == 0 || node.size_delta.abs() >= threshold_i64;

    if has_visible_children || (has_real_change && above_threshold) {
        Some(DiffNode {
            name: node.name.clone(),
            is_dir: node.is_dir,
            current_size: node.current_size,
            size_delta: node.size_delta,
            children: filtered_children,
        })
    } else {
        None
    }
}

pub fn has_significant_changes(node: &DiffNode, threshold: u64) -> bool {
    if node.size_delta.abs() >= threshold as i64 {
        return true;
    }
    for child in node.children.values() {
        if has_significant_changes(child, threshold) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileNode, FileType, Snapshot, SNAPSHOT_VERSION};
    use chrono::Utc;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_file(name: &str, size: u64) -> FileNode {
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

    fn make_dir(name: &str, children: Vec<FileNode>) -> FileNode {
        let mut map = HashMap::new();
        for child in children {
            map.insert(child.name.clone(), child);
        }
        let total = map.values().map(compute_total_size).sum();
        FileNode {
            name: name.to_string(),
            is_dir: true,
            file_type: FileType::Directory,
            size: total,
            modified: None,
            created: None,
            inode: None,
            device: None,
            has_metadata: true,
            children: map,
        }
    }

    fn compute_total_size(node: &FileNode) -> u64 {
        if node.children.is_empty() {
            node.size
        } else {
            node.children.values().map(compute_total_size).sum()
        }
    }

    fn make_snapshot(path: &str, root: FileNode) -> Snapshot {
        Snapshot {
            version: SNAPSHOT_VERSION,
            timestamp: Utc::now(),
            root_path: PathBuf::from(path),
            root_path_hash: crate::model::hash_root_path(&PathBuf::from(path)),
            total_size: compute_total_size(&root),
            root_node: root,
        }
    }

    #[test]
    fn test_compare_trees_both_empty() {
        let old = make_snapshot("/test", make_dir("test", vec![]));
        let new = make_snapshot("/test", make_dir("test", vec![]));
        let result = compare_trees(&old, &new).unwrap();
        assert_eq!(result.size_delta, 0);
        assert!(result.children.is_empty());
    }

    #[test]
    fn test_single_file_added() {
        let old = make_snapshot("/test", make_dir("test", vec![]));
        let new = make_snapshot("/test", make_dir("test", vec![make_file("file.txt", 100)]));
        let result = compare_trees(&old, &new).unwrap();
        assert_eq!(result.size_delta, 100);
        assert_eq!(result.children.len(), 1);
        let file = result.children.get("file.txt").unwrap();
        assert_eq!(file.size_delta, 100);
    }

    #[test]
    fn test_single_file_deleted() {
        let old = make_snapshot("/test", make_dir("test", vec![make_file("file.txt", 100)]));
        let new = make_snapshot("/test", make_dir("test", vec![]));
        let result = compare_trees(&old, &new).unwrap();
        assert_eq!(result.size_delta, -100);
        assert_eq!(result.children.len(), 1);
        let file = result.children.get("file.txt").unwrap();
        assert_eq!(file.size_delta, -100);
        assert_eq!(file.current_size, 0);
    }

    #[test]
    fn test_directory_added() {
        let old = make_snapshot("/test", make_dir("test", vec![]));
        let new = make_snapshot(
            "/test",
            make_dir("test", vec![make_dir("dir", vec![make_file("f.txt", 200)])]),
        );
        let result = compare_trees(&old, &new).unwrap();
        assert_eq!(result.size_delta, 200);
    }

    #[test]
    fn test_directory_shrunk() {
        let old = make_snapshot(
            "/test",
            make_dir(
                "test",
                vec![make_dir(
                    "dir",
                    vec![make_file("f1", 100), make_file("f2", 200)],
                )],
            ),
        );
        let new = make_snapshot(
            "/test",
            make_dir("test", vec![make_dir("dir", vec![make_file("f1", 100)])]),
        );
        let result = compare_trees(&old, &new).unwrap();
        assert_eq!(result.size_delta, -200);
    }

    #[test]
    fn test_deep_nesting() {
        let old = make_snapshot(
            "/test",
            make_dir(
                "test",
                vec![make_dir(
                    "a",
                    vec![make_dir(
                        "b",
                        vec![make_dir("c", vec![make_file("f", 100)])],
                    )],
                )],
            ),
        );
        let new = make_snapshot(
            "/test",
            make_dir(
                "test",
                vec![make_dir(
                    "a",
                    vec![make_dir(
                        "b",
                        vec![make_dir("c", vec![make_file("f", 300)])],
                    )],
                )],
            ),
        );
        let result = compare_trees(&old, &new).unwrap();
        let a = result.children.get("a").unwrap();
        let b = a.children.get("b").unwrap();
        let c = b.children.get("c").unwrap();
        assert_eq!(c.size_delta, 200);
        assert_eq!(b.size_delta, 200);
        assert_eq!(a.size_delta, 200);
        assert_eq!(result.size_delta, 200);
    }

    #[test]
    fn test_root_path_mismatch() {
        let old = make_snapshot("/path_a", make_dir("root", vec![]));
        let new = make_snapshot("/path_b", make_dir("root", vec![]));
        let result = compare_trees(&old, &new);
        assert!(matches!(result, Err(DiffError::RootPathMismatch(_, _))));
    }

    #[test]
    fn test_threshold_filter() {
        let tree = make_dir(
            "test",
            vec![
                make_file("small", 10),
                make_file("medium", 100),
                make_file("large", 1000),
            ],
        );
        let empty = make_dir("test", vec![]);
        let old = make_snapshot("/test", empty);
        let new = make_snapshot("/test", tree);
        let diff = compare_trees(&old, &new).unwrap();

        let filtered = filter_by_threshold(&diff, 100).unwrap();
        assert!(!filtered.children.contains_key("small"));
        assert!(filtered.children.contains_key("medium"));
        assert!(filtered.children.contains_key("large"));
    }

    #[test]
    fn test_no_changes() {
        let tree = make_dir("test", vec![make_file("f", 100)]);
        let old = make_snapshot("/test", tree.clone());
        let new = make_snapshot("/test", tree);
        let result = compare_trees(&old, &new).unwrap();
        assert_eq!(result.size_delta, 0);
    }

    #[test]
    fn test_threshold_zero_filters_unchanged() {
        let tree = make_dir(
            "test",
            vec![
                make_file("unchanged.txt", 100),
                make_file("changed.txt", 200),
            ],
        );
        let old = make_snapshot("/test", tree);
        let tree2 = make_dir(
            "test",
            vec![
                make_file("unchanged.txt", 100),
                make_file("changed.txt", 300),
            ],
        );
        let new = make_snapshot("/test", tree2);
        let diff = compare_trees(&old, &new).unwrap();
        let filtered = filter_by_threshold(&diff, 0).unwrap();
        assert!(!filtered.children.contains_key("unchanged.txt"));
        assert!(filtered.children.contains_key("changed.txt"));
        assert_eq!(
            filtered.children.get("changed.txt").unwrap().size_delta,
            100
        );
    }

    #[test]
    fn test_has_significant_changes() {
        let diff = DiffNode {
            name: "root".into(),
            is_dir: true,
            current_size: 1000,
            size_delta: 500,
            children: HashMap::new(),
        };
        assert!(has_significant_changes(&diff, 400));
        assert!(!has_significant_changes(&diff, 600));
    }
}
