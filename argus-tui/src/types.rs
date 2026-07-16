use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use argus_core::{FileNode, FileType, NodeIndex, Snapshot};

// ── Messages ─────────────────────────────────────────────────────────────────

/// Messages from background tasks to the UI
#[derive(Debug)]
pub enum AppMessage {
    ScanProgress {
        file_count: u64,
        total_bytes: u64,
        total_disk_bytes: u64,
        current_path: Option<String>,
    },
    ScanComplete(Snapshot),
    DaemonConnected(IpcClient),
    DaemonDisconnected,
    DeltaData(HashMap<Vec<String>, i64>, Option<IpcClient>),
    DeltaDetailLoaded(DeltaDetailState),
    DeleteProgress {
        current: u64,
        total: u64,
    },
    DeleteComplete {
        errors: Vec<String>,
        paths: Vec<PathBuf>,
    },
    Error(String),
    Info(String),
}

// Re-export IpcClient type for AppMessage without circular dependency
use crate::ipc_client::IpcClient;

// ── Modes ────────────────────────────────────────────────────────────────────

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppMode {
    Browsing,
    DeletePrompt,
    DeletePermanentPrompt,
    Deleting,
    Help,
    TimeHelp,
    Command,
    Finder, // Finder mode (Go to Path)
}

/// Tree search mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchMode {
    Inactive,
    Input,
    Active,
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

// ── Data types ───────────────────────────────────────────────────────────────

/// A single entry in the flat directory view (ncdu-like).
/// Represents one direct child of the current directory.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub node: TreeNode,
    pub path: Vec<String>,
    pub has_scan_data: bool,
    pub is_dir: bool,
    pub size: u64,
    pub disk_usage: u64,
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

#[derive(Debug, Clone)]
pub struct ScanSummary {
    pub root_path: PathBuf,
    pub total_size: u64,
    pub total_disk_usage: u64,
    pub total_files: u64,
    pub duration: Duration,
}

// ── Delta Detail Popup ───────────────────────────────────────────────────────

/// State for the delta event detail popup (opened with `K`)
#[derive(Debug, Clone)]
pub struct DeltaDetailState {
    pub path: PathBuf,
    pub entries: Vec<DeltaDetailRow>,
    pub scroll: usize,
}

/// A single row in the delta detail popup
#[derive(Debug, Clone)]
pub struct DeltaDetailRow {
    pub timestamp: String,  // formatted as "2026-07-13 HH:MM:SS"
    pub child_name: String, // direct child name (e.g. "argus", "README.md")
    pub delta_size: i64,
    pub delta_display: String, // "+ 100 MB", "-  10 KB"
    pub is_aggregated: bool,   // false=raw event, true=synthetic sum of descendants
}

// ── Constants ────────────────────────────────────────────────────────────────

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
