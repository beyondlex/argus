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

pub fn now_in_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn parse_duration(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num_str, mult) = if s.ends_with('w') || s.ends_with('W') {
        (&s[..s.len() - 1], 604_800_000u64)
    } else if s.ends_with('d') || s.ends_with('D') {
        (&s[..s.len() - 1], 86_400_000u64)
    } else if s.ends_with('h') || s.ends_with('H') {
        (&s[..s.len() - 1], 3_600_000u64)
    } else {
        (s, 3_600_000u64)
    };
    let n: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number: {num_str}"))?;
    Ok(n * mult)
}

fn parse_date_time(s: &str) -> Result<(u32, u32, u32, u32), String> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.len() {
        1 => {
            let date_parts: Vec<&str> = parts[0].split('-').collect();
            if date_parts.len() == 2 {
                let month: u32 = date_parts[0]
                    .parse()
                    .map_err(|_| format!("invalid month: {}", date_parts[0]))?;
                let day: u32 = date_parts[1]
                    .parse()
                    .map_err(|_| format!("invalid day: {}", date_parts[1]))?;
                Ok((month, day, 0, 0))
            } else {
                Err(format!("invalid date: {}", s))
            }
        }
        2 => {
            let date_parts: Vec<&str> = parts[0].split('-').collect();
            if date_parts.len() != 2 {
                return Err(format!("invalid date: {}", parts[0]));
            }
            let month: u32 = date_parts[0]
                .parse()
                .map_err(|_| format!("invalid month: {}", date_parts[0]))?;
            let day: u32 = date_parts[1]
                .parse()
                .map_err(|_| format!("invalid day: {}", date_parts[1]))?;
            let time_parts: Vec<&str> = parts[1].split(':').collect();
            if time_parts.len() != 2 {
                return Err(format!("invalid time: {}", parts[1]));
            }
            let hour: u32 = time_parts[0]
                .parse()
                .map_err(|_| format!("invalid hour: {}", time_parts[0]))?;
            let minute: u32 = time_parts[1]
                .parse()
                .map_err(|_| format!("invalid minute: {}", time_parts[1]))?;
            Ok((month, day, hour, minute))
        }
        _ => Err(format!("invalid date-time: {s}")),
    }
}

fn is_time_only(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 2
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].chars().all(|c| c.is_ascii_digit())
}

fn datetime_to_millis(month: u32, day: u32, hour: u32, minute: u32) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Use current year, if result is in the future try previous year
    for year_offset in [0i64, -1] {
        let year = 1970 + (now as f64 / 31557600.0) as i64 + year_offset;
        if let Some(ms) = date_to_millis(year as i32, month, day, hour, minute) {
            if ms <= now as u64 * 1000 || year_offset < 0 {
                return ms;
            }
        }
    }
    0
}

fn date_to_millis(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Option<u64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }

    let days = days_since_epoch(year, month, day)?;
    Some((days as u64 * 86400 + hour as u64 * 3600 + minute as u64 * 60) * 1000)
}

fn days_since_epoch(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) {
        return None;
    }
    let y = if month <= 2 {
        year as i64 - 1
    } else {
        year as i64
    };
    let m = if month <= 2 {
        month as i64 + 12
    } else {
        month as i64
    };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days)
}

fn format_time_label(left: &str, right: &str) -> String {
    format!("{} ~ {}", left, right)
}

fn format_duration_label(ms: u64) -> String {
    if ms % 604_800_000 == 0 {
        format!("{}w", ms / 604_800_000)
    } else if ms % 86_400_000 == 0 {
        format!("{}d", ms / 86_400_000)
    } else {
        format!("{}h", ms / 3_600_000)
    }
}

fn format_absolute_label(month: u32, day: u32, hour: u32, minute: u32) -> String {
    if hour == 0 && minute == 0 {
        format!("{month:02}-{day:02}")
    } else {
        format!("{month:02}-{day:02} {hour:02}:{minute:02}")
    }
}

fn today_md() -> (u32, u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = now / 86400 + 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    (month, day)
}

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
        let delta_cache = if self.delta_cache.is_empty() {
            None
        } else {
            Some(&self.delta_cache)
        };
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
        let delta_cache = if self.delta_cache.is_empty() {
            None
        } else {
            Some(&self.delta_cache)
        };
        sort_children_snapshot(&mut children, snap_arc, self.sort_mode, path, delta_cache);

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
                .retain(|m| m.tree_idx.is_some_and(|idx| visible.contains(&idx)));
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
                .filter(|c| fuzzy_match(&lower, &c.to_lowercase()))
                .copied()
                .collect();
        }
        if self.command_selected >= self.command_matches.len() {
            self.command_selected = 0;
        }
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
            "filterclear" => {
                if self.server_mode {
                    self.clear_filter_pane();
                    Ok("filter cleared".into())
                } else {
                    Err("not in server mode".into())
                }
            }
            "filterfocus" => {
                if self.server_mode {
                    self.focus = Focus::FilterPane;
                    self.filter_focus = FilterFocus::TimePreset;
                    Ok("filter pane focused".into())
                } else {
                    Err("not in server mode".into())
                }
            }
            "help" => {
                self.mode = AppMode::Help;
                Ok("help opened".into())
            }
            "delta" => {
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
            "time" => {
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

                // Split on " to " (case-insensitive)
                let to_lower = rest.to_lowercase();
                let to_marker = " to ";
                if let Some(pos) = to_lower.find(to_marker) {
                    let left = &rest[..pos];
                    let right = &rest[pos + to_marker.len()..];
                    let left = left.trim();
                    let right = right.trim();

                    if left.is_empty() || right.is_empty() {
                        return Err("invalid time range: empty side".into());
                    }

                    // Parse left side
                    let (from_ms, left_label, left_date) = if is_time_only(left) {
                        let time_parts: Vec<&str> = left.split(':').collect();
                        let h: u32 = time_parts[0]
                            .parse()
                            .map_err(|_| format!("invalid hour: {left}"))?;
                        let min: u32 = time_parts[1]
                            .parse()
                            .map_err(|_| format!("invalid minute: {left}"))?;
                        let (m, d) = today_md();
                        let label = format!("{:02}:{:02}", h, min);
                        (datetime_to_millis(m, d, h, min), label, Some((m, d)))
                    } else if let Ok(ms) = parse_duration(left) {
                        (
                            now_in_millis().saturating_sub(ms),
                            format_duration_label(ms),
                            None,
                        )
                    } else {
                        let (m, d, h, min) =
                            parse_date_time(left).map_err(|e| format!("invalid left side: {e}"))?;
                        (
                            datetime_to_millis(m, d, h, min),
                            format_absolute_label(m, d, h, min),
                            Some((m, d)),
                        )
                    };

                    // Parse right side
                    let (to_ms, right_label) = if is_time_only(right) {
                        let date =
                            left_date.ok_or("cannot inherit date for time-only right side")?;
                        let time_parts: Vec<&str> = right.split(':').collect();
                        let h: u32 = time_parts[0].parse().unwrap_or(0);
                        let min: u32 = time_parts[1].parse().unwrap_or(0);
                        (
                            datetime_to_millis(date.0, date.1, h, min),
                            format!("{:02}:{:02}", h, min),
                        )
                    } else if let Ok(ms) = parse_duration(right) {
                        (
                            now_in_millis().saturating_sub(ms),
                            format_duration_label(ms),
                        )
                    } else {
                        let (m, d, h, min) = parse_date_time(right)
                            .map_err(|e| format!("invalid right side: {e}"))?;
                        (
                            datetime_to_millis(m, d, h, min),
                            format_absolute_label(m, d, h, min),
                        )
                    };

                    self.time_from = from_ms;
                    self.time_to = to_ms;
                    self.time_custom = true;
                    self.time_custom_label = format_time_label(&left_label, &right_label);
                    self.request_delta_refresh();
                    Ok(format!("time range: {}", self.time_custom_label))
                } else {
                    // No "to" — single arg
                    let now = now_in_millis();
                    if is_time_only(arg) {
                        let time_parts: Vec<&str> = arg.split(':').collect();
                        let h: u32 = time_parts[0]
                            .parse()
                            .map_err(|_| format!("invalid hour: {arg}"))?;
                        let min: u32 = time_parts[1]
                            .parse()
                            .map_err(|_| format!("invalid minute: {arg}"))?;
                        let (m, d) = today_md();
                        let from_ms = datetime_to_millis(m, d, h, min);
                        self.time_from = from_ms;
                        self.time_to = now;
                        self.time_custom = true;
                        self.time_custom_label = format!("{:02}:{:02} ~ now", h, min);
                        self.request_delta_refresh();
                        Ok(format!("time range: {}", self.time_custom_label))
                    } else if let Ok((m, d, h, min)) = parse_date_time(arg) {
                        let from_ms = datetime_to_millis(m, d, h, min);
                        self.time_from = from_ms;
                        self.time_to = now;
                        self.time_custom = true;
                        self.time_custom_label =
                            format!("{} ~ now", format_absolute_label(m, d, h, min));
                        self.request_delta_refresh();
                        Ok(format!("time range: {}", self.time_custom_label))
                    } else {
                        let ms =
                            parse_duration(arg).map_err(|e| format!("invalid duration: {e}"))?;
                        self.time_from = now.saturating_sub(ms);
                        self.time_to = now;
                        self.time_custom = true;
                        self.time_custom_label = format_duration_label(ms);
                        self.request_delta_refresh();
                        Ok(format!("time range: in {}", self.time_custom_label))
                    }
                }
            }
            "sort" | "s" => {
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
            "sd" => {
                self.sort_mode = SortMode::Delta;
                self.update_tree_lines();
                Ok("Sort: Delta".into())
            }
            "ss" => {
                self.sort_mode = SortMode::Size;
                self.update_tree_lines();
                Ok("Sort: Size".into())
            }
            "sn" => {
                self.sort_mode = SortMode::Name;
                self.update_tree_lines();
                Ok("Sort: Name".into())
            }
            "scan" => {
                if self.scanning {
                    return Err("already scanning".into());
                }
                Ok("scan started".into())
            }
            "consolidate" => {
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
            match Self::fetch_deltas(&uds_path, &view_root, from, to, &log_path).await {
                Some(deltas) => {
                    log_msg(
                        &log_path,
                        &format!(
                            "fetch_deltas done: {} paths in {:?}",
                            deltas.len(),
                            t0.elapsed()
                        ),
                    );
                    let _ = tx.send(AppMessage::DeltaData(deltas)).await;
                }
                None => {
                    let _ = tx.send(AppMessage::DaemonDisconnected).await;
                }
            }
        });
    }

    async fn fetch_deltas(
        uds: &str,
        view_root: &std::path::Path,
        from: u64,
        to: u64,
        log_path: &Path,
    ) -> Option<HashMap<Vec<String>, i64>> {
        let t0 = Instant::now();
        let mut client = match IpcClient::connect(uds).await {
            Ok(c) => c,
            Err(e) => {
                log_msg(log_path, &format!("fetch_deltas: connect failed: {e}"));
                return None;
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
                return None;
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
        Some(deltas)
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

#[allow(clippy::too_many_arguments)]
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
    delta_cache: Option<&HashMap<Vec<String>, i64>>,
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
    delta_cache: Option<&HashMap<Vec<String>, i64>>,
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
            sort_children_snapshot(&mut children, snap, sort_mode, path, delta_cache);
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
