use ratatui_finder::FinderState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;

use argus_core::{NodeIndex, Snapshot, ROOT_NODE};

use crate::ipc_client::IpcClient;
use crate::theme::ColorTheme;
use crate::time_utils::*;
use crate::tree_ops;
pub use crate::types::*;
use crate::util::{default_log_path, log_msg};

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

    // Delta detail popup
    pub delta_detail: Option<DeltaDetailState>,

    // Command bar
    pub command_input: String,
    pub command_matches: Vec<&'static str>,
    pub command_selected: usize,
    pub command_history: Vec<String>,
    pub command_history_idx: Option<usize>,

    // Delete tracking
    pub deleted_bytes: u64,

    // Time help popup
    pub time_help_scroll: usize,
    pub should_quit: bool,

    // Finder (Go to Path)
    pub finder_state: Option<FinderState>,

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

    /// Total size of the current directory (for percentage calculation)
    pub current_dir_total: u64,

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
            multi_select: false,
            selected_paths: HashSet::new(),
            deleted_bytes: 0,
            info_data: None,
            delta_detail: None,
            command_input: String::new(),
            command_matches: Vec::new(),
            command_selected: 0,
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
            show_hidden: false,
            current_children: Vec::new(),
            current_filtered: Vec::new(),
            current_dir_path: Vec::new(),
            dir_stack: Vec::new(),
            current_dir_total: 0,
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

    /// Toggle selection of the item at the current cursor position
    pub fn toggle_selection(&mut self) {
        if let Some(entry) = self.selected_entry() {
            let path = entry.path.clone();
            if self.selected_paths.contains(&path) {
                self.selected_paths.remove(&path);
            } else {
                self.selected_paths.insert(path);
            }
        }
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

    // ── Flat mode methods ───────────────────────────────────────────────

    /// Load children of the current directory from the snapshot into `current_children`.
    /// Replaces (does not append). Also updates `current_filtered`, `current_dir_total`,
    /// and `parent_dir_total`.
    pub fn load_current_children(&mut self) {
        // Search matches are stale after children reload
        self.search_match_indices.clear();

        // Check if listing is needed before taking any borrow on tree_root
        let needs_listing = self.current_dir_path.len() > 1
            && self.tree_root.as_ref().is_some_and(|tr| {
                let TreeNode::Snapshot(snap, idx) = tr;
                snap.find_node(*idx, &self.current_dir_path)
                    .is_some_and(|fidx| {
                        let n = snap.node(fidx);
                        n.is_dir && n.children.is_empty()
                    })
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
                    let child_nodes: Vec<(String, argus_core::FileNode)> = listed
                        .node(ROOT_NODE)
                        .children
                        .iter()
                        .map(|(name, idx)| (name.clone(), listed.node(*idx).clone()))
                        .collect();
                    for (name, node) in child_nodes {
                        let new_idx = snap_mut.arena.len() as NodeIndex;
                        snap_mut.arena.push(node);
                        snap_mut.node_mut(target_idx).children.push((name, new_idx));
                    }
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
            let root_name = snap.node(*root_idx).name.clone();
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

        // (2) Resolve scan tree for has_scan_data lookups
        let root_scan_tree = tree_ops::resolve_scan_tree(&self.scan_cache, &self.view_root_path);

        // (3) Collect children
        let mut children: Vec<DirEntry> = Vec::with_capacity(dir_node.children.len());

        for (name, child_idx) in &dir_node.children {
            // Skip hidden files if show_hidden is false
            if !self.show_hidden && name.starts_with('.') {
                continue;
            }

            let child_node = snap.node(*child_idx);
            let mut child_path = self.current_dir_path.clone();
            child_path.push(name.clone());

            let has_scan = if child_node.is_dir {
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
                node: TreeNode::Snapshot(snap_arc.clone(), *child_idx),
                path: child_path,
                has_scan_data: has_scan,
                is_dir: child_node.is_dir,
                size: child_node.size,
            });
        }

        // (4) Sort children
        sort_children(&mut children, self.sort_mode, &self.delta_cache);

        // (5) Update totals
        self.current_children = children;
        self.current_dir_total = dir_node.size;
        self.parent_dir_total = if self.current_dir_path.len() <= 1 {
            dir_node.size
        } else {
            let parent_path = &self.current_dir_path[..self.current_dir_path.len() - 1];
            match snap.find_node(*root_idx, parent_path) {
                Some(pidx) => snap.node(pidx).size,
                None => dir_node.size,
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
    }

    /// Go to root directory (flat mode).
    /// Clears the navigation stack and reloads children at the root level.
    pub fn go_to_root(&mut self) {
        self.dir_stack.clear();
        self.current_dir_path.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
        self.load_current_children();
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
            self.set_info(
                format!("changed root to {}", self.view_root_path.display()),
                3,
            );
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
                b.size
                    .cmp(&a.size)
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
    fn test_unlisted_child_dir_keeps_dash() {
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
        app.scan_cache
            .insert(root_path.clone(), Arc::new(scan_snap));

        app.current_dir_path = vec!["test".to_string(), "target".to_string()];
        app.load_current_children();

        let build_entry = app
            .current_children
            .iter()
            .find(|entry| entry.node.name() == "build")
            .expect("build entry should exist");
        assert!(build_entry.is_dir);
        assert!(!build_entry.has_scan_data);
        assert_eq!(build_entry.size, 0);
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

    // ── delta filter tests ──────────────────────────────────────────────────

    #[test]
    fn test_delta_unit_multiplier_values() {
        assert_eq!(delta_unit_multiplier(0), 1024);
        assert_eq!(delta_unit_multiplier(1), 1024 * 1024);
        assert_eq!(delta_unit_multiplier(2), 1024 * 1024 * 1024);
        assert_eq!(delta_unit_multiplier(3), 1);
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
    fn test_delta_filter_inc_unit_level_up_stays_at_gb() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.delta_filter_value = 1024;
        app.delta_filter_unit = 2;
        app.delta_filter_inc();
        assert_eq!(app.delta_filter_value, 1025);
        assert_eq!(app.delta_filter_unit, 2);
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
        assert!(app.time_to - app.time_from <= 3_600_000);
    }

    #[test]
    fn test_set_time_preset_7d() {
        let (tx, _) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
        app.set_time_preset(5);
        assert_eq!(app.time_preset, 5);
        let diff = app.time_to - app.time_from;
        assert!(diff >= 604_800_000 - 1000);
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
        assert_eq!(root.file_type(), FileType::Directory);
        assert_eq!(file.current_size(), 1024);
    }

    // ── selected_node_full_path tests ─────────────────────────────────────────

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

    // ── Flat mode tests ──────────────────────────────────────────────────

    fn make_flat_app() -> App {
        use argus_core::{FileNode, FileType, Snapshot, ROOT_NODE};
        let arena = vec![
            FileNode {
                name: "test".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 200,
                children: vec![
                    ("src".into(), 1),
                    ("docs".into(), 2),
                    ("readme.md".into(), 3),
                ],
            },
            FileNode {
                name: "src".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 100,
                children: vec![],
            },
            FileNode {
                name: "docs".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 50,
                children: vec![],
            },
            FileNode {
                name: "readme.md".into(),
                parent: None,
                is_dir: false,
                file_type: FileType::File,
                size: 50,
                children: vec![],
            },
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 200);
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/test");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.current_dir_path = vec![String::from("test")];
        app.load_current_children();
        app
    }

    #[test]
    fn test_load_current_children_basic() {
        let app = make_flat_app();
        assert_eq!(app.current_children.len(), 3);
        assert_eq!(app.current_dir_total, 200);
        assert_eq!(app.current_children[0].node.name(), "src");
        assert_eq!(app.current_children[1].node.name(), "docs");
        assert_eq!(app.current_children[2].node.name(), "readme.md");
    }

    #[test]
    fn test_selected_entry_returns_correct_item() {
        let mut app = make_flat_app();
        app.cursor = 0;
        let entry = app.selected_entry().unwrap();
        assert_eq!(entry.node.name(), "src");
        assert!(entry.is_dir);

        app.cursor = 2;
        let entry = app.selected_entry().unwrap();
        assert_eq!(entry.node.name(), "readme.md");
        assert!(!entry.is_dir);
    }

    #[test]
    fn test_selected_entry_out_of_bounds() {
        let mut app = make_flat_app();
        app.cursor = 100;
        assert!(app.selected_entry().is_none());
    }

    #[test]
    fn test_enter_directory_into_subdir() {
        let mut app = make_flat_app();
        app.cursor = 0;

        app.enter_directory();
        assert_eq!(
            app.current_dir_path,
            vec![String::from("test"), String::from("src")]
        );
        assert_eq!(app.current_children.len(), 0);
        assert_eq!(app.dir_stack.len(), 1);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_enter_directory_non_dir_does_nothing() {
        let mut app = make_flat_app();
        let readme_idx = app
            .current_children
            .iter()
            .position(|e| e.node.name() == "readme.md")
            .unwrap();
        app.cursor = readme_idx;

        app.enter_directory();
        assert_eq!(app.current_dir_path, vec![String::from("test")]);
        assert!(app.dir_stack.is_empty());
    }

    #[test]
    fn test_go_to_parent_restores_previous() {
        let mut app = make_flat_app();
        app.cursor = 0;
        app.enter_directory();
        assert_eq!(
            app.current_dir_path,
            vec![String::from("test"), String::from("src")]
        );

        app.go_to_parent();
        assert_eq!(app.current_dir_path, vec![String::from("test")]);
        assert!(app.dir_stack.is_empty());
    }

    #[test]
    fn test_go_to_parent_at_root_does_nothing() {
        let mut app = make_flat_app();
        assert_eq!(app.current_dir_path, vec![String::from("test")]);

        app.go_to_parent();
        assert_eq!(app.current_dir_path, vec![String::from("test")]);
    }

    #[test]
    fn test_go_to_root_clears_stack() {
        let mut app = make_flat_app();
        app.cursor = 0;
        app.enter_directory();
        assert_eq!(app.dir_stack.len(), 1);

        app.go_to_root();
        assert_eq!(app.current_dir_path, vec![String::from("test")]);
        assert!(app.dir_stack.is_empty());
    }

    #[test]
    fn test_apply_search_filters_children() {
        let mut app = make_flat_app();
        app.search_word = "src".into();
        app.apply_search();
        // current_filtered is NOT filtered by search — all items remain visible
        assert_eq!(app.current_filtered.len(), 3);
        // search_match_indices contains the matching index
        assert_eq!(app.search_match_indices.len(), 1);
        assert_eq!(app.search_match_indices[0], 0);
        assert_eq!(app.current_children[0].node.name(), "src");
    }

    #[test]
    fn test_apply_search_empty_query_restores_all() {
        let mut app = make_flat_app();
        app.search_word = "nonexistent".into();
        app.apply_search();
        // current_filtered still has all items (not filtered by search)
        assert_eq!(app.current_filtered.len(), 3);
        // search_match_indices is empty for no matches
        assert!(app.search_match_indices.is_empty());

        app.search_word = "".into();
        app.apply_search();
        assert_eq!(app.current_filtered.len(), 3);
        assert!(app.search_match_indices.is_empty());
    }

    #[test]
    fn test_cycle_match_forward() {
        let mut app = make_flat_app();
        app.search_word = "readme".into();
        app.apply_search();
        assert_eq!(app.search_match_indices.len(), 1);

        app.cursor = 0;
        app.cycle_match(true);
        assert_eq!(app.cursor, 2); // cycles to readme.md index
    }

    #[test]
    fn test_cycle_match_backward() {
        let mut app = make_flat_app();
        app.search_word = "readme".into();
        app.apply_search();
        assert_eq!(app.search_match_indices.len(), 1);

        app.cursor = 0;
        app.cycle_match(false);
        assert_eq!(app.cursor, 2); // cycles backward to readme.md index
    }

    #[test]
    fn test_sort_by_name() {
        let mut app = make_flat_app();
        app.sort_mode = crate::types::SortMode::Name;
        crate::app::sort_children(&mut app.current_children, app.sort_mode, &app.delta_cache);
        app.refresh_current_filtered();
        assert_eq!(app.current_children[0].node.name(), "docs");
        assert_eq!(app.current_children[1].node.name(), "readme.md");
        assert_eq!(app.current_children[2].node.name(), "src");
    }

    #[test]
    fn test_sort_by_size() {
        let mut app = make_flat_app();
        app.sort_mode = crate::types::SortMode::Size;
        crate::app::sort_children(&mut app.current_children, app.sort_mode, &app.delta_cache);
        app.refresh_current_filtered();
        assert_eq!(app.current_children[0].node.name(), "src");
        assert_eq!(app.current_children[1].node.name(), "docs");
        assert_eq!(app.current_children[2].node.name(), "readme.md");
    }

    #[test]
    #[allow(unused_mut)]
    fn test_hidden_files_toggle_in_load() {
        use argus_core::{FileNode, FileType, Snapshot, ROOT_NODE};
        let arena = vec![
            FileNode {
                name: "test".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 100,
                children: vec![(".hidden".into(), 1), ("visible.txt".into(), 2)],
            },
            FileNode {
                name: ".hidden".into(),
                parent: None,
                is_dir: false,
                file_type: FileType::File,
                size: 50,
                children: vec![],
            },
            FileNode {
                name: "visible.txt".into(),
                parent: None,
                is_dir: false,
                file_type: FileType::File,
                size: 50,
                children: vec![],
            },
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 100);
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/test");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.current_dir_path = vec![String::from("test")];

        app.show_hidden = false;
        app.load_current_children();
        assert_eq!(app.current_children.len(), 1);
        assert_eq!(app.current_children[0].node.name(), "visible.txt");

        app.show_hidden = true;
        app.load_current_children();
        assert_eq!(app.current_children.len(), 2);
    }

    #[test]
    fn test_dir_stack_depth_multiple_entries() {
        use argus_core::{FileNode, FileType, Snapshot, ROOT_NODE};
        let arena = vec![
            FileNode {
                name: "root".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 300,
                children: vec![("a".into(), 1), ("b".into(), 2)],
            },
            FileNode {
                name: "a".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 200,
                children: vec![("deep".into(), 3)],
            },
            FileNode {
                name: "b".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 100,
                children: vec![],
            },
            FileNode {
                name: "deep".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 50,
                children: vec![],
            },
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/deep"), arena, 300);
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/deep");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.current_dir_path = vec!["root".into()];
        app.load_current_children();

        app.cursor = 0;
        app.enter_directory();
        assert_eq!(
            app.current_dir_path,
            vec![String::from("root"), String::from("a")]
        );

        app.cursor = 0;
        app.enter_directory();
        assert_eq!(
            app.current_dir_path,
            vec![
                String::from("root"),
                String::from("a"),
                String::from("deep")
            ]
        );
        assert_eq!(app.dir_stack.len(), 2);

        app.go_to_parent();
        assert_eq!(
            app.current_dir_path,
            vec![String::from("root"), String::from("a")]
        );
        assert_eq!(app.dir_stack.len(), 1);

        app.go_to_parent();
        assert_eq!(app.current_dir_path, vec![String::from("root")]);
        assert!(app.dir_stack.is_empty());
    }

    #[test]
    fn test_selected_node_full_path_flat_mode() {
        let mut app = make_flat_app();
        let idx = app
            .current_children
            .iter()
            .position(|e| e.node.name() == "readme.md")
            .unwrap();
        app.cursor = app
            .current_filtered
            .iter()
            .position(|&i| i == idx)
            .unwrap_or(0);
        let path = app.selected_node_full_path();
        assert_eq!(path, Some(PathBuf::from("/tmp/test/readme.md")));
    }

    #[test]
    fn test_refresh_current_filtered_delta_active() {
        let mut app = make_flat_app();
        app.delta_filter_active = true;
        app.delta_filter_value = 80;
        app.delta_filter_unit = 0;
        app.delta_cache = std::collections::HashMap::from([
            (vec![String::from("test"), String::from("src")], 100_000i64),
            (vec![String::from("test"), String::from("docs")], 50_000i64),
            (vec![String::from("test"), String::from("readme.md")], 0i64),
        ]);
        app.refresh_current_filtered();
        assert_eq!(app.current_filtered.len(), 1);
        let idx = app.current_filtered[0];
        assert_eq!(app.current_children[idx].node.name(), "src");
    }
}
