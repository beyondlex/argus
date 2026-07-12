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
use crate::time_utils::*;

// ── Data types ──────────────────────────────────────────────────────────────

/// Messages from background tasks to the UI
#[derive(Debug)]
pub enum AppMessage {
    ScanProgress { file_count: u64, total_bytes: u64 },
    ScanComplete(Snapshot),
    DaemonConnected(IpcClient),
    DaemonDisconnected,
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
    TimeHelp,
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

/// Tree search mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchMode {
    Inactive,
    Input,
    Active,
}

/// A match found by the tree search — `path` is the full relative path from the view root.
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
    Delta,
}

impl SortMode {
    pub fn toggle(self) -> Self {
        match self {
            SortMode::Name => SortMode::Size,
            SortMode::Size => SortMode::Delta,
            SortMode::Delta => SortMode::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Size => "Size",
            SortMode::Name => "Name",
            SortMode::Delta => "Delta",
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
    pub time_custom: bool,
    pub time_custom_label: String,

    // Filter pane state
    pub filter_focus: FilterFocus,
    pub delta_filter_active: bool,
    pub delta_filter_value: u64,
    pub delta_filter_unit: usize, // 0=KB, 1=MB, 2=GB
    pub delta_pending: bool,
    pub filtered_tree_lines: Vec<usize>,

    // Tree search (fuzzy search)
    pub search_word: String,
    pub search_mode: SearchMode,
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
    pub command_history: Vec<String>,
    pub command_history_idx: Option<usize>,

    // Time help popup
    pub time_help_scroll: usize,
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
            time_custom: false,
            time_custom_label: String::new(),
            filter_focus: FilterFocus::TimePreset,
            delta_filter_active: false,
            delta_filter_value: 100,
            delta_filter_unit: 1,
            delta_pending: false,
            filtered_tree_lines: Vec::new(),
            search_word: String::new(),
            search_mode: SearchMode::Inactive,
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
            command_history: Vec::new(),
            command_history_idx: None,
            rx,
            last_error: None,
            error_clear_at: None,
            log_path,
            time_help_scroll: 0,
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
                    let root_scan_tree =
                        crate::tree_ops::resolve_scan_tree(&self.scan_cache, &self.view_root_path);
                    crate::tree_ops::enrich_snapshot_sizes(
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
        let delta_cache = if self.delta_cache.is_empty() {
            None
        } else {
            Some(&self.delta_cache)
        };
        let lines = match &self.tree_root {
            Some(TreeNode::Snapshot(snap_arc, idx)) => {
                let mut lines = Vec::new();
                let root_scan_tree =
                    crate::tree_ops::resolve_scan_tree(&self.scan_cache, &self.view_root_path);
                crate::tree_ops::flatten_snapshot_tree(
                    snap_arc,
                    *idx,
                    0,
                    expanded,
                    sort_mode,
                    &mut lines,
                    &self.scan_cache,
                    &self.view_root_path,
                    root_scan_tree,
                    delta_cache,
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
        self.recompute_matches();
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
            AppMessage::DaemonDisconnected => {
                self.server_connected = false;
                self.server_mode = false;
                self.daemon_client = None;
                self.delta_cache.clear();
                self.delta_filter_active = false;
                self.focus = Focus::Tree;
                self.refresh_filtered_lines();
                self.set_error("daemon disconnected".into(), 4);
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

    /// Recompute match_indices for current search_word.
    /// Walks the full tree in display order (depth-first, sorted by sort_mode)
    /// so n/N jumps follow the natural top-to-bottom order.
    /// Also populates path_to_walk_idx cache to avoid a second full tree walk on n/N.
    pub fn recompute_matches(&mut self) {
        self.match_indices.clear();
        self.current_match = 0;
        self.path_to_walk_idx.clear();
        if self.search_word.is_empty() {
            return;
        }

        let Some(TreeNode::Snapshot(snap_arc, root_idx)) = &self.tree_root else {
            return;
        };

        let query = &self.search_word;
        let expanded = &self.expanded;
        let sort_mode = self.sort_mode;

        let delta_cache = if self.delta_cache.is_empty() {
            None
        } else {
            Some(&self.delta_cache)
        };
        let mut matches = Vec::new();
        let mut walk_index = 0usize;
        let mut visible_count = 0usize;
        let mut current_path = vec![snap_arc.node(*root_idx).name.clone()];
        crate::search::collect_matches_in_order(
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
            delta_cache,
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
        let Some(dir_idx) = snap_arc.find_node(*root_idx, path) else {
            return false;
        };
        let node = snap_arc.node(dir_idx);
        if !node.is_dir
            || node.children.is_empty()
            || node.children.len() > crate::tree_ops::MAX_DIR_CHILDREN
        {
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
        let delta_cache = if self.delta_cache.is_empty() {
            None
        } else {
            Some(&self.delta_cache)
        };
        crate::tree_ops::sort_children_snapshot(
            &mut children,
            snap_arc,
            self.sort_mode,
            path,
            delta_cache,
        );

        let expanded = &self.expanded;
        let sort_mode = self.sort_mode;
        let root_scan_tree =
            crate::tree_ops::resolve_scan_tree(&self.scan_cache, &self.view_root_path);

        let mut new_lines = Vec::new();
        let mut child_path = path.to_vec();
        for (_name, child_idx) in children {
            crate::tree_ops::flatten_snapshot_tree(
                snap_arc,
                child_idx,
                path.len(),
                expanded,
                sort_mode,
                &mut new_lines,
                &self.scan_cache,
                &self.view_root_path,
                root_scan_tree,
                delta_cache,
                &mut child_path,
            );
        }

        self.tree_lines.splice(pos + 1..pos + 1, new_lines);
        self.refresh_filtered_lines();
        self.recompute_matches();
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
        self.time_custom = false;
        self.time_custom_label.clear();
        let now = now_in_millis();
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
        // Keep search matches in sync: only show matches that are in the filtered view
        if !self.match_indices.is_empty() {
            let visible: HashSet<usize> = self.filtered_tree_lines.iter().copied().collect();
            self.match_indices
                .retain(|m| m.tree_idx.map_or(true, |idx| visible.contains(&idx)));
            self.current_match = self
                .current_match
                .min(self.match_indices.len().saturating_sub(1));
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

    /// Clear the delta filter and reset time, return to tree focus
    pub fn clear_filter_pane(&mut self) {
        self.delta_filter_active = false;
        self.delta_filter_value = 100;
        self.delta_filter_unit = 1;
        self.set_time_preset(0);
        self.focus = Focus::Tree;
        self.refresh_filtered_lines();
        self.recompute_matches();
    }

    // ── Command bar ────────────────────────────────────────────────────────────

    pub const COMMANDS: &'static [&'static str] = &[
        "Consolidate",
        "Delta",
        "FilterClear",
        "FilterFocus",
        "Help",
        "Scan",
        "Sort",
        "Time",
    ];

    pub fn update_command_matches(&mut self) {
        if self.command_input.is_empty() {
            self.command_matches = Self::COMMANDS.to_vec();
        } else {
            let lower = self.command_input.to_lowercase();
            self.command_matches = Self::COMMANDS
                .iter()
                .filter(|c| crate::search::fuzzy_match(&lower, &c.to_lowercase()))
                .copied()
                .collect();
        }
        if self.command_selected >= self.command_matches.len() {
            self.command_selected = 0;
        }
    }

    pub fn clear_command_state(&mut self) {
        self.command_input.clear();
        self.command_matches.clear();
        self.command_selected = 0;
    }

    pub fn push_command_history(&mut self, cmd: &str) {
        let cmd = cmd.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        if self.command_history.last().map(|s| s.as_str()) == Some(cmd.as_str()) {
            return;
        }
        self.command_history.push(cmd);
        if self.command_history.len() > 50 {
            self.command_history.remove(0);
        }
        self.command_history_idx = None;
    }

    pub fn execute_command(&mut self, cmd: &str) -> Result<String, String> {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return Err("empty command".into());
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let name = parts[0];
        let name_lower = name.to_lowercase();

        match name_lower.as_str() {
            "filterclear" => self.cmd_filterclear(),
            "filterfocus" => self.cmd_filterfocus(),
            "help" => self.cmd_help(),
            "delta" => self.cmd_delta(&parts),
            "time" => self.cmd_time(&parts),
            "sort" | "s" => self.cmd_sort(&parts),
            "sd" => self.cmd_sort_quick(SortMode::Delta, "Delta"),
            "ss" => self.cmd_sort_quick(SortMode::Size, "Size"),
            "sn" => self.cmd_sort_quick(SortMode::Name, "Name"),
            "scan" => self.cmd_scan(),
            "consolidate" => self.cmd_consolidate(),
            _ => Err(format!("unknown command: {name}")),
        }
    }

    fn cmd_filterclear(&mut self) -> Result<String, String> {
        if self.server_mode {
            self.clear_filter_pane();
            Ok("filter cleared".into())
        } else {
            Err("not in server mode".into())
        }
    }

    fn cmd_filterfocus(&mut self) -> Result<String, String> {
        if self.server_mode {
            self.focus = Focus::FilterPane;
            self.filter_focus = FilterFocus::TimePreset;
            Ok("filter pane focused".into())
        } else {
            Err("not in server mode".into())
        }
    }

    fn cmd_help(&mut self) -> Result<String, String> {
        self.mode = AppMode::Help;
        Ok("help opened".into())
    }

    fn cmd_delta(&mut self, parts: &[&str]) -> Result<String, String> {
        if !self.server_mode {
            return Err("not in server mode".into());
        }
        let (num_str, unit) = match parts.get(1).copied() {
            Some(arg) if arg.ends_with('k') || arg.ends_with('K') => {
                (&arg[..arg.len() - 1], 0usize)
            }
            Some(arg) if arg.ends_with('m') || arg.ends_with('M') => {
                (&arg[..arg.len() - 1], 1usize)
            }
            Some(arg) if arg.ends_with('g') || arg.ends_with('G') => {
                (&arg[..arg.len() - 1], 2usize)
            }
            Some(arg) => (arg, 1usize),
            None => ("0", 0usize),
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

    fn cmd_time(&mut self, parts: &[&str]) -> Result<String, String> {
        if !self.server_mode {
            return Err("not in server mode".into());
        }
        let arg = match parts.get(1) {
            Some(a) => *a,
            None => {
                self.mode = AppMode::TimeHelp;
                return Ok("time help opened".into());
            }
        };
        let rest: String = parts[1..].join(" ");
        let to_lower = rest.to_lowercase();
        let to_marker = " to ";
        if let Some(pos) = to_lower.find(to_marker) {
            let left = rest[..pos].trim();
            let right = rest[pos + to_marker.len()..].trim();
            if left.is_empty() || right.is_empty() {
                return Err("invalid time range: empty side".into());
            }
            let left_parsed = parse_single_time_arg(left)?;
            let (to_ms, right_label) = if is_time_only(right) {
                let date = left_parsed
                    .date
                    .ok_or("cannot inherit date for time-only right side")?;
                let parts: Vec<&str> = right.split(':').collect();
                let h: u32 = parts[0].parse().unwrap_or(0);
                let min: u32 = parts[1].parse().unwrap_or(0);
                (
                    datetime_to_millis(date.0, date.1, h, min),
                    format!("{:02}:{:02}", h, min),
                )
            } else {
                let parsed = parse_single_time_arg(right)?;
                (parsed.ms, parsed.label)
            };
            self.time_from = left_parsed.ms;
            self.time_to = to_ms;
            self.time_custom = true;
            self.time_custom_label = format_time_label(&left_parsed.label, &right_label);
            self.request_delta_refresh();
            Ok(format!("time range: {}", self.time_custom_label))
        } else {
            let parsed = parse_single_time_arg(arg)?;
            let now = now_in_millis();
            self.time_from = parsed.ms;
            self.time_to = now;
            self.time_custom = true;
            if parsed.date.is_some() {
                self.time_custom_label = format!("{} ~ now", parsed.label);
            } else {
                self.time_custom_label = parsed.label;
            }
            self.request_delta_refresh();
            if parsed.date.is_some() {
                Ok(format!("time range: {}", self.time_custom_label))
            } else {
                Ok(format!("time range: in {}", self.time_custom_label))
            }
        }
    }

    fn cmd_sort(&mut self, parts: &[&str]) -> Result<String, String> {
        let sub = parts.get(1).copied().unwrap_or("");
        match sub.to_lowercase().as_str() {
            "d" | "delta" => self.sort_mode = SortMode::Delta,
            "s" | "size" => self.sort_mode = SortMode::Size,
            "n" | "name" => self.sort_mode = SortMode::Name,
            "" => self.sort_mode = self.sort_mode.toggle(),
            _ => return Err(format!("unknown sort mode: {sub}")),
        }
        self.update_tree_lines();
        Ok(format!("Sort: {}", self.sort_mode.label()))
    }

    fn cmd_sort_quick(&mut self, mode: SortMode, label: &str) -> Result<String, String> {
        self.sort_mode = mode;
        self.update_tree_lines();
        Ok(format!("Sort: {label}"))
    }

    fn cmd_scan(&mut self) -> Result<String, String> {
        if self.scanning {
            return Err("already scanning".into());
        }
        Ok("scan started".into())
    }

    fn cmd_consolidate(&mut self) -> Result<String, String> {
        if !self.server_mode {
            return Err("not in server mode".into());
        }
        Ok("consolidation requested".into())
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

    // ── execute_command tests ───────────────────────────────────────────────────

    #[test]
    fn test_execute_empty_command() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(app.execute_command(""), Err("empty command".into()));
    }

    #[test]
    fn test_execute_unknown_command() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        let result = app.execute_command("foobar");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown command"));
    }

    #[test]
    fn test_execute_help() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert!(app.execute_command("help").is_ok());
        assert_eq!(app.mode, AppMode::Help);
    }

    #[test]
    fn test_execute_time_not_in_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(
            app.execute_command("time 2h"),
            Err("not in server mode".into())
        );
    }

    #[test]
    fn test_execute_time_no_arg_opens_help() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        assert!(app.execute_command("time").is_ok());
        assert_eq!(app.mode, AppMode::TimeHelp);
    }

    #[test]
    fn test_execute_time_duration() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        app.delta_pending = true;
        let result = app.execute_command("time 2h");
        assert!(result.is_ok());
        assert!(result.unwrap().contains("2h"));
        assert!(app.time_custom);
    }

    #[test]
    fn test_execute_time_time_only() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        app.delta_pending = true;
        let result = app.execute_command("time 14:30");
        assert!(result.is_ok());
        assert!(app.time_custom);
        assert!(app.time_custom_label.contains("14:30"));
    }

    #[test]
    fn test_execute_time_absolute() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        app.delta_pending = true;
        let result = app.execute_command("time 07-04");
        assert!(result.is_ok());
        assert!(app.time_custom);
    }

    #[test]
    fn test_execute_time_range_time_to_time() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        app.delta_pending = true;
        let result = app.execute_command("time 09:00 to 17:00");
        assert!(result.is_ok());
        assert!(app.time_custom);
        assert!(app.time_custom_label.contains("09:00"));
        assert!(app.time_custom_label.contains("17:00"));
    }

    #[test]
    fn test_execute_time_range_absolute_to_absolute() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        app.delta_pending = true;
        let result = app.execute_command("time 07-04 to 07-05");
        assert!(result.is_ok());
        assert!(app.time_custom);
        assert!(app.time_custom_label.contains("07-04"));
        assert!(app.time_custom_label.contains("07-05"));
    }

    #[test]
    fn test_execute_time_range_duration_to_date_errors() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        // 2h as left (duration, left_date=None) and 09:00 as right (time-only needs left_date)
        let result = app.execute_command("time 2h to 09:00");
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_time_invalid_duration() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        let result = app.execute_command("time 2x");
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_sort_name() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert!(app.execute_command("sort n").is_ok());
        assert_eq!(app.sort_mode, SortMode::Name);
    }

    #[test]
    fn test_execute_sort_toggle() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.sort_mode = SortMode::Name;
        assert!(app.execute_command("sort").is_ok());
        assert_eq!(app.sort_mode, SortMode::Size);
    }

    #[test]
    fn test_execute_sort_unknown() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        let result = app.execute_command("sort x");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown sort mode"));
    }

    #[test]
    fn test_execute_delta_with_unit() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        assert!(app.execute_command("delta 500k").is_ok());
        assert_eq!(app.delta_filter_value, 500);
        assert_eq!(app.delta_filter_unit, 0);
        assert!(app.delta_filter_active);
    }

    #[test]
    fn test_execute_delta_not_in_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(
            app.execute_command("delta 100m"),
            Err("not in server mode".into())
        );
    }

    #[test]
    fn test_execute_filterclear_not_in_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(
            app.execute_command("filterclear"),
            Err("not in server mode".into())
        );
    }
}
