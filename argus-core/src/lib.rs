pub mod bloom;
pub mod db;
pub mod ipc;
pub mod model;
pub mod scanner;

pub use db::{
    clear_all_events, consolidate_events, default_db_path, delete_ai_analysis, get_ai_analysis,
    has_ai_analysis, has_ai_analysis_batch, init_db, insert_events, load_all_ai_analyzed_paths,
    open_db, purge_events_before, query_db_size, query_delta_detail, query_delta_summary,
    query_delta_total, query_event_count, set_ai_analysis, DbError,
};
pub use ipc::{DaemonRequest, DaemonResponse, WatchDirInfo, DEFAULT_UDS_PATH};
pub use model::{
    hash_root_path, parse_human_size, DeltaEntry, DeltaEvent, DeltaSummary, FileNode, FileType,
    NodeIndex, ParseSizeError, ScanError, Snapshot, SnapshotBuilder, SnapshotError,
    INLINE_NAME_MAX, NO_PARENT, ROOT_NODE, SNAPSHOT_VERSION,
};
pub use scanner::{list_dir, scan_path};
