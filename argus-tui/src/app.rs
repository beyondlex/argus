use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

use argus_core::{filter_by_threshold, DiffNode, FileNode, Snapshot};

use crate::util;

// ── Data types ──────────────────────────────────────────────────────────────

/// Messages from background tasks to the UI
#[derive(Debug, Clone)]
pub enum AppMessage {
    ScanProgress { file_count: u64, total_bytes: u64 },
    ScanComplete(Snapshot),
    DiffComplete(DiffNode),
    Error(String),
}

/// Commands from UI to background tasks
#[allow(dead_code)]
pub enum AppCommand {
    Scan {
        path: PathBuf,
        cancel: Arc<AtomicBool>,
        tx: mpsc::Sender<AppMessage>,
    },
    Diff {
        old_hash: String,
        old_ts: DateTime<Utc>,
        new_hash: String,
        new_ts: DateTime<Utc>,
        tx: mpsc::Sender<AppMessage>,
    },
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
    FilterBar,
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
    Delta,
    Size,
}

impl SortMode {
    pub fn toggle(self) -> Self {
        match self {
            SortMode::Delta => SortMode::Size,
            SortMode::Size => SortMode::Name,
            SortMode::Name => SortMode::Delta,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Delta => "Δ",
            SortMode::Size => "Size",
            SortMode::Name => "Name",
        }
    }
}

/// Unified tree node for rendering
#[derive(Debug, Clone)]
pub enum TreeNode {
    Snapshot(FileNode),
    Diff(DiffNode),
}

impl TreeNode {
    pub fn name(&self) -> &str {
        match self {
            TreeNode::Snapshot(n) => &n.name,
            TreeNode::Diff(n) => &n.name,
        }
    }

    pub fn is_dir(&self) -> bool {
        match self {
            TreeNode::Snapshot(n) => n.is_dir,
            TreeNode::Diff(n) => n.is_dir,
        }
    }

    pub fn current_size(&self) -> u64 {
        match self {
            TreeNode::Snapshot(n) => n.size,
            TreeNode::Diff(n) => n.current_size,
        }
    }

    pub fn size_delta(&self) -> i64 {
        match self {
            TreeNode::Snapshot(_) => 0,
            TreeNode::Diff(n) => n.size_delta,
        }
    }

    #[allow(dead_code)]
    pub fn children_snapshot(&self) -> Option<&HashMap<String, FileNode>> {
        match self {
            TreeNode::Snapshot(n) => Some(&n.children),
            _ => None,
        }
    }

    pub fn children_diff(&self) -> Option<&HashMap<String, DiffNode>> {
        match self {
            TreeNode::Diff(n) => Some(&n.children),
            _ => None,
        }
    }

    pub fn modified(&self) -> Option<DateTime<Utc>> {
        match self {
            TreeNode::Snapshot(n) => n.modified,
            TreeNode::Diff(_) => None,
        }
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

/// Snapshot metadata parsed from filename
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub path_hash: String,
    pub timestamp: DateTime<Utc>,
    pub path: PathBuf,
}

/// Filter bar state
#[derive(Debug, Clone)]
pub struct FilterState {
    pub from_idx: Option<usize>,
    pub to_idx: Option<usize>,
    pub threshold: Option<u64>,
    pub dirty: bool,
}

impl FilterState {
    pub fn is_empty(&self) -> bool {
        self.from_idx.is_none() && self.to_idx.is_none() && self.threshold.is_none()
    }

    pub fn should_diff(&self) -> bool {
        self.from_idx.is_some() && self.to_idx.is_some()
    }

    pub fn clear(&mut self) {
        self.from_idx = None;
        self.to_idx = None;
        self.threshold = None;
        self.dirty = false;
    }
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

    // Snapshot index: path_hash → all available snapshots for that path
    pub snapshot_index: HashMap<String, Vec<SnapshotInfo>>,

    // Snapshots scoped to current view_root_path's hash
    pub available_snapshots: Vec<SnapshotInfo>,

    pub snapshots_dir: PathBuf,

    // Diff filter state
    pub filter_state: FilterState,

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
    pub cancel_scan: Arc<AtomicBool>,

    // Delete state
    pub delete_target_path: Option<PathBuf>,

    // Message channel
    pub tx: mpsc::Sender<AppMessage>,
    pub rx: mpsc::Receiver<AppMessage>,

    // Error display
    pub last_error: Option<String>,
    pub error_clear_at: Option<std::time::Instant>,

    // Quit
    pub should_quit: bool,
}

impl App {
    pub fn new(
        config: crate::config::TuiConfig,
        tx: mpsc::Sender<AppMessage>,
        rx: mpsc::Receiver<AppMessage>,
    ) -> Self {
        let snapshots_dir = util::default_snapshots_dir();
        let view_root_path =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        Self {
            config,
            tx,
            mode: AppMode::Browsing,
            focus: Focus::Tree,
            sort_mode: SortMode::Delta,
            view_root_path,
            tree_root: None,
            tree_lines: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            expanded: HashSet::new(),
            scan_cache: HashMap::new(),
            snapshot_index: HashMap::new(),
            available_snapshots: Vec::new(),
            snapshots_dir,
            filter_state: FilterState {
                from_idx: None,
                to_idx: None,
                threshold: None,
                dirty: false,
            },
            filter_word: String::new(),
            filter_mode: FilterMode::Inactive,
            match_indices: Vec::new(),
            current_match: 0,
            pending_gg: false,
            scanning: false,
            scan_progress: None,
            cancel_scan: Arc::new(AtomicBool::new(false)),
            delete_target_path: None,
            rx,
            last_error: None,
            error_clear_at: None,
            should_quit: false,
        }
    }

    /// Load all snapshots from disk into scan_cache and snapshot_index
    pub fn load_all_snapshots(&mut self) {
        let _ = std::fs::create_dir_all(&self.snapshots_dir);

        let entries = match std::fs::read_dir(&self.snapshots_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            let filename = match path.file_stem().and_then(|s| s.to_str()) {
                Some(f) => f.to_string(),
                None => continue,
            };

            let Some((path_hash, ts_str)) = filename.split_once('_') else {
                continue;
            };
            let Ok(ts) = chrono::DateTime::parse_from_rfc3339(ts_str) else {
                continue;
            };
            let ts = ts.with_timezone(&Utc);

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let snapshot: Snapshot = match serde_json::from_str(&content) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Index by root path for direct lookup
            self.scan_cache
                .insert(snapshot.root_path.clone(), snapshot.clone());

            // Index by hash for filter bar
            self.snapshot_index
                .entry(path_hash.to_string())
                .or_default()
                .push(SnapshotInfo {
                    path_hash: path_hash.to_string(),
                    timestamp: ts,
                    path,
                });
        }

        // Sort each snapshot_index entry by timestamp
        for snapshots in self.snapshot_index.values_mut() {
            snapshots.sort_by_key(|a| a.timestamp);
        }
    }

    /// Build tree for current view_root_path from scan_cache or filesystem
    pub fn rebuild_tree(&mut self) {
        // Scope available_snapshots to current view_root_path's hash
        let path_hash = argus_core::hash_root_path(&self.view_root_path);
        self.available_snapshots = self
            .snapshot_index
            .get(&path_hash)
            .cloned()
            .unwrap_or_default();
        self.available_snapshots.sort_by_key(|a| a.timestamp);

        // Check scan cache first
        if let Some(snapshot) = self.scan_cache.get(&self.view_root_path) {
            self.tree_root = Some(TreeNode::Snapshot(snapshot.root_node.clone()));
        } else {
            // Fall back to FS listing
            match argus_core::list_dir(&self.view_root_path) {
                Ok(node) => {
                    self.tree_root = Some(TreeNode::Snapshot(node));
                }
                Err(e) => {
                    self.last_error = Some(format!("failed to list directory: {}", e));
                    self.error_clear_at =
                        Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                    self.tree_root = None;
                }
            }
        }

        self.cursor = 0;
        self.expanded.clear();
        self.filter_state.clear();
        self.update_tree_lines();
    }

    /// Update the tree lines from current tree_root
    pub fn update_tree_lines(&mut self) {
        let expanded = &self.expanded;
        let sort_mode = self.sort_mode;
        let has_scan_data = self.scan_cache.contains_key(&self.view_root_path);

        let lines = match &self.tree_root {
            Some(TreeNode::Snapshot(root)) => {
                let mut lines = Vec::new();
                flatten_snapshot_tree(
                    root,
                    0,
                    expanded,
                    sort_mode,
                    &mut lines,
                    has_scan_data,
                    &mut Vec::new(),
                );
                lines
            }
            Some(TreeNode::Diff(root)) => {
                let mut lines = Vec::new();
                flatten_diff_tree(
                    root,
                    0,
                    expanded,
                    sort_mode,
                    &mut lines,
                    has_scan_data,
                    &mut Vec::new(),
                );
                lines
            }
            None => Vec::new(),
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
                self.scan_progress = None;

                // Update scan cache
                self.scan_cache
                    .insert(snapshot.root_path.clone(), snapshot.clone());

                // Update snapshot index
                let hash = argus_core::hash_root_path(&snapshot.root_path);
                let info = SnapshotInfo {
                    path_hash: hash.clone(),
                    timestamp: snapshot.timestamp,
                    path: self.snapshots_dir.join(format!(
                        "{}_{}.json",
                        hash,
                        snapshot.timestamp.format("%Y-%m-%dT%H:%M:%SZ")
                    )),
                };
                self.snapshot_index.entry(hash).or_default().push(info);
                // Re-sort after insertion
                if let Some(snapshots) = self
                    .snapshot_index
                    .get_mut(&argus_core::hash_root_path(&snapshot.root_path))
                {
                    snapshots.sort_by_key(|a| a.timestamp);
                }

                // Rebuild tree if scanned path matches current view
                if snapshot.root_path == self.view_root_path {
                    self.rebuild_tree();
                }
            }
            AppMessage::DiffComplete(diff) => {
                // Apply threshold filter if set
                let filtered = if let Some(thresh) = self.filter_state.threshold {
                    filter_by_threshold(&diff, thresh).unwrap_or(diff)
                } else {
                    diff
                };
                self.tree_root = Some(TreeNode::Diff(filtered));
                self.cursor = 0;
                self.expanded.clear();
                self.update_tree_lines();
            }
            AppMessage::Error(e) => {
                self.scanning = false;
                self.last_error = Some(e);
                self.error_clear_at =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
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

        let Some(TreeNode::Snapshot(ref root)) = self.tree_root else {
            return;
        };

        let query = &self.filter_word;
        let expanded = &self.expanded;
        let sort_mode = self.sort_mode;

        let mut matches = Vec::new();
        let mut walk_index = 0usize;
        let mut visible_count = 0usize;
        let mut current_path = vec![root.name.clone()];
        collect_matches_in_order(
            root,
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

    /// Check if delta column should be shown
    pub fn has_delta_column(&self) -> bool {
        matches!(&self.tree_root, Some(TreeNode::Diff(_)))
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
        for part in relative {
            path.push(part);
        }
        Some(path)
    }
}

// ── Tree flattening ─────────────────────────────────────────────────────────

fn flatten_snapshot_tree(
    node: &FileNode,
    depth: usize,
    expanded: &HashSet<Vec<String>>,
    sort_mode: SortMode,
    lines: &mut Vec<TreeLine>,
    has_scan_data: bool,
    path: &mut Vec<String>,
) {
    path.push(node.name.clone());
    let path_key = path.clone();
    let is_expanded = depth == 0 || expanded.contains(&path_key);

    lines.push(TreeLine {
        depth,
        node: TreeNode::Snapshot(node.clone()),
        expanded: is_expanded && node.is_dir && !node.children.is_empty(),
        has_scan_data: has_scan_data || !node.is_dir,
    });

    if is_expanded && node.is_dir {
        let mut children: Vec<&FileNode> = node.children.values().collect();
        sort_children_snapshot(&mut children, sort_mode);
        for child in children {
            flatten_snapshot_tree(
                child,
                depth + 1,
                expanded,
                sort_mode,
                lines,
                has_scan_data,
                path,
            );
        }
    }

    path.pop();
}

fn flatten_diff_tree(
    node: &DiffNode,
    depth: usize,
    expanded: &HashSet<Vec<String>>,
    sort_mode: SortMode,
    lines: &mut Vec<TreeLine>,
    _has_scan_data: bool,
    path: &mut Vec<String>,
) {
    path.push(node.name.clone());
    let path_key = path.clone();
    let is_expanded = depth == 0 || expanded.contains(&path_key);

    lines.push(TreeLine {
        depth,
        node: TreeNode::Diff(node.clone()),
        expanded: is_expanded && node.is_dir && !node.children.is_empty(),
        has_scan_data: true,
    });

    if is_expanded && node.is_dir {
        let mut children: Vec<&DiffNode> = node.children.values().collect();
        sort_children_diff(&mut children, sort_mode);
        for child in children {
            flatten_diff_tree(
                child,
                depth + 1,
                expanded,
                sort_mode,
                lines,
                _has_scan_data,
                path,
            );
        }
    }

    path.pop();
}

fn sort_children_snapshot(children: &mut Vec<&FileNode>, mode: SortMode) {
    match mode {
        SortMode::Name => children.sort_by(|a, b| a.name.cmp(&b.name)),
        SortMode::Size => children.sort_by_key(|b| std::cmp::Reverse(b.size)),
        SortMode::Delta => children.sort_by_key(|b| std::cmp::Reverse(b.size)),
    }
}

fn sort_children_diff(children: &mut Vec<&DiffNode>, mode: SortMode) {
    match mode {
        SortMode::Name => children.sort_by(|a, b| a.name.cmp(&b.name)),
        SortMode::Delta => children.sort_by_key(|b| std::cmp::Reverse(b.size_delta.abs())),
        SortMode::Size => children.sort_by_key(|b| std::cmp::Reverse(b.current_size)),
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

/// Walk the full tree in depth-first display order (children sorted by sort_mode).
/// - `visible_count`: tracks position in tree_lines for visible nodes
/// - Matches are pushed in walk order so n/N follows natural top-to-bottom flow
/// - Visible matches get `tree_idx = Some(pos)`, collapsed matches get `None`
#[allow(clippy::too_many_arguments)]
fn collect_matches_in_order(
    node: &FileNode,
    query: &str,
    expanded: &HashSet<Vec<String>>,
    sort_mode: SortMode,
    path: &mut Vec<String>,
    walk_index: &mut usize,
    visible_count: &mut usize,
    result: &mut Vec<SearchMatch>,
) {
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
        let mut children: Vec<&FileNode> = node.children.values().collect();
        sort_children_snapshot(&mut children, sort_mode);
        for child in children {
            path.push(child.name.clone());
            collect_matches_in_order(
                child,
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
