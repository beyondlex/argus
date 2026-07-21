use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use argus_core::{FileType, NodeIndex, Snapshot};
use serde::{Deserialize, Serialize};

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
    AiAnalysisComplete(Vec<AiPathVerdict>),
    AiAnalysisError(String),
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
    AiReview,
    QuitConfirm,
    MultiSelectExitConfirm,
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
/// Holds only a snapshot index (+ cached display fields); name comes from the snapshot blob.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub node: TreeNode,
    pub path: Vec<String>,
    pub has_scan_data: bool,
    pub has_ai: bool,
    pub ai_risk_level: Option<RiskLevel>,
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
    pub fn index(&self) -> NodeIndex {
        match self {
            TreeNode::Snapshot(_, idx) => *idx,
        }
    }

    pub fn snapshot(&self) -> &Snapshot {
        match self {
            TreeNode::Snapshot(snap, _) => snap.as_ref(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            TreeNode::Snapshot(snap, idx) => snap.name(*idx),
        }
    }

    pub fn is_dir(&self) -> bool {
        match self {
            TreeNode::Snapshot(snap, idx) => snap.node(*idx).is_dir(),
        }
    }

    pub fn file_type(&self) -> FileType {
        match self {
            TreeNode::Snapshot(snap, idx) => snap.node(*idx).file_type(),
        }
    }

    pub fn current_size(&self) -> u64 {
        match self {
            TreeNode::Snapshot(snap, idx) => snap.node(*idx).size(),
        }
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

// ── AI Review ─────────────────────────────────────────────────────────────────

/// Status of an AI analysis request
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AiStatus {
    Idle,
    Loading,
    Ready,
    Error(String),
}

/// Risk level for a path verdict
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RiskLevel {
    Safe,
    Low,
    Medium,
    High,
}

impl RiskLevel {
    pub fn label(self) -> &'static str {
        match self {
            RiskLevel::Safe => "Safe",
            RiskLevel::Low => "Low Risk",
            RiskLevel::Medium => "Medium Risk",
            RiskLevel::High => "High Risk",
        }
    }
}

/// AI verdict for a single path
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPathVerdict {
    pub path: PathBuf,
    pub size: u64,
    /// Program-determined label (built-in heuristic + user config mapping).
    /// Stable, predictable, usable for grouping/sorting/filtering.
    pub label: String,
    /// AI-determined specific source entity name (e.g. "Docker Desktop Buildx cache").
    /// Free-form, display-only, supplements the program label.
    pub label_detail: String,
    pub purpose: String,
    pub risk_level: RiskLevel,
    pub suggestion: String,
    /// Background knowledge explaining what the software/tool is
    /// (e.g. "Biome is a code formatter and linter written in Rust.")
    pub background: String,
    pub deletable: bool,
}

impl AiPathVerdict {
    /// Build a verdict from a program label and an AI response.
    /// Maps the AI's risk_level string to the local RiskLevel enum.
    pub fn from_response(
        path: PathBuf,
        size: u64,
        label: String,
        response: argus_core::AiResponse,
    ) -> Self {
        let risk_level = match response.risk_level.to_lowercase().as_str() {
            "safe" => RiskLevel::Safe,
            "low" => RiskLevel::Low,
            "medium" => RiskLevel::Medium,
            "high" => RiskLevel::High,
            _ => RiskLevel::Medium,
        };

        Self {
            path,
            size,
            label,
            label_detail: response.label_detail,
            purpose: response.description,
            risk_level,
            suggestion: response.suggestion,
            background: response.background,
            deletable: response.deletable,
        }
    }
}

/// State for the AI review popup
#[derive(Debug, Clone)]
pub struct AiReviewState {
    pub results: Vec<AiPathVerdict>,
    pub pending_paths: Vec<PathBuf>,
    pub pending_total_size: u64,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub mark_for_delete: HashSet<usize>,
    pub status: AiStatus,
    pub delete_confirm: Option<(Vec<PathBuf>, bool)>,
    pub info_item: Option<usize>,
}
