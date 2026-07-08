pub mod ai_feature;
pub mod diff;
pub mod model;
pub mod scanner;

pub use ai_feature::{extract_feature, generate_prompt};
pub use diff::{compare_trees, filter_by_threshold, has_significant_changes};
pub use model::{
    hash_root_path, parse_human_size, AiCache, AiContext, AiResult, DiffError, DiffNode, FileNode,
    FileType, ParseSizeError, RiskLevel, ScanError, Snapshot, SnapshotError, SNAPSHOT_VERSION,
};
pub use scanner::scan_path;
