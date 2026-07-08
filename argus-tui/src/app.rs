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
    pub selected: bool,
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

    // Tree state
    pub tree_root: Option<TreeNode>,
    pub tree_lines: Vec<TreeLine>,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub expanded: HashSet<String>,

    // Snapshot management
    pub snapshots_dir: PathBuf,
    pub available_snapshots: Vec<SnapshotInfo>,
    pub current_root_path: Option<PathBuf>,
    pub current_snapshot: Option<Snapshot>,

    // Filter state
    pub filter_state: FilterState,

    // Scan state
    pub scan_prompt_open: bool,
    pub scan_path_input: String,
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
    pub fn new(config: crate::config::TuiConfig, tx: mpsc::Sender<AppMessage>, rx: mpsc::Receiver<AppMessage>) -> Self {
        let snapshots_dir = util::default_snapshots_dir();

        Self {
            config,
            tx,
            mode: AppMode::Browsing,
            focus: Focus::Tree,
            sort_mode: SortMode::Delta,
            tree_root: None,
            tree_lines: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            expanded: HashSet::new(),
            snapshots_dir,
            available_snapshots: Vec::new(),
            current_root_path: None,
            current_snapshot: None,
            filter_state: FilterState {
                from_idx: None,
                to_idx: None,
                threshold: None,
                dirty: false,
            },
            scan_prompt_open: false,
            scan_path_input: String::new(),
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

    /// Load snapshots from disk and initialize the tree
    pub fn initialize_from_snapshots(&mut self) {
        let _ = std::fs::create_dir_all(&self.snapshots_dir);

        let snapshots = match self.load_available_snapshots() {
            Ok(s) => s,
            Err(e) => {
                self.last_error = Some(format!("failed to load snapshots: {}", e));
                return;
            }
        };

        if snapshots.is_empty() {
            return; // No snapshots — prompt will show
        }

        // Take the latest snapshot (any root path) as current root
        let latest = snapshots.iter().max_by_key(|s| s.timestamp).unwrap().clone();
        self.available_snapshots = snapshots
            .into_iter()
            .filter(|s| s.path_hash == latest.path_hash)
            .collect();
        self.available_snapshots.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        self.current_root_path = Some(self.snapshots_dir.join(format!(
            "{}_{}.json",
            latest.path_hash,
            latest.timestamp.format("%Y-%m-%dT%H:%M:%SZ")
        )));

        self.load_snapshot_tree(&latest);
    }

    fn load_available_snapshots(&self) -> Result<Vec<SnapshotInfo>, String> {
        let mut snapshots = Vec::new();

        let entries = match std::fs::read_dir(&self.snapshots_dir) {
            Ok(e) => e,
            Err(_) => return Ok(snapshots),
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

            // Parse filename: {hash}_{timestamp}
            if let Some((path_hash, ts_str)) = filename.split_once('_') {
                if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                    snapshots.push(SnapshotInfo {
                        path_hash: path_hash.to_string(),
                        timestamp: ts.with_timezone(&Utc),
                        path,
                    });
                }
            }
        }

        Ok(snapshots)
    }

    fn load_snapshot_tree(&mut self, info: &SnapshotInfo) {
        let content = match std::fs::read_to_string(&info.path) {
            Ok(c) => c,
            Err(e) => {
                self.last_error = Some(format!("failed to read snapshot: {}", e));
                return;
            }
        };

        let snapshot: Snapshot = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(e) => {
                self.last_error = Some(format!("failed to parse snapshot: {}", e));
                return;
            }
        };

        let root_path = snapshot.root_path.clone();
        self.current_root_path = Some(root_path);
        self.current_snapshot = Some(snapshot.clone());
        self.tree_root = Some(TreeNode::Snapshot(snapshot.root_node.clone()));
        self.update_tree_lines();
    }

    /// Update the tree lines from current tree_root
    pub fn update_tree_lines(&mut self) {
        let expanded = &self.expanded;
        let sort_mode = self.sort_mode;

        let lines = match &self.tree_root {
            Some(TreeNode::Snapshot(root)) => {
                let mut lines = Vec::new();
                flatten_snapshot_tree(root, 0, expanded, sort_mode, &mut lines);
                lines
            }
            Some(TreeNode::Diff(root)) => {
                let mut lines = Vec::new();
                flatten_diff_tree(root, 0, expanded, sort_mode, &mut lines);
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
            AppMessage::ScanProgress { file_count, total_bytes } => {
                self.scan_progress = Some((file_count, total_bytes));
            }
            AppMessage::ScanComplete(snapshot) => {
                self.scanning = false;
                self.scan_progress = None;
                self.current_snapshot = Some(snapshot.clone());
                self.current_root_path = Some(snapshot.root_path.clone());
                self.tree_root = Some(TreeNode::Snapshot(snapshot.root_node.clone()));
                self.cursor = 0;
                self.expanded.clear();
                self.filter_state.clear();
                self.update_tree_lines();
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
                self.error_clear_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
            }
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

    /// Get the full path of the selected node for deletion
    pub fn selected_node_full_path(&self) -> Option<PathBuf> {
        let line = self.selected_line()?;
        let root = self.current_root_path.as_ref()?;
        let node_name = line.node.name();
        if node_name == root.file_name()?.to_string_lossy() {
            return Some(root.clone());
        }
        // Compute relative path by traversing up from root
        Some(root.join(node_name))
    }
}

// ── Tree flattening ─────────────────────────────────────────────────────────

fn flatten_snapshot_tree(
    node: &FileNode,
    depth: usize,
    expanded: &HashSet<String>,
    sort_mode: SortMode,
    lines: &mut Vec<TreeLine>,
) {
    let path_key = node.name.clone();
    let is_expanded = expanded.contains(&path_key) || depth == 0;

    lines.push(TreeLine {
        depth,
        node: TreeNode::Snapshot(node.clone()),
        expanded: is_expanded && node.is_dir && !node.children.is_empty(),
        selected: false,
    });

    if is_expanded && node.is_dir {
        let mut children: Vec<&FileNode> = node.children.values().collect();
        sort_children_snapshot(&mut children, sort_mode);
        for child in children {
            flatten_snapshot_tree(child, depth + 1, expanded, sort_mode, lines);
        }
    }
}

fn flatten_diff_tree(
    node: &DiffNode,
    depth: usize,
    expanded: &HashSet<String>,
    sort_mode: SortMode,
    lines: &mut Vec<TreeLine>,
) {
    let path_key = node.name.clone();
    let is_expanded = expanded.contains(&path_key) || depth == 0;

    lines.push(TreeLine {
        depth,
        node: TreeNode::Diff(node.clone()),
        expanded: is_expanded && node.is_dir && !node.children.is_empty(),
        selected: false,
    });

    if is_expanded && node.is_dir {
        let mut children: Vec<&DiffNode> = node.children.values().collect();
        sort_children_diff(&mut children, sort_mode);
        for child in children {
            flatten_diff_tree(child, depth + 1, expanded, sort_mode, lines);
        }
    }
}

fn sort_children_snapshot(children: &mut Vec<&FileNode>, mode: SortMode) {
    match mode {
        SortMode::Name => children.sort_by(|a, b| a.name.cmp(&b.name)),
        SortMode::Size => children.sort_by(|a, b| b.size.cmp(&a.size)),
        SortMode::Delta => {
            // When no delta, sort by size descending
            children.sort_by(|a, b| b.size.cmp(&a.size));
        }
    }
}

fn sort_children_diff(children: &mut Vec<&DiffNode>, mode: SortMode) {
    match mode {
        SortMode::Name => children.sort_by(|a, b| a.name.cmp(&b.name)),
        SortMode::Delta => children.sort_by(|a, b| b.size_delta.abs().cmp(&a.size_delta.abs())),
        SortMode::Size => children.sort_by(|a, b| b.current_size.cmp(&a.current_size)),
    }
}
