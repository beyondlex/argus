use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use tokio::sync::mpsc;

use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};

// ── Data types ──────────────────────────────────────────────────────────────

/// Messages from background tasks to the UI
#[derive(Debug, Clone)]
pub enum AppMessage {
    ScanProgress { file_count: u64, total_bytes: u64 },
    ScanComplete(Snapshot),
    Error(String),
}

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppMode {
    Browsing,
    DeletePrompt,
    Help,
}

/// Which panel has focus
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Tree,
}

/// Tree filter mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterMode {
    Inactive,
    Input,
    Active,
}

/// A match found by the tree filter — `path` is the full relative path from the view root.
#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub path: Vec<String>,
    pub tree_idx: Option<usize>,
    pub walk_idx: usize,
}

/// Sort mode for tree children
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortMode {
    Name,
    Size,
}

impl SortMode {
    pub fn toggle(self) -> Self {
        match self {
            SortMode::Size => SortMode::Name,
            SortMode::Name => SortMode::Size,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Size => "Size",
            SortMode::Name => "Name",
        }
    }
}

/// Unified tree node for rendering (currently only Snapshot variant, Diff reserved for future daemon mode)
#[derive(Debug, Clone)]
pub enum TreeNode {
    Snapshot(Arc<Snapshot>, NodeIndex),
}

impl TreeNode {
    pub fn node(&self) -> &FileNode {
        match self {
            TreeNode::Snapshot(snap, idx) => snap.node(*idx),
        }
    }

    pub fn name(&self) -> &str {
        &self.node().name
    }

    pub fn is_dir(&self) -> bool {
        self.node().is_dir
    }

    pub fn file_type(&self) -> FileType {
        self.node().file_type
    }

    pub fn current_size(&self) -> u64 {
        self.node().size
    }
}

/// A single line in the flattened tree view
#[derive(Debug, Clone)]
pub struct TreeLine {
    pub depth: usize,
    pub node: TreeNode,
    pub expanded: bool,
    pub has_scan_data: bool,
}

#[derive(Debug, Clone)]
pub struct ScanSummary {
    pub root_path: PathBuf,
    pub total_size: u64,
    pub total_files: u64,
    pub duration: Duration,
}

// ── App ─────────────────────────────────────────────────────────────────────

pub struct App {
    pub config: crate::config::TuiConfig,
    pub mode: AppMode,
    pub focus: Focus,
    pub sort_mode: SortMode,

    // View root (always set, initialized to cwd)
    pub view_root_path: PathBuf,

    // Tree state
    pub tree_root: Option<TreeNode>,
    pub tree_lines: Vec<TreeLine>,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub expanded: HashSet<Vec<String>>,

    // Scan cache: path → full scanned snapshot
    pub scan_cache: HashMap<PathBuf, Snapshot>,

    // Tree filter (fuzzy search)
    pub filter_word: String,
    pub filter_mode: FilterMode,
    pub match_indices: Vec<SearchMatch>,
    pub current_match: usize,

    // gg double-tap tracking
    pub pending_gg: bool,

    // Scan state
    pub scanning: bool,
    pub scan_progress: Option<(u64, u64)>,
    pub scan_spinner: u8,
    pub scan_spinner_tick: Instant,
    pub scan_started_at: Option<Instant>,
    pub last_scan_summary: Option<ScanSummary>,
    pub cancel_scan: Arc<AtomicBool>,

    // Delete state
    pub delete_target_path: Option<PathBuf>,

    // Message channel
    pub tx: mpsc::Sender<AppMessage>,
    pub rx: mpsc::Receiver<AppMessage>,

    // Error display
    pub last_error: Option<String>,
    pub error_clear_at: Option<std::time::Instant>,

    // Log file path (~/.config/argus/argus.log)
    pub log_path: PathBuf,

    // Info popup
    pub info_data: Option<(std::path::PathBuf, std::fs::Metadata)>,

    // Quit
    pub should_quit: bool,
}

impl App {
    pub fn new(
        config: crate::config::TuiConfig,
        tx: mpsc::Sender<AppMessage>,
        rx: mpsc::Receiver<AppMessage>,
    ) -> Self {
        let view_root_path =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let log_path = default_log_path();

        Self {
            config,
            tx,
            mode: AppMode::Browsing,
            focus: Focus::Tree,
            sort_mode: SortMode::Size,
            view_root_path,
            tree_root: None,
            tree_lines: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            expanded: HashSet::new(),
            scan_cache: HashMap::new(),
            filter_word: String::new(),
            filter_mode: FilterMode::Inactive,
            match_indices: Vec::new(),
            current_match: 0,
            pending_gg: false,
            scanning: false,
            scan_progress: None,
            scan_spinner: 0,
            scan_spinner_tick: Instant::now(),
            scan_started_at: None,
            last_scan_summary: None,
            cancel_scan: Arc::new(AtomicBool::new(false)),
            delete_target_path: None,
            info_data: None,
            rx,
            last_error: None,
            error_clear_at: None,
            log_path,
            should_quit: false,
        }
    }

    /// Build tree for current view_root_path from scan_cache or filesystem
    pub fn rebuild_tree(&mut self) {
        self.build_current_tree();
        self.update_tree_lines();
    }

    fn build_current_tree(&mut self) {
        // Use full scan data when available, otherwise fall back to list_dir (one level)
        if let Some(snapshot) = self.scan_cache.get(&self.view_root_path).cloned() {
            self.tree_root = Some(TreeNode::Snapshot(Arc::new(snapshot), ROOT_NODE));
        } else {
            match argus_core::list_dir(&self.view_root_path) {
                Ok(mut snap) => {
                    let root_scan_tree =
                        resolve_scan_tree(&self.scan_cache, &self.view_root_path);
                    enrich_snapshot_sizes(
                        &mut snap,
                        ROOT_NODE,
                        &self.scan_cache,
                        &self.view_root_path,
                        root_scan_tree,
                        &mut Vec::new(),
                    );
                    self.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
                }
                Err(e) => {
                    self.set_error(format!("failed to list directory: {}", e), 3);
                    self.tree_root = None;
                }
            }
        }

        self.cursor = 0;
        self.expanded.clear();
    }

    /// Update the tree lines from current tree_root
    pub fn update_tree_lines(&mut self) {
        let expanded = &self.expanded;
        let sort_mode = self.sort_mode;
        let lines = match &self.tree_root {
            Some(TreeNode::Snapshot(snap_arc, idx)) => {
                let mut lines = Vec::new();
                let root_scan_tree = resolve_scan_tree(&self.scan_cache, &self.view_root_path);
                flatten_snapshot_tree(
                    snap_arc,
                    *idx,
                    0,
                    expanded,
                    sort_mode,
                    &mut lines,
                    &self.scan_cache,
                    &self.view_root_path,
                    root_scan_tree,
                    &mut Vec::new(),
                );
                lines
            }
            _ => Vec::new(),
        };

        self.tree_lines = lines;
        if self.cursor >= self.tree_lines.len() && !self.tree_lines.is_empty() {
            self.cursor = self.tree_lines.len() - 1;
        }
    }

    /// Handle a message from background tasks
    pub fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::ScanProgress {
                file_count,
                total_bytes,
            } => {
                self.scan_progress = Some((file_count, total_bytes));
            }
            AppMessage::ScanComplete(snapshot) => {
                self.scanning = false;
                let items = self.scan_progress.map(|(count, _)| count).unwrap_or(0);
                let duration = self
                    .scan_started_at
                    .take()
                    .map(|started| started.elapsed())
                    .unwrap_or_default();
                self.last_scan_summary = Some(ScanSummary {
                    root_path: snapshot.root_path.clone(),
                    total_size: snapshot.total_size,
                    total_files: items,
                    duration,
                });
                self.scan_progress = None;

                // Update scan cache
                self.scan_cache
                    .insert(snapshot.root_path.clone(), snapshot.clone());

                // Rebuild tree if scanned path matches current view
                if snapshot.root_path == self.view_root_path {
                    self.rebuild_tree();
                }
            }
            AppMessage::Error(e) => {
                self.scanning = false;
                self.scan_started_at = None;
                self.set_error(e, 5);
            }
        }
    }

    /// Recompute match_indices for current filter_word.
    /// Walks the full tree in display order (depth-first, sorted by sort_mode)
    /// so n/N jumps follow the natural top-to-bottom order.
    pub fn recompute_matches(&mut self) {
        self.match_indices.clear();
        self.current_match = 0;
        if self.filter_word.is_empty() {
            return;
        }

        let Some(TreeNode::Snapshot(snap_arc, root_idx)) = &self.tree_root else {
            return;
        };

        let query = &self.filter_word;
        let expanded = &self.expanded;
        let sort_mode = self.sort_mode;

        let mut matches = Vec::new();
        let mut walk_index = 0usize;
        let mut visible_count = 0usize;
        let mut current_path = vec![snap_arc.node(*root_idx).name.clone()];
        collect_matches_in_order(
            snap_arc,
            *root_idx,
            query,
            expanded,
            sort_mode,
            &mut current_path,
            &mut walk_index,
            &mut visible_count,
            &mut matches,
        );

        self.match_indices = matches;
        if self.current_match >= self.match_indices.len() && !self.match_indices.is_empty() {
            self.current_match = self.match_indices.len() - 1;
        }
    }

    /// Get the currently selected tree line
    pub fn selected_line(&self) -> Option<&TreeLine> {
        self.tree_lines.get(self.cursor)
    }

    /// Set error message and log to file.
    pub fn set_error(&mut self, msg: String, duration_secs: u64) {
        self.last_error = Some(msg.clone());
        self.error_clear_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(duration_secs));
        if let Ok(ts) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            let line = format!("[{}] {}\n", ts.as_secs(), msg);
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
                .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        }
    }

    /// Return the relative path of the tree line at `idx`, rooted at `view_root_path`.
    pub fn tree_line_relative_path(&self, idx: usize) -> Option<Vec<String>> {
        let line = self.tree_lines.get(idx)?;
        if line.depth == 0 {
            return Some(vec![line.node.name().to_string()]);
        }

        let mut ancestors = Vec::new();
        let mut target_depth = line.depth;

        for i in (0..idx).rev() {
            let l = &self.tree_lines[i];
            if l.depth < target_depth {
                ancestors.push(l.node.name().to_string());
                target_depth = l.depth;
            }
        }

        ancestors.reverse();
        ancestors.push(line.node.name().to_string());
        Some(ancestors)
    }

    /// Get the full path of the selected node
    pub fn selected_node_full_path(&self) -> Option<PathBuf> {
        let mut path = self.view_root_path.clone();
        let relative = self.tree_line_relative_path(self.cursor)?;
        // relative includes the root dir name as the first component;
        // skip it because view_root_path already contains the full root path.
        for part in relative.iter().skip(1) {
            path.push(part);
        }
        Some(path)
    }
}

// ── Tree flattening ─────────────────────────────────────────────────────────

fn flatten_snapshot_tree(
    snap_arc: &Arc<Snapshot>,
    idx: NodeIndex,
    depth: usize,
    expanded: &HashSet<Vec<String>>,
    sort_mode: SortMode,
    lines: &mut Vec<TreeLine>,
    scan_cache: &HashMap<PathBuf, Snapshot>,
    view_root_path: &Path,
    root_scan_tree: Option<(&Snapshot, NodeIndex)>,
    path: &mut Vec<String>,
) {
    let node = snap_arc.node(idx);
    path.push(node.name.clone());
    let path_key = path.clone();
    let is_expanded = depth == 0 || expanded.contains(&path_key);

    let node_has_scan =
        has_snapshot_for_path(scan_cache, view_root_path, root_scan_tree, &path_key);

    lines.push(TreeLine {
        depth,
        node: TreeNode::Snapshot(Arc::clone(snap_arc), idx),
        expanded: is_expanded && node.is_dir && !node.children.is_empty(),
        has_scan_data: node_has_scan || !node.is_dir,
    });

    if is_expanded && node.is_dir {
        let mut children: Vec<(&String, NodeIndex)> =
            node.children.iter().map(|(n, i)| (n, *i)).collect();
        sort_children_snapshot(&mut children, snap_arc, sort_mode);
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
                path,
            );
        }
    }

    path.pop();
}

fn enrich_snapshot_sizes(
    snap: &mut Snapshot,
    idx: NodeIndex,
    scan_cache: &HashMap<PathBuf, Snapshot>,
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

fn size_for_path(
    scan_cache: &HashMap<PathBuf, Snapshot>,
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
        find_snapshot_node(snap, idx, path_key).map(|found_idx| snap.node(found_idx).size)
    })
}

fn has_snapshot_for_path(
    scan_cache: &HashMap<PathBuf, Snapshot>,
    view_root_path: &Path,
    root_scan_tree: Option<(&Snapshot, NodeIndex)>,
    path_key: &[String],
) -> bool {
    if path_key.is_empty() {
        return false;
    }

    let mut path = view_root_path.to_path_buf();
    for component in path_key.iter().skip(1) {
        path.push(component);
    }

    if scan_cache.contains_key(&path) {
        return true;
    }

    root_scan_tree
        .and_then(|(snap, idx)| find_snapshot_node(snap, idx, path_key))
        .is_some()
}

/// Find the best-available scan tree node for a view path.
///
/// First tries an exact match in scan_cache. If not found, walks up
/// the path hierarchy to find a parent-level scan, then walks down
/// the scan tree to find the subtree matching the view root.
pub(crate) fn resolve_scan_tree<'a>(
    scan_cache: &'a HashMap<PathBuf, Snapshot>,
    view_root_path: &Path,
) -> Option<(&'a Snapshot, NodeIndex)> {
    if let Some(snapshot) = scan_cache.get(view_root_path) {
        return Some((snapshot, ROOT_NODE));
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
            return Some((snapshot, idx));
        }
        parent = parent.parent()?;
    }
}

fn find_snapshot_node(
    snap: &Snapshot,
    idx: NodeIndex,
    target_path: &[String],
) -> Option<NodeIndex> {
    let node = snap.node(idx);
    let (head, tail) = target_path.split_first()?;
    if node.name != *head {
        return None;
    }
    if tail.is_empty() {
        return Some(idx);
    }

    let child_idx = node.child_idx(&tail[0])?;
    find_snapshot_node(snap, child_idx, tail)
}

fn sort_children_snapshot(
    children: &mut Vec<(&String, NodeIndex)>,
    snap: &Snapshot,
    mode: SortMode,
) {
    match mode {
        SortMode::Name => children.sort_by(|a, b| a.0.cmp(b.0)),
        SortMode::Size => children.sort_by(|a, b| {
            let a_size = snap.node(a.1).size;
            let b_size = snap.node(b.1).size;
            b_size.cmp(&a_size)
        }),
    }
}

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

/// Default path for the log file: ~/.config/argus/argus.log
pub fn default_log_path() -> PathBuf {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_else(|| std::ffi::OsString::from("/tmp"));
            PathBuf::from(home).join(".config")
        });
    let dir = config_dir.join("argus");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("argus.log")
}

/// Walk the full tree in depth-first display order (children sorted by sort_mode).
/// - `visible_count`: tracks position in tree_lines for visible nodes
/// - Matches are pushed in walk order so n/N follows natural top-to-bottom flow
/// - Visible matches get `tree_idx = Some(pos)`, collapsed matches get `None`
#[allow(clippy::too_many_arguments)]
fn collect_matches_in_order(
    snap: &Snapshot,
    idx: NodeIndex,
    query: &str,
    expanded: &HashSet<Vec<String>>,
    sort_mode: SortMode,
    path: &mut Vec<String>,
    walk_index: &mut usize,
    visible_count: &mut usize,
    result: &mut Vec<SearchMatch>,
) {
    let node = snap.node(idx);
    let is_visible = path_is_visible(path, expanded);

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
        let mut children: Vec<(&String, NodeIndex)> =
            node.children.iter().map(|(n, i)| (n, *i)).collect();
        sort_children_snapshot(&mut children, snap, sort_mode);
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
            );
            path.pop();
        }
    }
}

fn path_is_visible(path: &[String], expanded: &HashSet<Vec<String>>) -> bool {
    if path.len() <= 1 {
        return true;
    }

    (1..path.len()).all(|len| expanded.contains(&path[..len].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TuiConfig;
    use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tokio::sync::mpsc;

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

        // Live tree arena (simulating list_dir output — dirs have 0 size)
        let live_arena = vec![
            node("test", true, 0, vec![("target", 1)]),
            node("target", true, 0, vec![("debug", 2)]),
            node("debug", true, 0, vec![("build", 3)]),
            node("build", true, 0, vec![("build-script-build", 4)]),
            node("build-script-build", false, 475_880, vec![]),
        ];
        let mut live_snap = Snapshot::new(root_path.clone(), live_arena, 0);

        // Scan snapshot arena (has proper sizes from a real scan)
        let scan_arena = vec![
            node("test", true, 475_880, vec![("target", 1)]),
            node("target", true, 475_880, vec![("debug", 2)]),
            node("debug", true, 475_880, vec![("build", 3)]),
            node("build", true, 475_880, vec![("build-script-build", 4)]),
            node("build-script-build", false, 475_880, vec![]),
        ];
        let scan_snap = Snapshot::new(root_path.clone(), scan_arena, 475_880);
        let mut scan_cache = HashMap::new();
        scan_cache.insert(root_path.clone(), scan_snap);
        let root_scan_tree = resolve_scan_tree(&scan_cache, &root_path);

        enrich_snapshot_sizes(
            &mut live_snap,
            ROOT_NODE,
            &scan_cache,
            &root_path,
            root_scan_tree,
            &mut Vec::new(),
        );

        // build dir (index 3) should now have the file's size
        assert_eq!(live_snap.node(3).size, 475_880);
    }

    #[test]
    fn test_unlisted_child_dir_keeps_dash_even_when_root_is_scanned() {
        let root_path = PathBuf::from("/tmp/test");

        // Live tree: test/target/{debug, build}  — "build" is NOT in the scan
        let live_arena = vec![
            node("test", true, 0, vec![("target", 1)]),
            node("target", true, 0, vec![("debug", 2), ("build", 3)]),
            node("debug", true, 0, vec![]),
            node("build", true, 0, vec![]),
        ];
        let live_snap = Snapshot::new(root_path.clone(), live_arena, 0);

        // Scan cache: test/target/debug only
        let scan_arena = vec![
            node("test", true, 0, vec![("target", 1)]),
            node("target", true, 0, vec![("debug", 2)]),
            node("debug", true, 0, vec![]),
        ];
        let scan_snap = Snapshot::new(root_path.clone(), scan_arena, 0);

        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, rx);
        app.view_root_path = root_path.clone();
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(live_snap), ROOT_NODE));
        app.scan_cache.insert(root_path.clone(), scan_snap);

        app.expanded
            .insert(vec!["test".to_string(), "target".to_string()]);

        app.update_tree_lines();

        let build_line = app
            .tree_lines
            .iter()
            .find(|line| line.node.name() == "build")
            .expect("build line should exist");
        assert!(build_line.node.is_dir());
        assert!(!build_line.has_scan_data);
        assert_eq!(build_line.node.current_size(), 0);
    }
}
