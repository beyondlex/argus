use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use argus_core::{NodeIndex, Snapshot};

use crate::app::{App, SearchMatch, SortMode};
use crate::event::SHOULD_QUIT;

pub fn jump_to_next_match(app: &mut App, delta: isize) {
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
        for path in &newly_expanded {
            app.expand_path_in_tree(path);
        }
    }

    // Fast path: target match visible in filtered view
    if let Some(&pos) = app.path_to_tree_idx.get(&target_path) {
        if let Some(cursor_pos) = app.filtered_tree_lines.iter().position(|&i| i == pos) {
            app.cursor = cursor_pos;
            return;
        }
    }

    // Fallback: find next visible match using HashMap O(1) per lookup
    let total = app.match_indices.len();
    for offset in 1..total {
        let try_idx = if delta >= 0 {
            (new_idx + offset) % total
        } else {
            (new_idx + total - offset) % total
        };
        let try_path = &app.match_indices[try_idx].path;
        if let Some(&pos) = app.path_to_tree_idx.get(try_path) {
            if let Some(cursor_pos) = app.filtered_tree_lines.iter().position(|&i| i == pos) {
                app.current_match = try_idx;
                app.cursor = cursor_pos;
                return;
            }
        }
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

// ── Search functions moved from app.rs ───────────────────────────────────────

/// Walk tree in depth-first display order, collecting search matches.
#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_matches_in_order(
    snap: &Snapshot,
    idx: NodeIndex,
    query: &str,
    expanded: &HashSet<Vec<String>>,
    sort_mode: SortMode,
    path: &mut Vec<String>,
    walk_index: &mut usize,
    visible_count: &mut usize,
    result: &mut Vec<SearchMatch>,
    path_to_walk_idx: &mut HashMap<Vec<String>, usize>,
    delta_cache: Option<&HashMap<Vec<String>, i64>>,
) {
    let node = snap.node(idx);
    let is_visible = path_is_visible(path, expanded);

    path_to_walk_idx.insert(path.clone(), *walk_index);

    if fuzzy_match_indices(query, &node.name).is_some() {
        result.push(SearchMatch {
            path: path.clone(),
            tree_idx: if is_visible {
                Some(*visible_count)
            } else {
                None
            },
            walk_idx: *walk_index,
        });
    }

    if is_visible {
        *visible_count += 1;
    }
    *walk_index += 1;

    if node.is_dir {
        let skip_subtree = node.children.len() > crate::tree_ops::MAX_DIR_CHILDREN;

        if !skip_subtree {
            let mut children: Vec<(&String, NodeIndex)> =
                node.children.iter().map(|(n, i)| (n, *i)).collect();
            crate::tree_ops::sort_children_snapshot(
                &mut children,
                snap,
                sort_mode,
                path,
                delta_cache,
            );
            for (name, child_idx) in children {
                path.push(name.clone());
                collect_matches_in_order(
                    snap,
                    child_idx,
                    query,
                    expanded,
                    sort_mode,
                    path,
                    walk_index,
                    visible_count,
                    result,
                    path_to_walk_idx,
                    delta_cache,
                );
                path.pop();
            }
        }
    }
}

fn path_is_visible(path: &[String], expanded: &HashSet<Vec<String>>) -> bool {
    if path.len() <= 1 {
        return true;
    }
    (1..path.len()).all(|len| expanded.contains(&path[..len].to_vec()))
}

/// Simple substring-based match returning character indices for highlighting.
pub fn fuzzy_match_indices(query: &str, target: &str) -> Option<Vec<usize>> {
    if query.is_empty() {
        return None;
    }
    let target_lc = target.to_lowercase();
    let query_lc = query.to_lowercase();
    let byte_pos = target_lc.find(&query_lc)?;
    let start = target_lc[..byte_pos].chars().count();
    let end = start + query_lc.chars().count();
    Some((start..end).collect())
}

/// Fuzzy substring match for command autocomplete filtering.
pub(crate) fn fuzzy_match(query: &str, target: &str) -> bool {
    let mut chars = target.chars();
    for qc in query.chars() {
        loop {
            match chars.next() {
                Some(tc) if tc == qc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{SearchMatch, TreeNode};
    use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};
    use std::path::PathBuf;
    use std::sync::Arc;
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
        app.search_word = "target".to_string();
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
    fn test_jump_auto_expands_collapsed_ancestors() {
        let root_arena = vec![
            dir_node("test", vec![("a", 1), ("b", 2)]),
            dir_node("a", vec![("target_file", 3)]),
            dir_node("b", vec![("other", 4)]),
            file_node("target_file", 1),
            file_node("other", 1),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), root_arena, 2);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);

        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.search_word = "target".to_string();
        app.recompute_matches();

        assert_eq!(app.match_indices.len(), 1);
        assert!(
            app.match_indices[0].tree_idx.is_none(),
            "match should be hidden (tree_idx=None) before expansion"
        );

        app.cursor = 0;
        jump_to_next_match(&mut app, 1);

        let selected = app.selected_line().expect("cursor should point to a line");
        assert_eq!(selected.node.name(), "target_file");
        assert!(
            app.expanded
                .contains(&vec!["test".to_string(), "a".to_string()]),
            "parent 'a' should be in expanded set"
        );
        let tree_paths: Vec<Vec<String>> = app.tree_lines.iter().map(|l| l.path.clone()).collect();
        assert!(
            tree_paths.contains(&vec![
                "test".to_string(),
                "a".to_string(),
                "target_file".to_string()
            ]),
            "target_file should be in tree_lines after expansion"
        );
    }

    #[test]
    fn test_jump_auto_expands_deeply_nested() {
        let root_arena = vec![
            dir_node("test", vec![("a", 1)]),
            dir_node("a", vec![("b", 2)]),
            dir_node("b", vec![("c", 3)]),
            dir_node("c", vec![("target", 4)]),
            file_node("target", 1),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), root_arena, 1);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);

        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.search_word = "target".to_string();
        app.recompute_matches();

        assert_eq!(app.match_indices.len(), 1);
        assert!(app.match_indices[0].tree_idx.is_none());

        app.cursor = 0;
        jump_to_next_match(&mut app, 1);

        let selected = app.selected_line().expect("cursor should point to a line");
        assert_eq!(selected.node.name(), "target");
        assert!(app
            .expanded
            .contains(&vec!["test".to_string(), "a".to_string()]));
        assert!(app
            .expanded
            .contains(&vec!["test".to_string(), "a".to_string(), "b".to_string()]));
        assert!(app.expanded.contains(&vec![
            "test".to_string(),
            "a".to_string(),
            "b".to_string(),
            "c".to_string()
        ]));
        let tree_paths: Vec<Vec<String>> = app.tree_lines.iter().map(|l| l.path.clone()).collect();
        assert!(
            tree_paths.contains(&vec![
                "test".to_string(),
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "target".to_string()
            ]),
            "deeply nested target should be in tree_lines"
        );
    }

    #[test]
    fn test_jump_auto_expands_partially_expanded() {
        let root_arena = vec![
            dir_node("test", vec![("a", 1), ("x", 2)]),
            dir_node("a", vec![("b", 3)]),
            dir_node("x", vec![]),
            dir_node("b", vec![("target", 4)]),
            file_node("target", 1),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), root_arena, 1);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);

        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["test".to_string(), "a".to_string()]);
        app.update_tree_lines();

        app.search_word = "target".to_string();
        app.recompute_matches();

        assert_eq!(app.match_indices.len(), 1);
        assert!(
            app.match_indices[0].tree_idx.is_none(),
            "match should be hidden because 'a/b' is collapsed"
        );

        app.cursor = 0;
        jump_to_next_match(&mut app, 1);

        let selected = app.selected_line().expect("cursor should point to a line");
        assert_eq!(selected.node.name(), "target");
        let tree_paths: Vec<Vec<String>> = app.tree_lines.iter().map(|l| l.path.clone()).collect();
        assert!(
            tree_paths.contains(&vec![
                "test".to_string(),
                "a".to_string(),
                "b".to_string(),
                "target".to_string()
            ]),
            "target should be in tree_lines after partial expansion"
        );
    }

    #[test]
    fn test_refresh_filtered_lines_keeps_hidden_matches() {
        let root_arena = vec![
            dir_node("test", vec![("a", 1)]),
            dir_node("a", vec![("target", 2)]),
            file_node("target", 1),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), root_arena, 1);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);

        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.search_word = "target".to_string();
        app.recompute_matches();

        assert_eq!(app.match_indices.len(), 1);
        assert!(
            app.match_indices[0].tree_idx.is_none(),
            "match in collapsed subtree should have tree_idx=None"
        );

        app.refresh_filtered_lines();

        assert!(
            !app.match_indices.is_empty(),
            "hidden matches must survive refresh_filtered_lines"
        );
        assert_eq!(
            app.match_indices[0].tree_idx, None,
            "hidden match should still have tree_idx=None after refresh"
        );

        app.cursor = 0;
        jump_to_next_match(&mut app, 1);
        let selected = app.selected_line().expect("cursor should point to a line");
        assert_eq!(selected.node.name(), "target");
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
    fn test_prev_match_index_basic() {
        let matches = vec![
            SearchMatch {
                path: vec!["a".to_string()],
                tree_idx: Some(0),
                walk_idx: 0,
            },
            SearchMatch {
                path: vec!["b".to_string()],
                tree_idx: Some(1),
                walk_idx: 5,
            },
            SearchMatch {
                path: vec!["c".to_string()],
                tree_idx: Some(2),
                walk_idx: 10,
            },
        ];

        let result = prev_match_index(&matches, 7);
        assert_eq!(result, Some(1));

        let result = prev_match_index(&matches, 5);
        assert_eq!(result, Some(0));
    }

    #[test]
    fn test_prev_match_index_wrap() {
        let matches = vec![
            SearchMatch {
                path: vec!["a".to_string()],
                tree_idx: Some(0),
                walk_idx: 0,
            },
            SearchMatch {
                path: vec!["b".to_string()],
                tree_idx: Some(1),
                walk_idx: 5,
            },
        ];

        let result = prev_match_index(&matches, 0);
        assert_eq!(result, Some(1));
    }

    #[test]
    fn test_prev_match_index_empty() {
        let matches: Vec<SearchMatch> = vec![];
        let result = prev_match_index(&matches, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_next_match_index_basic() {
        let matches = vec![
            SearchMatch {
                path: vec!["a".to_string()],
                tree_idx: Some(0),
                walk_idx: 0,
            },
            SearchMatch {
                path: vec!["b".to_string()],
                tree_idx: Some(1),
                walk_idx: 5,
            },
            SearchMatch {
                path: vec!["c".to_string()],
                tree_idx: Some(2),
                walk_idx: 10,
            },
        ];

        let result = next_match_index(&matches, 3);
        assert_eq!(result, Some(1));

        let result = next_match_index(&matches, 5);
        assert_eq!(result, Some(2));
    }

    #[test]
    fn test_next_match_index_wrap() {
        let matches = vec![
            SearchMatch {
                path: vec!["a".to_string()],
                tree_idx: Some(0),
                walk_idx: 0,
            },
            SearchMatch {
                path: vec!["b".to_string()],
                tree_idx: Some(1),
                walk_idx: 5,
            },
        ];

        let result = next_match_index(&matches, 10);
        assert_eq!(result, Some(0));
    }

    #[test]
    fn test_next_match_index_empty() {
        let matches: Vec<SearchMatch> = vec![];
        let result = next_match_index(&matches, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_expand_ancestor_prefixes_basic() {
        let mut expanded = std::collections::HashSet::new();
        expanded.insert(vec!["root".to_string()]);

        let result = expand_ancestor_prefixes(
            &mut expanded,
            &[
                "root".to_string(),
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
            ],
        );

        assert_eq!(result.len(), 3);
        assert!(expanded.contains(&vec!["root".to_string(), "a".to_string()]));
        assert!(expanded.contains(&vec!["root".to_string(), "a".to_string(), "b".to_string()]));
        assert!(expanded.contains(&vec![
            "root".to_string(),
            "a".to_string(),
            "b".to_string(),
            "c".to_string()
        ]));
    }

    #[test]
    fn test_expand_ancestor_prefixes_short_path() {
        let mut expanded = std::collections::HashSet::new();
        expanded.insert(vec!["root".to_string()]);

        let result = expand_ancestor_prefixes(&mut expanded, &["root".to_string()]);

        assert!(result.is_empty());
    }

    #[test]
    fn test_expand_ancestor_prefixes_skips_existing() {
        let mut expanded = std::collections::HashSet::new();
        expanded.insert(vec!["root".to_string()]);
        expanded.insert(vec!["root".to_string(), "a".to_string()]);

        let result = expand_ancestor_prefixes(
            &mut expanded,
            &["root".to_string(), "a".to_string(), "b".to_string()],
        );

        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            vec!["root".to_string(), "a".to_string(), "b".to_string()]
        );
    }
}
