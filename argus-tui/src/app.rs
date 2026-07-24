use ratatui_finder::FinderState;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;

use argus_core::{labels, Snapshot, ROOT_NODE};

use crate::ipc_client::IpcClient;
use crate::theme::ColorTheme;
use crate::time_utils::*;
use crate::tree_ops;
pub use crate::types::*;
use crate::util::{default_log_path, log_msg};

/// A snapshot of navigation state for back/forward history.
#[derive(Debug, Clone)]
pub struct NavPosition {
    pub current_dir_path: Vec<String>,
    pub view_root_path: PathBuf,
    pub cursor: usize,
    pub scroll_offset: usize,
}

// ── App ─────────────────────────────────────────────────────────────────────

pub struct App {
    pub config: crate::config::TuiConfig,
    pub theme: ColorTheme,
    pub mode: AppMode,
    pub sort_mode: SortMode,

    // View root (always set, initialized to cwd)
    pub view_root_path: PathBuf,

    // Tree state
    pub tree_root: Option<TreeNode>,
    pub cursor: usize,
    pub scroll_offset: usize,

    // Scan cache: path → full scanned snapshot (shared; treat as immutable after insert)
    pub scan_cache: HashMap<PathBuf, Arc<Snapshot>>,

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
    pub delta_filter_active: bool,
    pub delta_filter_value: u64,
    pub delta_filter_unit: usize, // 0=KB, 1=MB, 2=GB
    pub delta_pending: bool,

    // Tree search (fuzzy search)
    pub search_word: String,
    pub search_mode: SearchMode,
    /// Indices of current_children that match the search word, for n/N navigation.
    pub search_match_indices: Vec<usize>,

    // gg double-tap tracking
    pub pending_gg: bool,

    // Scan state
    pub scanning: bool,
    pub scan_progress: Option<(u64, u64)>,
    pub scan_current_path: Option<String>,
    pub scan_spinner: u8,
    pub scan_spinner_tick: Instant,
    pub scan_started_at: Option<Instant>,
    pub last_scan_summary: Option<ScanSummary>,
    pub cancel_scan: Arc<AtomicBool>,

    // Delete state
    pub delete_target_path: Option<PathBuf>,
    pub delete_target_paths: Vec<PathBuf>,

    // Delete progress
    pub deleting: bool,
    pub delete_progress: Option<(u64, u64)>,
    pub delete_permanent: bool,

    // AI spinner for loading animation
    pub ai_spinner: u8,
    pub ai_spinner_tick: Instant,

    // Multi-select state
    pub multi_select: bool,
    pub selected_paths: HashSet<Vec<String>>,

    // Message channel
    pub tx: mpsc::Sender<AppMessage>,
    pub rx: mpsc::Receiver<AppMessage>,

    // Status message display (error or info)
    pub last_error: Option<String>,
    pub status_is_error: bool,
    pub error_clear_at: Option<std::time::Instant>,

    // Log file path (~/.config/argus/argus.log)
    pub log_path: PathBuf,

    // Info popup
    pub info_data: Option<(std::path::PathBuf, std::fs::Metadata)>,
    pub info_ai: Option<AiPathVerdict>,

    // Delta detail popup
    pub delta_detail: Option<DeltaDetailState>,

    // Command bar
    pub command_input: String,
    pub command_matches: Vec<&'static str>,
    pub command_selected: usize,
    pub command_scroll: usize,
    pub command_history: Vec<String>,
    pub command_history_idx: Option<usize>,

    // Delete tracking
    pub deleted_bytes: u64,

    // Time help popup
    pub time_help_scroll: usize,
    pub should_quit: bool,

    // Finder (Go to Path)
    pub finder_state: Option<FinderState>,

    // AI review
    pub ai_state: Option<AiReviewState>,
    pub ai_cache: HashMap<PathBuf, AiPathVerdict>,
    pub ai_analyzed: HashMap<PathBuf, RiskLevel>,

    // Cleanup / Uninstall
    pub cleanup_state: Option<CleanupState>,
    pub uninstall_state: Option<UninstallState>,

    // Hidden files
    pub show_hidden: bool,

    // ── Flat mode (ncdu-like) fields ─────────────────────────────────
    /// Current directory's direct children (entries visible in the flat view)
    pub current_children: Vec<DirEntry>,

    /// Filtered indices into current_children (search + delta filter)
    pub current_filtered: Vec<usize>,

    /// Current directory's relative path from view_root (e.g. [root_name, "src"])
    pub current_dir_path: Vec<String>,

    /// Navigation stack: each entry directory push, `h` pops
    pub dir_stack: Vec<Vec<String>>,

    /// Navigation history for back/forward (b/f)
    pub nav_history: Vec<NavPosition>,
    pub nav_history_idx: usize,

    /// Total size of the current directory (for percentage calculation)
    pub current_dir_total: u64,

    /// Total disk usage of the current directory
    pub current_dir_disk_usage: u64,

    /// Number of visible items in the current directory
    pub current_dir_items: u64,

    /// Total size of the parent directory
    pub parent_dir_total: u64,
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

        let theme = crate::theme::detect_theme(&config.theme.color_scheme);

        Self {
            config,
            theme,
            tx,
            mode: AppMode::Browsing,
            sort_mode: SortMode::Size,
            view_root_path,
            tree_root: None,
            cursor: 0,
            scroll_offset: 0,
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
            delta_filter_active: false,
            delta_filter_value: 100,
            delta_filter_unit: 1,
            delta_pending: false,
            search_word: String::new(),
            search_mode: SearchMode::Inactive,
            search_match_indices: Vec::new(),
            pending_gg: false,
            scanning: false,
            scan_progress: None,
            scan_current_path: None,
            scan_spinner: 0,
            scan_spinner_tick: Instant::now(),
            scan_started_at: None,
            last_scan_summary: None,
            cancel_scan: Arc::new(AtomicBool::new(false)),
            delete_target_path: None,
            delete_target_paths: Vec::new(),
            deleting: false,
            delete_progress: None,
            delete_permanent: false,
            ai_spinner: 0,
            ai_spinner_tick: Instant::now(),
            multi_select: false,
            selected_paths: HashSet::new(),
            deleted_bytes: 0,
            info_data: None,
            info_ai: None,
            delta_detail: None,
            command_input: String::new(),
            command_matches: Vec::new(),
            command_selected: 0,
            command_scroll: 0,
            command_history: Vec::new(),
            command_history_idx: None,
            rx,
            last_error: None,
            status_is_error: false,
            error_clear_at: None,
            log_path,
            time_help_scroll: 0,
            should_quit: false,
            finder_state: None,
            ai_state: None,
            ai_cache: HashMap::new(),
            ai_analyzed: {
                let mut map = HashMap::new();
                if let Ok(conn) = argus_core::open_db(&argus_core::default_db_path()) {
                    if let Ok(paths) = argus_core::load_all_ai_analyzed_paths(&conn) {
                        for p in paths {
                            let path = PathBuf::from(&p);
                            let risk = argus_core::get_ai_analysis(&conn, &p)
                                .ok()
                                .flatten()
                                .and_then(|data| {
                                    serde_json::from_slice::<AiPathVerdict>(&data).ok()
                                })
                                .map(|v| v.risk_level)
                                .unwrap_or(RiskLevel::Medium);
                            map.insert(path, risk);
                        }
                    }
                }
                map
            },
            cleanup_state: None,
            uninstall_state: None,
            show_hidden: false,
            current_children: Vec::new(),
            current_filtered: Vec::new(),
            current_dir_path: Vec::new(),
            dir_stack: Vec::new(),
            nav_history: Vec::new(),
            nav_history_idx: 0,
            current_dir_total: 0,
            current_dir_disk_usage: 0,
            current_dir_items: 0,
            parent_dir_total: 0,
        }
    }

    /// Build tree for current view_root_path from scan_cache or filesystem
    pub fn rebuild_tree(&mut self) {
        self.build_current_tree();
        // Reset flat mode navigation state
        self.current_dir_path.clear();
        self.dir_stack.clear();
        // Populate flat mode children
        self.load_current_children();
        if self.server_mode {
            self.request_delta_refresh();
        }
    }

    fn build_current_tree(&mut self) {
        // Use full scan data when available, otherwise fall back to list_dir (one level)
        if let Some(snapshot) = self.scan_cache.get(&self.view_root_path) {
            self.tree_root = Some(TreeNode::Snapshot(Arc::clone(snapshot), ROOT_NODE));
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

                    // Recompute root totals from enriched children
                    let mut total_size = 0u64;
                    let mut total_disk = 0u64;
                    for &child_idx in snap.children(ROOT_NODE) {
                        let c = snap.node(child_idx);
                        total_size = total_size.saturating_add(c.size());
                        total_disk = total_disk.saturating_add(c.disk_usage());
                    }
                    snap.root_mut().set_size(total_size);
                    snap.root_mut().set_disk_usage(total_disk);
                    snap.total_size = total_size;
                    snap.total_disk_usage = total_disk;

                    self.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
                }
                Err(e) => {
                    self.set_error(format!("failed to list directory: {}", e), 3);
                    self.tree_root = None;
                }
            }
        }
    }

    /// Handle a message from background tasks
    pub fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::ScanProgress {
                file_count,
                total_bytes,
                total_disk_bytes: _,
                current_path,
            } => {
                self.scan_progress = Some((file_count, total_bytes));
                self.scan_current_path = current_path;
            }
            AppMessage::ScanComplete(snapshot) => {
                self.scanning = false;
                self.scan_current_path = None;
                let duration = self
                    .scan_started_at
                    .take()
                    .map(|started| started.elapsed())
                    .unwrap_or_default();
                self.last_scan_summary = Some(ScanSummary {
                    root_path: snapshot.root_path.clone(),
                    total_size: snapshot.total_size,
                    total_disk_usage: snapshot.total_disk_usage,
                    total_files: snapshot.total_files,
                    duration,
                });
                self.scan_progress = None;

                // Update scan cache (share Arc; no full Snapshot clone)
                let root_path = snapshot.root_path.clone();
                let matches_view =
                    root_path == self.view_root_path || root_path == self.current_scan_path();
                let was_in_subdir = self.current_dir_path.len() > 1;
                let scanned_subdir = root_path != self.view_root_path;
                self.scan_cache
                    .insert(root_path.clone(), Arc::new(snapshot));

                // Rebuild tree if scanned path matches current view
                if matches_view {
                    if scanned_subdir {
                        // Subdirectory was scanned: update view_root_path so
                        // rebuild_tree() finds the scan result in the cache.
                        self.view_root_path = root_path.clone();
                    }
                    // Only restore saved_dir when scanning the root (not a subdirectory),
                    // because a subdirectory scan changes view_root_path and invalidates
                    // the old navigation path which referenced the parent tree root name.
                    let saved_dir = if was_in_subdir && !scanned_subdir {
                        Some(self.current_dir_path.clone())
                    } else {
                        None
                    };
                    self.rebuild_tree();
                    // Restore browsing position after rebuild
                    if let Some(dir) = saved_dir {
                        if dir.len() > 1 {
                            self.current_dir_path = dir;
                            self.load_current_children();
                        }
                    }
                }
            }
            AppMessage::Error(e) => {
                self.scanning = false;
                self.scan_current_path = None;
                self.scan_started_at = None;
                self.set_error(e, 5);
            }
            AppMessage::DaemonConnected(client) => {
                self.server_mode = true;
                self.server_connected = true;
                self.daemon_client = Some(client);
                self.default_time_range();
                self.set_info("connected to daemon".into(), 2);
                self.request_delta_refresh();
            }
            AppMessage::DaemonDisconnected => {
                self.server_connected = false;
                self.server_mode = false;
                self.daemon_client = None;
                self.delta_cache.clear();
                self.delta_filter_active = false;
                self.refresh_current_filtered();
                self.set_error("daemon disconnected".into(), 4);
            }
            AppMessage::DeltaData(deltas, returned_client) => {
                let t0 = Instant::now();
                self.delta_pending = false;
                self.delta_cache = deltas;
                if let Some(client) = returned_client {
                    self.daemon_client = Some(client);
                }
                self.load_current_children();
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
            AppMessage::DeleteProgress { current, total } => {
                self.delete_progress = Some((current, total));
            }
            AppMessage::DeleteComplete { errors, paths } => {
                self.deleting = false;
                self.delete_progress = None;
                self.mode = AppMode::Browsing;

                let mut total_freed = 0u64;
                for path in &paths {
                    let freed = crate::tree_ops::apply_deletion_to_state(self, path);
                    total_freed = total_freed.saturating_add(freed);
                }
                self.deleted_bytes = self.deleted_bytes.saturating_add(total_freed);
                self.load_current_children();
                self.exit_multi_select();

                if !errors.is_empty() {
                    self.set_error(
                        format!("{} delete(s) failed: {}", errors.len(), errors.join("; ")),
                        5,
                    );
                } else {
                    self.set_info(format!("deleted {} item(s)", paths.len()), 3);
                }
            }
            AppMessage::AiAnalysisComplete(results) => {
                if let Some(ref mut state) = self.ai_state {
                    for result in &results {
                        self.ai_cache.insert(result.path.clone(), result.clone());
                        self.ai_analyzed
                            .insert(result.path.clone(), result.risk_level);
                    }
                    state.pending_paths.clear();
                    state.results = {
                        let mut r = results;
                        r.sort_by(|a, b| a.path.cmp(&b.path));
                        r
                    };
                    state.status = AiStatus::Ready;
                }
                self.load_current_children();
            }
            AppMessage::AiAnalysisError(msg) => {
                self.set_error(format!("AI analysis error: {}", msg), 10);
                if let Some(ref mut state) = self.ai_state {
                    state.results = Vec::new();
                    state.status = AiStatus::Error(msg);
                }
            }
            AppMessage::CleanupScanComplete { mut items, total_bytes } => {
                self.scanning = false;
                if let Some(ref mut state) = self.cleanup_state {
                    state.scanning = false;
                    items.sort_by(|a, b| b.size.cmp(&a.size));
                    state.items = items;
                    state.total_bytes = total_bytes;
                    // Pre-select items over 1GB
                    for (i, item) in state.items.iter().enumerate() {
                        if item.size > 1_000_000_000 {
                            state.selected.insert(i);
                        }
                    }
                }
            }
            AppMessage::CleanupExecComplete(report) => {
                if let Some(ref mut state) = self.cleanup_state {
                    state.confirm_pending = false;
                    state.report = Some(report);
                }
            }
            AppMessage::CleanupScanProgress(path) => {
                if let Some(ref mut state) = self.cleanup_state {
                    state.scan_current_path = Some(path);
                }
            }
            AppMessage::CleanupDetailReady(details) => {
                if let Some(ref mut state) = self.cleanup_state {
                    state.detail_items = Some(details);
                }
            }
            AppMessage::AppListReady(apps) => {
                self.scanning = false;
                if let Some(ref mut state) = self.uninstall_state {
                    state.scanning = false;
                    state.apps = apps;
                    state.filtered = (0..state.apps.len()).collect();
                }
            }
            AppMessage::UninstallScanProgress(path) => {
                if let Some(ref mut state) = self.uninstall_state {
                    state.scan_current_path = Some(path);
                }
            }
            AppMessage::UninstallLeftoversReady(leftovers) => {
                if let Some(ref mut state) = self.uninstall_state {
                    state.scanning = false;
                    state.leftovers = Some(leftovers.clone());
                    // Pre-select all leftovers
                    state.selected_leftovers = (0..leftovers.leftover_paths.len()).collect();
                }
            }
            AppMessage::UninstallComplete(report) => {
                if let Some(ref mut state) = self.uninstall_state {
                    state.confirm_pending = false;
                    state.report = Some(report);
                }
            }
        }
    }

    /// Set error message and log to file.
    pub fn set_error(&mut self, msg: String, duration_secs: u64) {
        self.last_error = Some(msg.clone());
        self.status_is_error = true;
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

    /// Set info/status message (not an error). Displayed in status bar with success color.
    pub fn set_info(&mut self, msg: String, duration_secs: u64) {
        self.last_error = Some(msg);
        self.status_is_error = false;
        self.error_clear_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(duration_secs));
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

    /// Clear the delta filter and reset time
    pub fn clear_filter_pane(&mut self) {
        self.delta_filter_active = false;
        self.delta_filter_value = 100;
        self.delta_filter_unit = 1;
        self.set_time_preset(0);
        self.refresh_current_filtered();
        if self.server_mode {
            self.request_delta_refresh();
        }
        self.set_info("filter cleared".into(), 2);
    }

    /// Get the full path of the directory currently being browsed.
    pub fn current_scan_path(&self) -> PathBuf {
        if self.current_dir_path.len() > 1 {
            let mut path = self.view_root_path.clone();
            for component in self.current_dir_path.iter().skip(1) {
                path.push(component);
            }
            path
        } else {
            self.view_root_path.clone()
        }
    }

    /// Enter multi-select mode
    pub fn enter_multi_select(&mut self) {
        self.multi_select = true;
    }

    /// Exit multi-select mode and clear selections
    pub fn exit_multi_select(&mut self) {
        self.multi_select = false;
        self.selected_paths.clear();
    }

    /// Toggle selection of the item at the current cursor position.
    /// If the item is a descendant of an already-selected directory, it cannot be toggled.
    pub fn toggle_selection(&mut self) {
        if let Some(entry) = self.selected_entry() {
            if self.is_inherited_selection(&entry.path) {
                return;
            }
            let path = entry.path.clone();
            if self.selected_paths.contains(&path) {
                self.selected_paths.remove(&path);
            } else {
                self.selected_paths.insert(path);
            }
        }
    }

    /// Check if a path is a descendant of any selected directory.
    pub fn is_inherited_selection(&self, path: &[String]) -> bool {
        self.selected_paths
            .iter()
            .any(|p| p.len() < path.len() && path.starts_with(p))
    }

    /// Get the full path for a relative path key
    pub fn selected_paths_full(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for path_key in &self.selected_paths {
            let mut path = self.view_root_path.clone();
            for part in path_key.iter().skip(1) {
                path.push(part);
            }
            paths.push(path);
        }
        paths
    }

    pub fn selected_node_full_path(&self) -> Option<PathBuf> {
        let entry = self.selected_entry()?;
        let mut path = self.view_root_path.clone();
        // path includes the root dir name as the first component;
        // skip it because view_root_path already contains the full root path.
        for part in entry.path.iter().skip(1) {
            path.push(part);
        }
        Some(path)
    }

    /// Sum of `size` for all selected entries in `current_children`.
    pub fn selected_total_size(&self) -> u64 {
        self.current_children
            .iter()
            .filter(|e| self.selected_paths.contains(&e.path))
            .map(|e| e.size)
            .sum()
    }

    // ── Flat mode methods ───────────────────────────────────────────────

    /// Load children of the current directory from the snapshot into `current_children`.
    /// Replaces (does not append). Also updates `current_filtered`, `current_dir_total`,
    /// and `parent_dir_total`.
    pub fn load_current_children(&mut self) {
        // Search matches are stale after children reload
        self.search_word.clear();
        self.search_match_indices.clear();
        self.search_mode = SearchMode::Inactive;

        // Rebuild tree at root level to pick up scan cache updates from deletions
        if self.current_dir_path.len() <= 1 {
            self.build_current_tree();
        }

        // Check if listing is needed before taking any borrow on tree_root
        let needs_listing = self.current_dir_path.len() > 1
            && self.tree_root.as_ref().is_some_and(|tr| {
                let TreeNode::Snapshot(snap, idx) = tr;
                snap.find_node(*idx, &self.current_dir_path)
                    .is_some_and(|fidx| snap.node(fidx).is_dir() && snap.children_is_empty(fidx))
            });

        if needs_listing {
            let full_path = {
                let mut p = self.view_root_path.clone();
                for part in self.current_dir_path.iter().skip(1) {
                    p.push(part);
                }
                p
            };
            if let Ok(listed) = argus_core::list_dir(&full_path) {
                if let Some(TreeNode::Snapshot(snap_arc, _)) = &mut self.tree_root {
                    let snap_mut = Arc::make_mut(snap_arc);
                    let target_idx = snap_mut
                        .find_node(ROOT_NODE, &self.current_dir_path)
                        .unwrap_or(ROOT_NODE);
                    snap_mut.graft_children_from(target_idx, &listed);
                }
            }
        }

        let Some(TreeNode::Snapshot(snap_arc, root_idx)) = &self.tree_root else {
            self.current_children.clear();
            self.current_filtered.clear();
            return;
        };
        let snap = snap_arc.as_ref();

        // Initialize current_dir_path to root name if empty
        if self.current_dir_path.is_empty() {
            let root_name = snap.name(*root_idx).to_string();
            self.current_dir_path = vec![root_name];
        }

        // (1) Locate current directory node in snapshot
        let dir_idx = if self.current_dir_path.len() <= 1 {
            *root_idx
        } else {
            match snap.find_node(*root_idx, &self.current_dir_path) {
                Some(idx) => idx,
                None => {
                    // current_dir_path should always exist in the snapshot
                    self.current_children.clear();
                    self.current_filtered.clear();
                    return;
                }
            }
        };

        let dir_node = snap.node(dir_idx);
        let dir_children_len = snap.children_len(dir_idx);
        let dir_size = dir_node.size();
        let dir_disk = dir_node.disk_usage();

        // (2) Resolve scan tree for has_scan_data lookups
        let root_scan_tree = tree_ops::resolve_scan_tree(&self.scan_cache, &self.view_root_path);

        // (3) Collect children
        let mut children: Vec<DirEntry> = Vec::with_capacity(dir_children_len);

        for &child_idx in snap.children(dir_idx) {
            let child_node = snap.node(child_idx);
            let name = snap.name(child_idx);

            // Skip hidden files if show_hidden is false
            if !self.show_hidden && name.starts_with('.') {
                continue;
            }

            let mut child_path = self.current_dir_path.clone();
            child_path.push(name.to_string());

            let has_scan = if child_node.is_dir() {
                tree_ops::size_for_path(
                    &self.scan_cache,
                    &self.view_root_path,
                    root_scan_tree,
                    &child_path,
                )
                .is_some()
            } else {
                // Files always have their own size data
                true
            };

            children.push(DirEntry {
                node: TreeNode::Snapshot(snap_arc.clone(), child_idx),
                path: child_path,
                has_scan_data: has_scan,
                has_ai: false,
                ai_risk_level: None,
                is_dir: child_node.is_dir(),
                size: child_node.size(),
                disk_usage: child_node.disk_usage(),
            });
        }

        // (4) Sort children
        sort_children(&mut children, self.sort_mode, &self.delta_cache);

        // (4.5) Populate AI analysis flags
        for entry in &mut children {
            let mut full_path = self.view_root_path.clone();
            for part in entry.path.iter().skip(1) {
                full_path.push(part);
            }
            entry.has_ai = self.ai_analyzed.contains_key(&full_path);
            entry.ai_risk_level = self.ai_analyzed.get(&full_path).copied();
        }

        // (5) Update totals
        self.current_children = children;
        self.current_dir_total = dir_size;
        self.current_dir_disk_usage = dir_disk;
        self.current_dir_items = dir_children_len as u64;
        self.parent_dir_total = if self.current_dir_path.len() <= 1 {
            dir_size
        } else {
            let parent_path = &self.current_dir_path[..self.current_dir_path.len() - 1];
            match snap.find_node(*root_idx, parent_path) {
                Some(pidx) => snap.node(pidx).size(),
                None => dir_size,
            }
        };

        // (6) Re-apply filters
        self.refresh_current_filtered();
    }

    /// Get the currently selected entry (from filtered view via cursor)
    pub fn selected_entry(&self) -> Option<&DirEntry> {
        self.current_filtered
            .get(self.cursor)
            .and_then(|&idx| self.current_children.get(idx))
    }

    /// Rebuild `current_filtered` from `current_children` based on delta filter
    /// and search word. Clamps cursor if out of bounds.
    pub fn refresh_current_filtered(&mut self) {
        // Start with all indices
        self.current_filtered = (0..self.current_children.len()).collect();

        // Apply delta filter if active
        if self.delta_filter_active {
            let threshold = self.delta_filter_value
                * crate::types::delta_unit_multiplier(self.delta_filter_unit);
            let strict = self.delta_filter_value == 0;
            self.current_filtered.retain(|&i| {
                let delta = self
                    .delta_cache
                    .get(&self.current_children[i].path)
                    .copied()
                    .unwrap_or(0);
                if strict {
                    delta > 0
                } else {
                    (delta as u64) >= threshold
                }
            });
        }

        // (No search filter — non-matching items stay visible, highlighted in render)

        // Clamp cursor
        if self.cursor >= self.current_filtered.len() && !self.current_filtered.is_empty() {
            self.cursor = self.current_filtered.len() - 1;
        } else if self.current_filtered.is_empty() {
            self.cursor = 0;
        }
    }

    /// Enter the selected directory (flat mode).
    /// Pushes current directory to stack, updates current_dir_path, and reloads children.
    /// If the target subdirectory has a cached scan result, switches to that scan.
    pub fn enter_directory(&mut self) {
        let entry_path = self.selected_entry().map(|e| (e.is_dir, e.path.clone()));
        let Some((is_dir, path)) = entry_path else {
            return;
        };
        if !is_dir {
            return;
        }

        // Build full filesystem path for the target subdirectory
        let mut full_path = self.view_root_path.clone();
        for comp in path.iter().skip(1) {
            full_path.push(comp);
        }

        // If this subdirectory was previously scanned, switch to that scan
        if self.scan_cache.contains_key(&full_path) {
            self.view_root_path = full_path;
            self.dir_stack.clear();
            self.rebuild_tree();
            self.push_nav_history();
            self.set_info(
                format!("switched to cached scan: {}", self.view_root_path.display()),
                2,
            );
            return;
        }

        self.dir_stack.push(self.current_dir_path.clone());
        self.current_dir_path = path;
        self.cursor = 0;
        self.scroll_offset = 0;
        self.load_current_children();
        self.push_nav_history();
    }

    /// Go to parent directory (flat mode).
    /// Pops the navigation stack and reloads children.
    pub fn go_to_parent(&mut self) {
        if self.current_dir_path.len() <= 1 {
            return; // Already at root
        }
        if let Some(prev_path) = self.dir_stack.pop() {
            self.current_dir_path = prev_path;
        } else {
            self.current_dir_path.clear();
            self.dir_stack.clear();
        }
        self.cursor = 0;
        self.scroll_offset = 0;
        self.load_current_children();
        self.push_nav_history();
    }

    /// Go to root directory (flat mode).
    /// Clears the navigation stack and reloads children at the root level.
    pub fn go_to_root(&mut self) {
        self.dir_stack.clear();
        self.current_dir_path.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
        self.load_current_children();
        self.push_nav_history();
    }

    /// Go up one directory level. If inside a subdirectory of the current tree,
    /// pops up within the tree. At tree root, changes view_root_path to the
    /// filesystem parent directory. Repeatable until reaching `/`.
    pub fn go_up_fs(&mut self) {
        if self.current_dir_path.len() > 1 {
            self.go_to_parent();
            return;
        }
        let parent = self.view_root_path.parent().map(|p| p.to_path_buf());
        if let Some(parent) = parent {
            self.view_root_path = parent;
            self.rebuild_tree();
            self.push_nav_history();
            self.set_info(
                format!("changed root to {}", self.view_root_path.display()),
                3,
            );
        }
    }

    /// Record current position in nav history (called after navigation).
    fn push_nav_history(&mut self) {
        let pos = NavPosition {
            current_dir_path: self.current_dir_path.clone(),
            view_root_path: self.view_root_path.clone(),
            cursor: self.cursor,
            scroll_offset: self.scroll_offset,
        };
        // Truncate any forward history beyond current position
        // Keep elements [0..=idx] (current position inclusive), remove rest
        self.nav_history.truncate(self.nav_history_idx + 1);
        self.nav_history.push(pos);
        self.nav_history_idx = self.nav_history.len() - 1;
    }

    /// Push the initial navigation state (called once after rebuild_tree).
    pub fn push_initial_nav_state(&mut self) {
        self.push_nav_history();
    }

    /// Go back in navigation history.
    pub fn nav_back(&mut self) {
        if self.nav_history_idx == 0 {
            return;
        }
        self.nav_history_idx -= 1;
        if let Some(pos) = self.nav_history.get(self.nav_history_idx) {
            let root_changed = pos.view_root_path != self.view_root_path;
            self.current_dir_path = pos.current_dir_path.clone();
            self.view_root_path = pos.view_root_path.clone();
            self.cursor = pos.cursor;
            self.scroll_offset = pos.scroll_offset;
            if root_changed {
                self.rebuild_tree();
            } else {
                self.load_current_children();
            }
        }
    }

    /// Go forward in navigation history.
    pub fn nav_forward(&mut self) {
        if self.nav_history_idx + 1 >= self.nav_history.len() {
            return;
        }
        self.nav_history_idx += 1;
        if let Some(pos) = self.nav_history.get(self.nav_history_idx) {
            let root_changed = pos.view_root_path != self.view_root_path;
            self.current_dir_path = pos.current_dir_path.clone();
            self.view_root_path = pos.view_root_path.clone();
            self.cursor = pos.cursor;
            self.scroll_offset = pos.scroll_offset;
            if root_changed {
                self.rebuild_tree();
            } else {
                self.load_current_children();
            }
        }
    }

    /// Compute search match indices for n/N navigation.
    /// Does not filter current_filtered — non-matching items remain visible.
    pub fn apply_search(&mut self) {
        if self.search_word.is_empty() {
            self.search_match_indices.clear();
            self.refresh_current_filtered();
            return;
        }
        let query = &self.search_word;
        self.search_match_indices = self
            .current_children
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                crate::search::fuzzy_match_indices(query, entry.node.name()).is_some()
            })
            .map(|(i, _)| i)
            .collect();

        self.refresh_current_filtered();

        // Jump cursor to first match if any
        if let Some(&first) = self.search_match_indices.first() {
            self.cursor = first;
        }
    }

    /// Cycle through search matches (n/N behavior).
    pub fn cycle_match(&mut self, forward: bool) {
        if self.search_match_indices.is_empty() {
            return;
        }
        // Find current cursor position in search_match_indices
        let pos = self
            .search_match_indices
            .iter()
            .position(|&i| i == self.cursor);
        let next = match (pos, forward) {
            (Some(p), true) => (p + 1) % self.search_match_indices.len(),
            (Some(p), false) => {
                (p + self.search_match_indices.len() - 1) % self.search_match_indices.len()
            }
            (None, true) => 0,
            (None, false) => self.search_match_indices.len() - 1,
        };
        self.cursor = self.search_match_indices[next];
    }

    // ── AI Review ────────────────────────────────────────────────────

    /// Enter AI review mode for a single path at cursor.
    pub fn enter_ai_review_single(&mut self) {
        let Some(full_path) = self.selected_node_full_path() else {
            return;
        };
        let paths = vec![full_path];
        let total = self.compute_pending_total_size(&paths);
        self.ai_state = Some(AiReviewState {
            results: Vec::new(),
            pending_paths: paths.clone(),
            pending_total_size: total,
            cursor: 0,
            scroll_offset: 0,
            mark_for_delete: HashSet::new(),
            status: AiStatus::Loading,
            delete_confirm: None,
            info_item: None,
        });
        self.mode = AppMode::AiReview;
        self.spawn_ai_analysis(paths);
    }

    /// Enter AI review mode for all multi-selected paths.
    pub fn enter_ai_review_multi(&mut self) {
        let mut paths = self.selected_paths_full();
        if paths.is_empty() {
            self.set_info("no items selected".into(), 3);
            return;
        }
        paths.sort();
        let total = self.compute_pending_total_size(&paths);
        self.ai_state = Some(AiReviewState {
            results: Vec::new(),
            pending_paths: paths.clone(),
            pending_total_size: total,
            cursor: 0,
            scroll_offset: 0,
            mark_for_delete: HashSet::new(),
            status: AiStatus::Loading,
            delete_confirm: None,
            info_item: None,
        });
        self.mode = AppMode::AiReview;
        self.spawn_ai_analysis(paths);
    }

    /// Compute total size of paths using scan cache, with metadata fallback.
    fn compute_pending_total_size(&self, paths: &[PathBuf]) -> u64 {
        let mut total = 0u64;
        for path in paths {
            if let Some(snapshot) = self.scan_cache.get(path) {
                total += snapshot.total_size;
            } else {
                let mut found = false;
                for (root, snapshot) in &self.scan_cache {
                    if let Ok(relative) = path.strip_prefix(root) {
                        let mut idx = argus_core::ROOT_NODE;
                        let mut ok = true;
                        for component in relative.components() {
                            let name = component.as_os_str().to_str().unwrap_or("");
                            if let Some(child) = snapshot.child_idx(idx, name) {
                                idx = child;
                            } else {
                                ok = false;
                                break;
                            }
                        }
                        if ok {
                            total += snapshot.node(idx).size();
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    if let Ok(meta) = std::fs::metadata(path) {
                        total += meta.len();
                    }
                }
            }
        }
        total
    }

    /// Get the recursive size of a path using scan cache, with metadata fallback.
    fn compute_path_size(&self, path: &Path) -> u64 {
        if let Some(snapshot) = self.scan_cache.get(path) {
            return snapshot.total_size;
        }
        for (root, snapshot) in &self.scan_cache {
            if let Ok(relative) = path.strip_prefix(root) {
                let mut idx = argus_core::ROOT_NODE;
                let mut ok = true;
                for component in relative.components() {
                    let name = component.as_os_str().to_str().unwrap_or("");
                    if let Some(child) = snapshot.child_idx(idx, name) {
                        idx = child;
                    } else {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    return snapshot.node(idx).size();
                }
            }
        }
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    /// Spawn a background thread to compute AI verdicts.
    fn spawn_ai_analysis(&self, paths: Vec<PathBuf>) {
        // Pre-compute correct sizes using scan cache (recursive for dirs)
        let path_sizes: HashMap<PathBuf, u64> = paths
            .iter()
            .map(|p| (p.clone(), self.compute_path_size(p)))
            .collect();

        let tx = self.tx.clone();
        let ai_config = self.config.ai.clone();
        let db_path = argus_core::default_db_path();
        let log_path = crate::util::default_log_path();

        std::thread::spawn(move || {
            let conn = argus_core::open_db(&db_path).ok();
            let use_ai =
                ai_config.enabled && !ai_config.api_url.is_empty() && !ai_config.api_key.is_empty();

            // First pass: check DB cache, prepare uncached paths
            let mut cached: Vec<AiPathVerdict> = Vec::new();
            let mut uncached: Vec<(PathBuf, u64, String)> = Vec::new();

            for path in &paths {
                let path_str = path.to_string_lossy();
                let mut found = false;

                if let Some(ref conn) = conn {
                    if let Ok(Some(data)) = argus_core::get_ai_analysis(conn, &path_str) {
                        if let Ok(mut verdict) = serde_json::from_slice::<AiPathVerdict>(&data) {
                            // Override cached size with scan-cache aggregated size (fixes
                            // directories that were cached with metadata.len() values)
                            if let Some(&scan_size) = path_sizes.get(path) {
                                verdict.size = scan_size;
                            }
                            cached.push(verdict);
                            found = true;
                        }
                    }
                }

                if !found {
                    let size = path_sizes.get(path).copied().unwrap_or(0);
                    let label = resolve_label(path);
                    uncached.push((path.to_path_buf(), size, label));
                }
            }

            if !uncached.is_empty() && use_ai {
                let contexts: Vec<argus_core::AiContext> = uncached
                    .iter()
                    .map(|(path, size, _)| argus_core::AiContext {
                        target_path: path.to_string_lossy().to_string(),
                        current_size_mb: *size as f64 / (1024.0 * 1024.0),
                    })
                    .collect();

                let prompt = argus_core::build_prompt(&contexts, &ai_config.language);
                crate::util::log_msg(&log_path, &format!("AI prompt:\n{}", prompt));

                match argus_core::analyze(&contexts, &ai_config.to_core_config()) {
                    Ok(ai_map) => {
                        crate::util::log_msg(
                            &log_path,
                            &format!(
                                "AI response ({} paths):\n{}",
                                ai_map.len(),
                                serde_json::to_string_pretty(&ai_map).unwrap_or_default()
                            ),
                        );

                        for (path, size, label) in &uncached {
                            let path_str = path.to_string_lossy();
                            let verdict = if let Some(resp) = ai_map.get(path_str.as_ref()) {
                                AiPathVerdict::from_response(
                                    path.to_path_buf(),
                                    *size,
                                    label.clone(),
                                    resp.clone(),
                                )
                            } else {
                                mock_ai_verdict(path, *size)
                            };

                            if let Some(ref conn) = conn {
                                if let Ok(data) = serde_json::to_vec(&verdict) {
                                    let _ = argus_core::set_ai_analysis(conn, &path_str, &data);
                                }
                            }
                            cached.push(verdict);
                        }
                    }
                    Err(e) => {
                        let msg = format!("AI error: {}", e);
                        crate::util::log_msg(&log_path, &msg);
                        let _ = tx.blocking_send(AppMessage::AiAnalysisError(msg));
                        return;
                    }
                }
            } else {
                // AI not available: use heuristic for uncached paths
                for (path, size, _label) in uncached {
                    let verdict = mock_ai_verdict(&path, size);
                    if let Some(ref conn) = conn {
                        let path_str = path.to_string_lossy();
                        if let Ok(data) = serde_json::to_vec(&verdict) {
                            let _ = argus_core::set_ai_analysis(conn, &path_str, &data);
                        }
                    }
                    cached.push(verdict);
                }
            }

            let _ = tx.blocking_send(AppMessage::AiAnalysisComplete(cached));
        });
    }

    /// Exit AI review mode and clear state.
    pub fn exit_ai_review(&mut self) {
        self.ai_state = None;
        self.mode = AppMode::Browsing;
    }

    // ── Cleanup / Uninstall ────────────────────────────────────────

    pub fn enter_cleanup(&mut self, mode: CleanupMode) {
        self.cleanup_state = Some(CleanupState {
            mode,
            scanning: true,
            scan_current_path: None,
            items: Vec::new(),
            total_bytes: 0,
            selected: HashSet::new(),
            cursor: 0,
            scroll_offset: 0,
            dry_run: false,
            confirm_pending: false,
            report: None,
            detail_pending: false,
            detail_items: None,
        });
        self.mode = AppMode::Cleanup;
        self.scanning = true;
        self.scan_spinner = 0;
        self.scan_spinner_tick = Instant::now();
        self.spawn_cleanup_scan(mode);
    }

    pub fn exit_cleanup(&mut self) {
        self.cleanup_state = None;
        self.mode = AppMode::Browsing;
        self.scanning = false;
    }

    fn spawn_cleanup_scan(&self, mode: CleanupMode) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let items = match mode {
                CleanupMode::Clean => {
                    let targets = argus_core::default_clean_targets();
                    match argus_core::dry_clean(&targets) {
                        Ok(plan) => plan.items,
                        Err(e) => {
                            let _ = tx.blocking_send(AppMessage::Error(format!("clean scan failed: {e}")));
                            return;
                        }
                    }
                }
                CleanupMode::Purge => {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                    let roots = vec![std::path::PathBuf::from(&home)];
                    match argus_core::find_artifacts(&roots) {
                        Ok(artifacts) => artifacts.into_iter().map(|a| argus_core::CleanItem {
                            path: a.path,
                            size: a.size,
                            risk: argus_core::RiskLevel::Safe,
                            target_id: format!("{:?}", a.kind),
                        }).collect(),
                        Err(e) => {
                            let _ = tx.blocking_send(AppMessage::Error(format!("purge scan failed: {e}")));
                            return;
                        }
                    }
                }
            };
            let total_bytes: u64 = items.iter().map(|i| i.size).sum();
            let _ = tx.blocking_send(AppMessage::CleanupScanComplete { items, total_bytes });
        });
    }

    pub fn enter_uninstall(&mut self) {
        self.uninstall_state = Some(UninstallState {
            apps: Vec::new(),
            filtered: Vec::new(),
            search_word: String::new(),
            filter_mode: false,
            sort_mode: 0,
            cursor: 0,
            phase: UninstallPhase::SelectApp,
            selected_app: None,
            leftovers: None,
            selected_leftovers: HashSet::new(),
            remove_leftovers: true,
            scanning: true,
            scan_current_path: None,
            confirm_pending: false,
            report: None,
        });
        self.mode = AppMode::Uninstall;
        self.scanning = true;
        self.scan_spinner = 0;
        self.scan_spinner_tick = Instant::now();
        self.spawn_app_list_scan();
    }

    pub fn exit_uninstall(&mut self) {
        self.uninstall_state = None;
        self.mode = AppMode::Browsing;
        self.scanning = false;
    }

    fn spawn_app_list_scan(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let (prog_tx, prog_rx) = std::sync::mpsc::channel::<String>();
            let tx2 = tx.clone();
            std::thread::spawn(move || {
                while let Ok(path) = prog_rx.recv() {
                    if tx2.blocking_send(AppMessage::UninstallScanProgress(path)).is_err() {
                        break;
                    }
                }
            });
            match argus_core::find_installed_apps(Some(prog_tx)) {
                Ok(apps) => {
                    let _ = tx.blocking_send(AppMessage::AppListReady(apps));
                }
                Err(e) => {
                    let _ = tx.blocking_send(AppMessage::Error(format!("app scan failed: {e}")));
                }
            }
        });
    }

    /// Toggle delete mark on the current AI review item.
    pub fn ai_review_toggle_mark(&mut self) {
        let Some(ref mut state) = self.ai_state else {
            return;
        };
        if state.mark_for_delete.contains(&state.cursor) {
            state.mark_for_delete.remove(&state.cursor);
        } else {
            state.mark_for_delete.insert(state.cursor);
        }
    }
}

/// Determine the program label for a path based on built-in heuristics.
/// Used by both mock_ai_verdict and the AI analysis path.
fn resolve_label(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let name_lower = name.to_lowercase();

    match name_lower.as_str() {
        "target" | "build" | "builds" | "dist" | "out" | "output" => labels::BUILD_ARTIFACTS,
        "node_modules" | "vendor" | "bower_components" => labels::PACKAGE_DEPENDENCIES,
        ".git" | ".svn" | ".hg" => labels::VCS_DATA,
        ".cache" | "cache" | "caches" => labels::APP_CACHE,
        "logs" | "log" => labels::LOG_FILES,
        ".terraform" | ".serverless" | ".next" | ".nuxt" => labels::FRAMEWORK_CACHE,
        "tmp" | "temp" | "temporary" | ".trash" | "$trash" | ".recycle" => labels::TEMP_FILES,
        "downloads" | ".download" => labels::DOWNLOADS,
        _ => {
            if name.starts_with('.') {
                labels::HIDDEN_CONFIG
            } else {
                labels::UNCATEGORIZED
            }
        }
    }
    .to_string()
}

/// Generate a mock AI verdict based on directory/file name heuristics.
/// Phase 1: no real AI call. Phase 2+: will call AI API.
/// Label is program-determined (built-in heuristic). label_detail is left empty
/// in Phase 1 mock; Phase 2+ AI will fill it.
fn mock_ai_verdict(path: &std::path::Path, size: u64) -> AiPathVerdict {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let name_lower = name.to_lowercase();

    let (label, purpose, risk_level, suggestion, deletable, background) = match name_lower.as_str() {
        "target" | "build" | "builds" | "dist" | "out" | "output" => (
            labels::BUILD_ARTIFACTS,
            "Compiled output from the build process. Contains object files, binaries, and intermediate build products.",
            RiskLevel::Safe,
            "Safe to delete. Will be recreated on next build. Deleting frees significant disk space.",
            true,
            "Rust (Cargo) or other compiled language build system. Produces binaries and intermediate artifacts.",
        ),
        "node_modules" | "vendor" | "bower_components" => (
            labels::PACKAGE_DEPENDENCIES,
            "Third-party dependencies downloaded by a package manager (npm, yarn, Composer, etc.).",
            RiskLevel::Safe,
            "Safe to delete. Can be restored with the package manager's install command.",
            true,
            "npm (Node.js) / yarn / Composer (PHP) package manager directory. Contains all external libraries used by the project.",
        ),
        ".git" | ".svn" | ".hg" => (
            labels::VCS_DATA,
            "Version control history and metadata. Contains all commits, branches, and revision history.",
            RiskLevel::High,
            "Do NOT delete unless you want to remove version history. May break git/svn operations.",
            false,
            "Git is a distributed version control system. Deleting .git erases all commit history.",
        ),
        ".cache" | "cache" | "caches" => (
            labels::APP_CACHE,
            "Cached data from various applications. Speeds up repeated operations.",
            RiskLevel::Safe,
            "Safe to delete. Caches will be regenerated as needed. Deleting may slow down first use.",
            true,
            "",
        ),
        "logs" | "log" => (
            labels::LOG_FILES,
            "Application or system log files recording events, errors, and debug information.",
            RiskLevel::Safe,
            "Safe to delete if you don't need historical logs. May help with debugging if kept.",
            true,
            "",
        ),
        ".terraform" | ".serverless" | ".next" | ".nuxt" => (
            labels::FRAMEWORK_CACHE,
            "Cache and build artifacts from infrastructure or frontend framework tooling.",
            RiskLevel::Safe,
            "Safe to delete. Will be regenerated on next deploy or build.",
            true,
            "Terraform: infrastructure-as-code tool. Serverless: serverless framework. Next.js/Nuxt: JavaScript frameworks.",
        ),
        "tmp" | "temp" | "temporary" | ".trash" | "$trash" | ".recycle" => (
            labels::TEMP_FILES,
            "Temporary files that should have been cleaned up by the creating application.",
            RiskLevel::Safe,
            "Safe to delete. These are temporary files not needed for normal operation.",
            true,
            "",
        ),
        "downloads" | ".download" => (
            labels::DOWNLOADS,
            "Downloaded files. May contain important documents or installers.",
            RiskLevel::Low,
            "Review contents before deleting. Safe to delete installers and temporary downloads.",
            true,
            "",
        ),
        _ => {
            if name.starts_with('.') {
                (
                    labels::HIDDEN_CONFIG,
                    "Application configuration or data directory. Used by various programs to store settings.",
                    RiskLevel::Medium,
                    "Check which application owns this directory before deleting. May lose app settings.",
                    true,
                    "",
                )
            } else {
                (
                    labels::UNCATEGORIZED,
                    "Unable to determine purpose automatically. May contain user data or application files.",
                    RiskLevel::Medium,
                    "Review contents manually before deciding to delete.",
                    true,
                    "",

                )
            }
        }
    };

    AiPathVerdict {
        path: path.to_path_buf(),
        size,
        label: label.to_string(),
        label_detail: String::new(),
        purpose: purpose.to_string(),
        risk_level,
        suggestion: suggestion.to_string(),
        background: background.to_string(),
        deletable,
    }
}

/// Sort a slice of DirEntry by the given mode.
/// For Delta sort, uses the delta_cache to get delta values.
pub(crate) fn sort_children(
    children: &mut [DirEntry],
    mode: SortMode,
    delta_cache: &HashMap<Vec<String>, i64>,
) {
    match mode {
        SortMode::Name => {
            children.sort_by(|a, b| a.node.name().cmp(b.node.name()));
        }
        SortMode::Size => {
            children.sort_by(|a, b| {
                b.disk_usage
                    .cmp(&a.disk_usage)
                    .then_with(|| a.node.name().cmp(b.node.name()))
            });
        }
        SortMode::Delta => {
            children.sort_by(|a, b| {
                let a_delta = delta_cache.get(&a.path).copied().unwrap_or(0).abs();
                let b_delta = delta_cache.get(&b.path).copied().unwrap_or(0).abs();
                b_delta
                    .cmp(&a_delta)
                    .then_with(|| a.node.name().cmp(b.node.name()))
            });
        }
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
