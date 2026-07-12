use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use tokio::sync::mpsc;

use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};

use crate::ipc_client::IpcClient;

/// Directories with more children than this won't have their subtree matched or expanded
/// during search navigation. Prevents n/N from hanging when jumping into massive directories.
const MAX_DIR_CHILDREN: usize = 2000;

// ── Data types ──────────────────────────────────────────────────────────────

/// Messages from background tasks to the UI
#[derive(Debug)]
pub enum AppMessage {
    ScanProgress { file_count: u64, total_bytes: u64 },
    ScanComplete(Snapshot),
    DaemonConnected(IpcClient),
    DeltaData(HashMap<Vec<String>, i64>),
    Error(String),
    Info(String),
}

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppMode {
    Browsing,
    DeletePrompt,
    DeletePermanentPrompt,
    Help,
    Command,
}

/// Which panel has focus
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Tree,
    FilterPane,
}

/// Which field in the filter pane has focus
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterFocus {
    TimePreset,
    DeltaValue,
    DeltaUnit,
}

/// Multiplier for delta filter unit index
pub fn delta_unit_multiplier(unit: usize) -> u64 {
    match unit {
        0 => 1024,               // KB
        1 => 1024 * 1024,        // MB
        2 => 1024 * 1024 * 1024, // GB
        _ => 1,
    }
}

pub const DELTA_UNIT_LABELS: &[&str] = &["KB", "MB", "GB"];

pub const TIME_PRESET_COUNT: usize = 7;

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
    pub path: Vec<String>,
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

    // Server mode (connected to daemon)
    pub server_mode: bool,
    pub daemon_client: Option<IpcClient>,
    pub server_connected: bool,
    pub delta_cache: HashMap<Vec<String>, i64>,
    pub time_from: u64,
    pub time_to: u64,
    pub time_preset: usize,

    // Filter pane state
    pub filter_focus: FilterFocus,
    pub delta_filter_active: bool,
    pub delta_filter_value: u64,
    pub delta_filter_unit: usize, // 0=KB, 1=MB, 2=GB
    pub delta_pending: bool,
    pub filtered_tree_lines: Vec<usize>,

    // Tree filter (fuzzy search)
    pub filter_word: String,
    pub filter_mode: FilterMode,
    pub match_indices: Vec<SearchMatch>,
    pub current_match: usize,
    pub path_to_walk_idx: HashMap<Vec<String>, usize>,

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

    // Command bar
    pub command_input: String,
    pub command_matches: Vec<&'static str>,
    pub command_selected: usize,

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
            server_mode: false,
            daemon_client: None,
            server_connected: false,
            delta_cache: HashMap::new(),
            time_from: 0,
            time_to: 0,
            time_preset: 0,
            filter_focus: FilterFocus::TimePreset,
            delta_filter_active: false,
            delta_filter_value: 100,
            delta_filter_unit: 1,
            delta_pending: false,
            filtered_tree_lines: Vec::new(),
            filter_word: String::new(),
            filter_mode: FilterMode::Inactive,
            match_indices: Vec::new(),
            current_match: 0,
            path_to_walk_idx: HashMap::new(),
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
            command_input: String::new(),
            command_matches: Vec::new(),
            command_selected: 0,
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
        if self.server_mode {
            self.request_delta_refresh();
        }
    }

    fn build_current_tree(&mut self) {
        // Use full scan data when available, otherwise fall back to list_dir (one level)
        if let Some(snapshot) = self.scan_cache.get(&self.view_root_path).cloned() {
            self.tree_root = Some(TreeNode::Snapshot(Arc::new(snapshot), ROOT_NODE));
        } else {
            match argus_core::list_dir(&self.view_root_path) {
                Ok(mut snap) => {
                    let root_scan_tree = resolve_scan_tree(&self.scan_cache, &self.view_root_path);
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
        self.refresh_filtered_lines();
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
            AppMessage::DaemonConnected(client) => {
                self.server_mode = true;
                self.server_connected = true;
                self.daemon_client = Some(client);
                self.default_time_range();
                self.set_error("connected to daemon".into(), 2);
                self.request_delta_refresh();
            }
            AppMessage::DeltaData(deltas) => {
                let t0 = Instant::now();
                self.delta_pending = false;
                self.delta_cache = deltas;
                self.refresh_filtered_lines();
                log_msg(
                    &self.log_path,
                    &format!("DeltaData applied in {:?}", t0.elapsed()),
                );
            }
            AppMessage::Info(msg) => {
                self.set_error(msg, 4);
            }
        }
    }

    /// Recompute match_indices for current filter_word.
    /// Walks the full tree in display order (depth-first, sorted by sort_mode)
    /// so n/N jumps follow the natural top-to-bottom order.
    /// Also populates path_to_walk_idx cache to avoid a second full tree walk on n/N.
    pub fn recompute_matches(&mut self) {
        self.match_indices.clear();
        self.current_match = 0;
        self.path_to_walk_idx.clear();
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
            &mut self.path_to_walk_idx,
        );

        self.match_indices = matches;
        if self.current_match >= self.match_indices.len() && !self.match_indices.is_empty() {
            self.current_match = self.match_indices.len() - 1;
        }
    }

    /// O(1) lookup for walk_idx of a path, using the cache built during recompute_matches.
    pub fn get_walk_idx(&self, path: &[String]) -> Option<usize> {
        self.path_to_walk_idx.get(path).copied()
    }

    /// Incrementally expand a single collapsed directory in tree_lines by inserting child lines.
    /// Unlike update_tree_lines() which rebuilds the entire visible tree, this only adds the
    /// newly visible lines for the specified path. Returns true if any lines were inserted.
    pub fn expand_path_in_tree(&mut self, path: &[String]) -> bool {
        let Some(TreeNode::Snapshot(snap_arc, root_idx)) = &self.tree_root else {
            return false;
        };
        let Some(dir_idx) = find_snapshot_node(snap_arc, *root_idx, path) else {
            return false;
        };
        let node = snap_arc.node(dir_idx);
        if !node.is_dir || node.children.is_empty() || node.children.len() > MAX_DIR_CHILDREN {
            return false;
        }

        let pos = self.tree_lines.iter().position(|line| line.path == path);
        let Some(pos) = pos else {
            return false;
        };

        // Skip if already expanded (next line is a child)
        if pos + 1 < self.tree_lines.len()
            && self.tree_lines[pos + 1].depth > self.tree_lines[pos].depth
        {
            return false;
        }

        // Mark this directory as expanded in-place
        if let Some(line) = self.tree_lines.get_mut(pos) {
            line.expanded = true;
        }

        let mut children: Vec<(&String, NodeIndex)> =
            node.children.iter().map(|(n, i)| (n, *i)).collect();
        sort_children_snapshot(&mut children, snap_arc, self.sort_mode);

        let expanded = &self.expanded;
        let sort_mode = self.sort_mode;
        let root_scan_tree = resolve_scan_tree(&self.scan_cache, &self.view_root_path);

        let mut new_lines = Vec::new();
        let mut child_path = path.to_vec();
        for (_name, child_idx) in children {
            flatten_snapshot_tree(
                snap_arc,
                child_idx,
                path.len(),
                expanded,
                sort_mode,
                &mut new_lines,
                &self.scan_cache,
                &self.view_root_path,
                root_scan_tree,
                &mut child_path,
            );
        }

        self.tree_lines.splice(pos + 1..pos + 1, new_lines);
        self.refresh_filtered_lines();
        true
    }

    /// Get the currently selected tree line (from filtered view)
    pub fn selected_line(&self) -> Option<&TreeLine> {
        self.filtered_tree_lines
            .get(self.cursor)
            .and_then(|&idx| self.tree_lines.get(idx))
    }

    /// Set error message and log to file.
    pub fn set_error(&mut self, msg: String, duration_secs: u64) {
        self.last_error = Some(msg.clone());
        self.error_clear_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(duration_secs));
        let now = chrono::Local::now();
        let line = format!("[{}] {}\n", now.format("%Y-%m-%d %H:%M:%S"), msg);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    }

    /// Return the relative path of the tree line at `idx` (into filtered view), rooted at `view_root_path`.
    pub fn tree_line_relative_path(&self, idx: usize) -> Option<Vec<String>> {
        self.filtered_tree_lines
            .get(idx)
            .and_then(|&actual| self.tree_lines.get(actual))
            .map(|line| line.path.clone())
    }

    /// Set time range preset (0=1h, 1=6h, 2=12h, 3=1d, 4=3d, 5=7d)
    pub fn set_time_preset(&mut self, preset: usize) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.time_to = now;
        self.time_preset = preset;
        self.time_from = match preset {
            0 => now.saturating_sub(3_600_000),   // 1h
            1 => now.saturating_sub(21_600_000),  // 6h
            2 => now.saturating_sub(43_200_000),  // 12h
            3 => now.saturating_sub(86_400_000),  // 1d (same as 24h)
            4 => now.saturating_sub(259_200_000), // 3d
            5 => now.saturating_sub(604_800_000), // 7d
            _ => now.saturating_sub(3_600_000),
        };
    }

    pub fn default_time_range(&mut self) {
        self.set_time_preset(0);
    }

    pub fn time_preset_label(preset: usize) -> &'static str {
        match preset {
            0 => "1h",
            1 => "6h",
            2 => "12h",
            3 => "1d",
            4 => "3d",
            5 => "7d",
            _ => "1h",
        }
    }
    /// Rebuild filtered_tree_lines from tree_lines based on delta filter
    pub fn refresh_filtered_lines(&mut self) {
        if !self.delta_filter_active {
            self.filtered_tree_lines = (0..self.tree_lines.len()).collect();
        } else {
            let threshold = self.delta_filter_value * delta_unit_multiplier(self.delta_filter_unit);
            let strict = self.delta_filter_value == 0;
            self.filtered_tree_lines = self
                .tree_lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    let delta = self.delta_cache.get(&line.path).copied().unwrap_or(0);
                    if strict {
                        delta > 0
                    } else {
                        (delta as u64) >= threshold
                    }
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.cursor >= self.filtered_tree_lines.len() && !self.filtered_tree_lines.is_empty() {
            self.cursor = self.filtered_tree_lines.len() - 1;
        }
    }
    /// Get the current delta filter threshold in bytes, or 0 if inactive
    pub fn delta_filter_threshold(&self) -> u64 {
        if !self.delta_filter_active {
            0
        } else {
            self.delta_filter_value * delta_unit_multiplier(self.delta_filter_unit)
        }
    }

    /// Map filtered view cursor to actual tree_lines index
    pub fn cursor_to_tree_idx(&self) -> usize {
        self.filtered_tree_lines
            .get(self.cursor)
            .copied()
            .unwrap_or(0)
    }

    /// Increment delta filter value, with auto unit level-up at 1024
    pub fn delta_filter_inc(&mut self) {
        self.delta_filter_value += 1;
        if self.delta_filter_value > 1024 && self.delta_filter_unit < 2 {
            self.delta_filter_value = 1;
            self.delta_filter_unit += 1;
        }
    }

    /// Decrement delta filter value (min 0)
    pub fn delta_filter_dec(&mut self) {
        if self.delta_filter_value > 0 {
            self.delta_filter_value -= 1;
        }
    }

    /// Cycle delta filter unit (KB→MB→GB→KB)
    pub fn delta_filter_cycle_unit(&mut self) {
        self.delta_filter_unit = (self.delta_filter_unit + 1) % 3;
    }

    /// Clear the delta filter and return to tree focus
    pub fn clear_filter_pane(&mut self) {
        self.delta_filter_active = false;
        self.delta_filter_value = 100;
        self.delta_filter_unit = 1;
        self.focus = Focus::Tree;
        self.refresh_filtered_lines();
    }

    // ── Command bar ────────────────────────────────────────────────────────────

    pub const COMMANDS: &'static [&'static str] = &[
        "FilterClear",
        "FilterFocus",
        "Help",
        "FilterDelta",
        "FilterTime",
        "Scan",
        "Consolidate",
    ];

    pub fn update_command_matches(&mut self) {
        if self.command_input.is_empty() {
            self.command_matches = Self::COMMANDS.to_vec();
        } else {
            let lower = self.command_input.to_lowercase();
            self.command_matches = Self::COMMANDS
                .iter()
                .filter(|c| fuzzy_match(&lower, &c.to_lowercase()))
                .copied()
                .collect();
        }
        if self.command_selected >= self.command_matches.len() {
            self.command_selected = 0;
        }
    }

    pub fn execute_command(&mut self, cmd: &str) -> Result<String, String> {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return Err("empty command".into());
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let name = parts[0];

        match name {
            "FilterClear" => {
                if self.server_mode {
                    self.clear_filter_pane();
                    Ok("filter cleared".into())
                } else {
                    Err("not in server mode".into())
                }
            }
            "FilterFocus" => {
                if self.server_mode {
                    self.focus = Focus::FilterPane;
                    self.filter_focus = FilterFocus::TimePreset;
                    Ok("filter pane focused".into())
                } else {
                    Err("not in server mode".into())
                }
            }
            "Help" => {
                self.mode = AppMode::Help;
                Ok("help opened".into())
            }
            "FilterDelta" => {
                if !self.server_mode {
                    return Err("not in server mode".into());
                }
                let arg = *parts.get(1).ok_or("usage: FilterDelta <N>[k|m|g]")?;
                let (num_str, unit) = if arg.ends_with('k') || arg.ends_with('K') {
                    (&arg[..arg.len() - 1], 0usize)
                } else if arg.ends_with('m') || arg.ends_with('M') {
                    (&arg[..arg.len() - 1], 1usize)
                } else if arg.ends_with('g') || arg.ends_with('G') {
                    (&arg[..arg.len() - 1], 2usize)
                } else {
                    (arg, 1usize)
                };
                let value: u64 = num_str
                    .parse()
                    .map_err(|_| format!("invalid number: {num_str}"))?;
                self.delta_filter_active = true;
                self.delta_filter_value = value;
                self.delta_filter_unit = unit;
                self.refresh_filtered_lines();
                Ok(format!(
                    "delta filter set to {}{}",
                    value,
                    ["KB", "MB", "GB"][unit]
                ))
            }
            "FilterTime" => {
                if !self.server_mode {
                    return Err("not in server mode".into());
                }
                let arg = *parts.get(1).ok_or("usage: FilterTime <N>h")?;
                let num_str = if arg.ends_with('h')
                    || arg.ends_with('H')
                    || arg.ends_with('d')
                    || arg.ends_with('D')
                {
                    &arg[..arg.len() - 1]
                } else {
                    arg
                };
                let hours: u64 = if arg.ends_with('d') || arg.ends_with('D') {
                    num_str
                        .parse::<u64>()
                        .map_err(|_| format!("invalid number: {num_str}"))?
                        * 24
                } else {
                    num_str
                        .parse()
                        .map_err(|_| format!("invalid number: {num_str}"))?
                };
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                self.time_from = now.saturating_sub(hours * 3_600_000);
                self.time_to = now;
                self.request_delta_refresh();
                Ok(format!("time range set to {hours}h"))
            }
            "Scan" => {
                if self.scanning {
                    return Err("already scanning".into());
                }
                Ok("scan started".into())
            }
            "Consolidate" => {
                if !self.server_mode {
                    return Err("not in server mode".into());
                }
                Ok("consolidation requested".into())
            }
            _ => Err(format!("unknown command: {name}")),
        }
    }

    /// Request delta data from daemon — single query to root path, build full map locally
    pub fn request_delta_refresh(&mut self) {
        if self.delta_pending {
            return;
        }
        let view_root = self.view_root_path.clone();
        if !view_root.exists() {
            return;
        }
        let from = self.time_from;
        let to = self.time_to;
        let tx = self.tx.clone();
        let uds_path = crate::config::TuiConfig::default().daemon.uds_path;
        let log_path = self.log_path.clone();
        self.delta_pending = true;
        log_msg(
            &log_path,
            &format!(
                "request_delta_refresh: from={from} to={to} root={}",
                view_root.display()
            ),
        );
        tokio::spawn(async move {
            let t0 = Instant::now();
            let deltas = Self::fetch_deltas(&uds_path, &view_root, from, to, &log_path).await;
            log_msg(
                &log_path,
                &format!(
                    "fetch_deltas done: {} paths in {:?}",
                    deltas.len(),
                    t0.elapsed()
                ),
            );
            let _ = tx.send(AppMessage::DeltaData(deltas)).await;
        });
    }

    async fn fetch_deltas(
        uds: &str,
        view_root: &std::path::Path,
        from: u64,
        to: u64,
        log_path: &Path,
    ) -> HashMap<Vec<String>, i64> {
        let t0 = Instant::now();
        let mut client = match IpcClient::connect(uds).await {
            Ok(c) => c,
            Err(e) => {
                log_msg(log_path, &format!("fetch_deltas: connect failed: {e}"));
                return HashMap::new();
            }
        };
        let t1 = Instant::now();
        log_msg(
            log_path,
            &format!("fetch_deltas: connected in {:?}", t1 - t0),
        );
        let (_total, entries) = match client.get_delta(view_root, from, to).await {
            Ok(r) => r,
            Err(e) => {
                log_msg(log_path, &format!("fetch_deltas: query failed: {e}"));
                return HashMap::new();
            }
        };
        let t2 = Instant::now();
        log_msg(
            log_path,
            &format!(
                "fetch_deltas: query returned {} entries in {:?}",
                entries.len(),
                t2 - t1
            ),
        );
        let mut file_deltas: HashMap<PathBuf, i64> = HashMap::new();
        for entry in &entries {
            *file_deltas.entry(entry.path.clone()).or_insert(0) += entry.delta_size;
        }
        let mut deltas: HashMap<Vec<String>, i64> = HashMap::new();
        for (abs_path, delta) in &file_deltas {
            let relative = abs_path.strip_prefix(view_root).ok();
            let Some(relative) = relative else { continue };
            let mut components: Vec<String> = Vec::new();
            components.push(
                view_root
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            );
            for comp in relative.components() {
                components.push(comp.as_os_str().to_string_lossy().to_string());
            }
            for i in 1..=components.len() {
                let ancestor = components[..i].to_vec();
                *deltas.entry(ancestor).or_insert(0) += delta;
            }
        }
        log_msg(
            log_path,
            &format!(
                "fetch_deltas: map build done, {} paths in {:?}",
                deltas.len(),
                t2.elapsed()
            ),
        );
        deltas
    }
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
        path: path.clone(),
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

/// Append a timestamped message to the log file
pub fn log_msg(log_path: &Path, msg: &str) {
    let now = chrono::Local::now();
    let line = format!("[{}] {}\n", now.format("%Y-%m-%d %H:%M:%S%.3f"), msg);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
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
    path_to_walk_idx: &mut HashMap<Vec<String>, usize>,
) {
    let node = snap.node(idx);
    let is_visible = path_is_visible(path, expanded);

    // Cache walk_idx for every node so n/N jumping is O(1) instead of O(n).
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
        // Skip subtrees with too many siblings — prevents n/N from trying to
        // expand a massive directory and hanging the UI.
        let skip_subtree = node.children.len() > MAX_DIR_CHILDREN;

        if !skip_subtree {
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
                    path_to_walk_idx,
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

fn fuzzy_match(query: &str, target: &str) -> bool {
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
    use crate::config::TuiConfig;
    use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};
    use std::collections::{HashMap, HashSet};
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
