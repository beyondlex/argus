pub mod ai;
pub mod bloom;
pub mod db;
pub mod ipc;
pub mod model;
pub mod scanner;

#[cfg(feature = "cleanup")]
pub mod cleaner;

pub use ai::{
    build_prompt, estimate_tokens, try_parse_json, AiConfig, AiContext, AiError, AiLanguage,
    AiResponse,
};

#[cfg(feature = "ai")]
pub use ai::{analyze, call_ai_api};

#[cfg(feature = "cleanup")]
pub use cleaner::{
    audit::{log_operation, read_audit_log, AuditEntry, AuditOp},
    brew::{
        brew_cache_size, brew_dependents_of, brew_prefix, compare_oldest_first,
        format_last_used_relative, is_brew_available, keg_path, list_brew_packages,
        sort_oldest_first, uninstall_brew_package, BrewFilterType, BrewPackage, BrewPackageType,
        BrewSortMode,
    },
    categories::{default_clean_targets, CleanTarget, TargetCategory},
    cleaner::{dry_clean, exec_clean, plan_clean, CleanItem, CleanPlan, CleanReport},
    dir_size,
    purge::{find_artifacts, remove_artifacts, Artifact, ArtifactKind},
    safety::{check_deletion_allowed, classify_risk, is_protected, RiskLevel},
    uninstaller::{
        find_installed_apps, find_leftovers, find_orphaned_data, uninstall_app,
        uninstall_app_with_leftovers, AppInfo, AppLeftovers, OrphanedData,
    },
};

#[cfg(feature = "shell-cmds")]
pub use cleaner::shell_cmd::{
    default_shell_cmd_targets, try_exec_shell_cmd, ShellCmdResult, ShellCmdTarget,
};

pub use db::{
    clear_all_events, config_dir, consolidate_events, default_db_path, delete_ai_analysis,
    get_ai_analysis, init_db, insert_events, load_ai_cache_entries, open_db, purge_events_before,
    query_db_size, query_delta_detail, query_delta_summary, query_delta_total, query_event_count,
    set_ai_analysis, DbError,
};
pub use ipc::{DaemonRequest, DaemonResponse, WatchDirInfo, DEFAULT_UDS_PATH};
pub use model::{
    hash_root_path, labels, parse_human_size, DeltaEntry, DeltaEvent, DeltaSummary, FileNode,
    FileType, Label, NodeIndex, ParseSizeError, ScanError, Snapshot, SnapshotBuilder,
    SnapshotError, INLINE_NAME_MAX, NO_PARENT, ROOT_NODE, SNAPSHOT_VERSION,
};
pub use scanner::{list_dir, scan_path};
