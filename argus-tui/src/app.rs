use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;

use argus_core::{NodeIndex, Snapshot, ROOT_NODE};

use crate::ipc_client::IpcClient;
use crate::time_utils::*;
pub use crate::types::*;
use crate::util::{default_log_path, log_msg};

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
    pub path_to_tree_idx: HashMap<Vec<String>, usize>,

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

    // Delta detail popup
    pub delta_detail: Option<DeltaDetailState>,

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
            path_to_tree_idx: HashMap::new(),
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
            delta_detail: None,
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
        self.path_to_tree_idx = self
            .tree_lines
            .iter()
            .enumerate()
            .map(|(i, l)| (l.path.clone(), i))
            .collect();
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
                if self.sort_mode == SortMode::Delta {
                    self.update_tree_lines();
                } else {
                    self.refresh_filtered_lines();
                }
                log_msg(
                    &self.log_path,
                    &format!("DeltaData applied in {:?}", t0.elapsed()),
                );
            }
            AppMessage::DeltaDetailLoaded(state) => {
                self.delta_detail = Some(state);
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
        self.path_to_tree_idx = self
            .tree_lines
            .iter()
            .enumerate()
            .map(|(i, l)| (l.path.clone(), i))
            .collect();
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
        if self.server_mode {
            self.request_delta_refresh();
        }
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

    // ── delta filter tests ──────────────────────────────────────────────────

    #[test]
    fn test_delta_unit_multiplier_values() {
        assert_eq!(delta_unit_multiplier(0), 1024);
        assert_eq!(delta_unit_multiplier(1), 1024 * 1024);
        assert_eq!(delta_unit_multiplier(2), 1024 * 1024 * 1024);
        assert_eq!(delta_unit_multiplier(3), 1);
    }

    #[test]
    fn test_delta_filter_threshold_inactive() {
        let (tx, _) = mpsc::channel(1);
        let app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(app.delta_filter_threshold(), 0);
    }

    #[test]
    fn test_delta_filter_threshold_active() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.delta_filter_active = true;
        app.delta_filter_value = 50;
        app.delta_filter_unit = 1;
        assert_eq!(app.delta_filter_threshold(), 50 * 1024 * 1024);
    }

    #[test]
    fn test_delta_filter_inc_basic() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.delta_filter_value = 5;
        app.delta_filter_inc();
        assert_eq!(app.delta_filter_value, 6);
    }

    #[test]
    fn test_delta_filter_inc_unit_level_up() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.delta_filter_value = 1024;
        app.delta_filter_unit = 0;
        app.delta_filter_inc();
        assert_eq!(app.delta_filter_value, 1);
        assert_eq!(app.delta_filter_unit, 1);
    }

    #[test]
    fn test_delta_filter_inc_unit_level_up_max_unit() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.delta_filter_value = 1024;
        app.delta_filter_unit = 2;
        app.delta_filter_inc();
        assert_eq!(app.delta_filter_value, 1025);
        assert_eq!(app.delta_filter_unit, 2);
    }

    #[test]
    fn test_delta_filter_dec_basic() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.delta_filter_value = 5;
        app.delta_filter_dec();
        assert_eq!(app.delta_filter_value, 4);
    }

    #[test]
    fn test_delta_filter_dec_min_zero() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.delta_filter_value = 0;
        app.delta_filter_dec();
        assert_eq!(app.delta_filter_value, 0);
    }

    #[test]
    fn test_delta_filter_cycle_unit() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(app.delta_filter_unit, 1);
        app.delta_filter_cycle_unit();
        assert_eq!(app.delta_filter_unit, 2);
        app.delta_filter_cycle_unit();
        assert_eq!(app.delta_filter_unit, 0);
        app.delta_filter_cycle_unit();
        assert_eq!(app.delta_filter_unit, 1);
    }

    #[test]
    fn test_cursor_to_tree_idx_maps_through_filtered() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.tree_lines = vec![TreeLine {
            depth: 0,
            node: TreeNode::Snapshot(
                Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                ROOT_NODE,
            ),
            expanded: false,
            has_scan_data: false,
            path: vec!["root".into()],
        }];
        app.filtered_tree_lines = vec![0];
        app.cursor = 0;
        assert_eq!(app.cursor_to_tree_idx(), 0);
    }

    #[test]
    fn test_cursor_to_tree_idx_fallback_zero() {
        let (tx, _) = mpsc::channel(1);
        let app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(app.cursor_to_tree_idx(), 0);
    }

    // ── refresh_filtered_lines tests ─────────────────────────────────────────

    #[test]
    fn test_refresh_filtered_lines_inactive() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.tree_lines = vec![
            TreeLine {
                depth: 0,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                    ROOT_NODE,
                ),
                expanded: false,
                has_scan_data: false,
                path: vec!["a".into()],
            },
            TreeLine {
                depth: 0,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                    ROOT_NODE,
                ),
                expanded: false,
                has_scan_data: false,
                path: vec!["b".into()],
            },
        ];
        app.delta_filter_active = false;
        app.refresh_filtered_lines();
        assert_eq!(app.filtered_tree_lines, vec![0, 1]);
    }

    #[test]
    fn test_refresh_filtered_lines_active() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.tree_lines = vec![
            TreeLine {
                depth: 0,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                    ROOT_NODE,
                ),
                expanded: false,
                has_scan_data: false,
                path: vec!["a".into()],
            },
            TreeLine {
                depth: 0,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                    ROOT_NODE,
                ),
                expanded: false,
                has_scan_data: false,
                path: vec!["b".into()],
            },
            TreeLine {
                depth: 0,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                    ROOT_NODE,
                ),
                expanded: false,
                has_scan_data: false,
                path: vec!["c".into()],
            },
        ];
        app.delta_filter_active = true;
        app.delta_filter_value = 100;
        app.delta_filter_unit = 1;
        app.delta_cache = HashMap::from([
            (vec!["a".into()], 200_000_000i64),
            (vec!["b".into()], 50_000_000i64),
            (vec!["c".into()], 0i64),
        ]);
        app.refresh_filtered_lines();
        // a = 200MB >= 100MB, b = 50MB < 100MB, c = 0 < 100MB
        assert_eq!(app.filtered_tree_lines, vec![0]);
    }

    #[test]
    fn test_refresh_filtered_lines_strict_zero() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.tree_lines = vec![
            TreeLine {
                depth: 0,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                    ROOT_NODE,
                ),
                expanded: false,
                has_scan_data: false,
                path: vec!["a".into()],
            },
            TreeLine {
                depth: 0,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                    ROOT_NODE,
                ),
                expanded: false,
                has_scan_data: false,
                path: vec!["b".into()],
            },
        ];
        app.delta_filter_active = true;
        app.delta_filter_value = 0;
        app.delta_filter_unit = 0;
        app.delta_cache = HashMap::from([(vec!["a".into()], 0i64), (vec!["b".into()], 500i64)]);
        app.refresh_filtered_lines();
        // strict mode: delta > 0, so only b passes
        assert_eq!(app.filtered_tree_lines, vec![1]);
    }

    #[test]
    fn test_refresh_filtered_lines_cursor_clamp() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.tree_lines = vec![TreeLine {
            depth: 0,
            node: TreeNode::Snapshot(
                Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                ROOT_NODE,
            ),
            expanded: false,
            has_scan_data: false,
            path: vec!["a".into()],
        }];
        app.delta_filter_active = true;
        app.delta_filter_value = 1;
        app.delta_filter_unit = 2;
        app.delta_cache = HashMap::from([(vec!["a".into()], 0i64)]);
        app.filtered_tree_lines = vec![0];
        app.cursor = 0;
        app.refresh_filtered_lines();
        // line filtered out, cursor should be 0 when filtered_tree_lines is empty
        assert!(app.filtered_tree_lines.is_empty());
        assert_eq!(app.cursor, 0);
    }

    // ── clear_filter_pane tests ──────────────────────────────────────────────

    #[test]
    fn test_clear_filter_pane_resets_state() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.delta_filter_active = true;
        app.delta_filter_value = 500;
        app.delta_filter_unit = 2;
        app.focus = Focus::FilterPane;
        app.clear_filter_pane();
        assert!(!app.delta_filter_active);
        assert_eq!(app.delta_filter_value, 100);
        assert_eq!(app.delta_filter_unit, 1);
        assert_eq!(app.focus, Focus::Tree);
    }

    // ── command history tests ─────────────────────────────────────────────────

    #[test]
    fn test_push_command_history_empty() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.push_command_history("");
        assert!(app.command_history.is_empty());
    }

    #[test]
    fn test_push_command_history_trimmed() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.push_command_history("  scan  ");
        assert_eq!(app.command_history, vec!["scan"]);
    }

    #[test]
    fn test_push_command_history_dedup() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.push_command_history("scan");
        app.push_command_history("scan");
        assert_eq!(app.command_history.len(), 1);
    }

    #[test]
    fn test_push_command_history_cap() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        for i in 0..60 {
            app.push_command_history(&format!("cmd{i}"));
        }
        assert_eq!(app.command_history.len(), 50);
        assert_eq!(app.command_history[0], "cmd10");
        assert_eq!(app.command_history[49], "cmd59");
    }

    #[test]
    fn test_push_command_history_resets_idx() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.command_history_idx = Some(3);
        app.push_command_history("scan");
        assert_eq!(app.command_history_idx, None);
    }

    #[test]
    fn test_clear_command_state() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.command_input = "scan".into();
        app.command_matches = vec!["Scan"];
        app.command_selected = 1;
        app.clear_command_state();
        assert!(app.command_input.is_empty());
        assert!(app.command_matches.is_empty());
        assert_eq!(app.command_selected, 0);
    }

    // ── update_command_matches tests ──────────────────────────────────────────

    #[test]
    fn test_update_command_matches_empty() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.update_command_matches();
        assert_eq!(app.command_matches.len(), App::COMMANDS.len());
    }

    #[test]
    fn test_update_command_matches_fuzzy() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.command_input = "sc".into();
        app.update_command_matches();
        assert!(app.command_matches.contains(&"Scan"));
    }

    // ── cmd_* direct tests ────────────────────────────────────────────────────

    #[test]
    fn test_cmd_scan_not_scanning() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert!(app.cmd_scan().is_ok());
    }

    #[test]
    fn test_cmd_scan_already_scanning() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.scanning = true;
        assert_eq!(app.cmd_scan(), Err("already scanning".into()));
    }

    #[test]
    fn test_cmd_consolidate_not_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(app.cmd_consolidate(), Err("not in server mode".into()));
    }

    #[test]
    fn test_cmd_consolidate_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        assert!(app.cmd_consolidate().is_ok());
    }

    #[test]
    fn test_cmd_filterfocus_not_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(app.cmd_filterfocus(), Err("not in server mode".into()));
    }

    #[test]
    fn test_cmd_filterfocus_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        app.focus = Focus::Tree;
        assert!(app.cmd_filterfocus().is_ok());
        assert_eq!(app.focus, Focus::FilterPane);
        assert_eq!(app.filter_focus, FilterFocus::TimePreset);
    }

    #[test]
    fn test_cmd_sort_quick_delta() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.sort_mode = SortMode::Name;
        let result = app.cmd_sort_quick(SortMode::Delta, "Delta");
        assert!(result.is_ok());
        assert_eq!(app.sort_mode, SortMode::Delta);
    }

    #[test]
    fn test_cmd_sort_quick_size() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.sort_mode = SortMode::Name;
        let result = app.cmd_sort_quick(SortMode::Size, "Size");
        assert!(result.is_ok());
        assert_eq!(app.sort_mode, SortMode::Size);
    }

    // ── default_log_path tests ────────────────────────────────────────────────

    #[test]
    fn test_default_log_path_returns_non_empty() {
        let path = default_log_path();
        assert!(path.ends_with("argus.log"));
    }

    // ── time methods tests ────────────────────────────────────────────────────

    #[test]
    fn test_set_time_preset_0() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.set_time_preset(0);
        assert!(!app.time_custom);
        assert!(app.time_from < app.time_to);
        // 1h = 3_600_000 ms
        assert!(app.time_to - app.time_from <= 3_600_000);
    }

    #[test]
    fn test_set_time_preset_7d() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.set_time_preset(5);
        assert_eq!(app.time_preset, 5);
        // 7d = 604_800_000 ms
        let diff = app.time_to - app.time_from;
        assert!(diff >= 604_800_000 - 1000); // account for small time drift
        assert!(diff <= 604_800_000 + 1000);
    }

    #[test]
    fn test_time_preset_label_all() {
        assert_eq!(App::time_preset_label(0), "1h");
        assert_eq!(App::time_preset_label(1), "6h");
        assert_eq!(App::time_preset_label(2), "12h");
        assert_eq!(App::time_preset_label(3), "1d");
        assert_eq!(App::time_preset_label(4), "3d");
        assert_eq!(App::time_preset_label(5), "7d");
        assert_eq!(App::time_preset_label(99), "1h");
    }

    #[test]
    fn test_default_time_range() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.default_time_range();
        assert_eq!(app.time_preset, 0);
        assert!(!app.time_custom);
    }

    // ── execute_command additional tests ──────────────────────────────────────

    #[test]
    fn test_execute_delta_no_unit_uses_mb() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        assert!(app.execute_command("delta 200").is_ok());
        assert_eq!(app.delta_filter_value, 200);
        assert_eq!(app.delta_filter_unit, 1);
    }

    #[test]
    fn test_execute_delta_m_unit() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        assert!(app.execute_command("delta 50m").is_ok());
        assert_eq!(app.delta_filter_value, 50);
        assert_eq!(app.delta_filter_unit, 1);
    }

    #[test]
    fn test_execute_delta_g_unit() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        assert!(app.execute_command("delta 2g").is_ok());
        assert_eq!(app.delta_filter_value, 2);
        assert_eq!(app.delta_filter_unit, 2);
    }

    #[test]
    fn test_execute_delta_invalid_number() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        let result = app.execute_command("delta abc");
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_sort_shortcuts() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert!(app.execute_command("sd").is_ok());
        assert_eq!(app.sort_mode, SortMode::Delta);
        assert!(app.execute_command("ss").is_ok());
        assert_eq!(app.sort_mode, SortMode::Size);
        assert!(app.execute_command("sn").is_ok());
        assert_eq!(app.sort_mode, SortMode::Name);
    }

    #[test]
    fn test_execute_scan() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert!(app.execute_command("scan").is_ok());
    }

    #[test]
    fn test_execute_consolidate_not_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(
            app.execute_command("consolidate"),
            Err("not in server mode".into())
        );
    }

    #[test]
    fn test_execute_consolidate_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        assert!(app.execute_command("consolidate").is_ok());
    }

    #[test]
    fn test_execute_filterfocus_not_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert_eq!(
            app.execute_command("filterfocus"),
            Err("not in server mode".into())
        );
    }

    #[test]
    fn test_execute_filterfocus_server_mode() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.server_mode = true;
        assert!(app.execute_command("filterfocus").is_ok());
        assert_eq!(app.focus, Focus::FilterPane);
    }

    // ── SortMode tests ────────────────────────────────────────────────────────

    #[test]
    fn test_sort_mode_toggle() {
        assert_eq!(SortMode::Name.toggle(), SortMode::Size);
        assert_eq!(SortMode::Size.toggle(), SortMode::Delta);
        assert_eq!(SortMode::Delta.toggle(), SortMode::Name);
    }

    #[test]
    fn test_sort_mode_label() {
        assert_eq!(SortMode::Name.label(), "Name");
        assert_eq!(SortMode::Size.label(), "Size");
        assert_eq!(SortMode::Delta.label(), "Delta");
    }

    // ── TreeNode tests ────────────────────────────────────────────────────────

    #[test]
    fn test_tree_node_snapshot_basics() {
        let arena = vec![
            FileNode {
                name: "root".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 0,
                children: vec![("file.txt".into(), 1)],
            },
            FileNode {
                name: "file.txt".into(),
                parent: None,
                is_dir: false,
                file_type: FileType::File,
                size: 1024,
                children: Vec::new(),
            },
        ];
        let snap = Arc::new(Snapshot::new(PathBuf::from("/tmp"), arena, 0));
        let root = TreeNode::Snapshot(snap.clone(), ROOT_NODE);
        assert!(root.is_dir());
        assert_eq!(root.name(), "root");
        assert_eq!(root.file_type(), FileType::Directory);
        assert_eq!(root.current_size(), 0);

        let file = TreeNode::Snapshot(snap.clone(), 1);
        assert!(!file.is_dir());
        assert_eq!(file.name(), "file.txt");
        assert_eq!(file.file_type(), FileType::File);
        assert_eq!(file.current_size(), 1024);
    }

    // ── selected_node_full_path tests ─────────────────────────────────────────

    #[test]
    fn test_selected_node_full_path_skips_root() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.view_root_path = PathBuf::from("/home/user");
        app.tree_lines = vec![TreeLine {
            depth: 0,
            node: TreeNode::Snapshot(
                Arc::new(Snapshot::new(PathBuf::from("/home/user"), vec![], 0)),
                ROOT_NODE,
            ),
            expanded: false,
            has_scan_data: false,
            path: vec!["user".into(), "docs".into(), "file.txt".into()],
        }];
        app.filtered_tree_lines = vec![0];
        app.cursor = 0;
        let path = app.selected_node_full_path();
        assert_eq!(path, Some(PathBuf::from("/home/user/docs/file.txt")));
    }

    #[test]
    fn test_selected_node_full_path_empty_tree() {
        let (tx, _) = mpsc::channel(1);
        let app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert!(app.selected_node_full_path().is_none());
    }

    // ── set_error tests ───────────────────────────────────────────────────────

    #[test]
    fn test_set_error_sets_last_error() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.set_error("test error".into(), 5);
        assert_eq!(app.last_error.as_deref(), Some("test error"));
        assert!(app.error_clear_at.is_some());
    }

    // ── selected_line tests ───────────────────────────────────────────────────

    #[test]
    fn test_selected_line_returns_none_when_empty() {
        let (tx, _) = mpsc::channel(1);
        let app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert!(app.selected_line().is_none());
    }

    #[test]
    fn test_selected_line_maps_through_filtered() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        let line = TreeLine {
            depth: 0,
            node: TreeNode::Snapshot(
                Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                ROOT_NODE,
            ),
            expanded: false,
            has_scan_data: false,
            path: vec!["root".into()],
        };
        app.tree_lines = vec![line.clone()];
        app.filtered_tree_lines = vec![0];
        app.cursor = 0;
        assert_eq!(app.selected_line().unwrap().depth, 0);
    }

    // ── tree_line_relative_path tests ─────────────────────────────────────────

    #[test]
    fn test_tree_line_relative_path_returns_path() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.tree_lines = vec![TreeLine {
            depth: 0,
            node: TreeNode::Snapshot(
                Arc::new(Snapshot::new(PathBuf::from("/"), vec![], 0)),
                ROOT_NODE,
            ),
            expanded: false,
            has_scan_data: false,
            path: vec!["root".into(), "dir".into()],
        }];
        app.filtered_tree_lines = vec![0];
        let path = app.tree_line_relative_path(0);
        assert_eq!(path, Some(vec!["root".into(), "dir".into()]));
    }

    #[test]
    fn test_tree_line_relative_path_out_of_bounds() {
        let (tx, _) = mpsc::channel(1);
        let app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        assert!(app.tree_line_relative_path(0).is_none());
    }
}
