use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use argus_core::{FileNode, FileType, Snapshot};

/// Helper: create a mock FileNode for testing
fn make_file(name: &str, size: u64) -> FileNode {
    FileNode {
        name: name.to_string(),
        is_dir: false,
        file_type: FileType::File,
        size,
        modified: None,
        created: None,
        inode: None,
        device: None,
        has_metadata: true,
        children: HashMap::new(),
    }
}

fn make_dir(name: &str, children: Vec<FileNode>) -> FileNode {
    let mut map = HashMap::new();
    for child in children {
        map.insert(child.name.clone(), child);
    }
    let total: u64 = map.values().map(|c| c.size).sum();
    FileNode {
        name: name.to_string(),
        is_dir: true,
        file_type: FileType::Directory,
        size: total,
        modified: None,
        created: None,
        inode: None,
        device: None,
        has_metadata: true,
        children: map,
    }
}

fn make_snapshot(path: &str, root: FileNode) -> Snapshot {
    let size = root.size;
    Snapshot::new(PathBuf::from(path), root, size)
}

// ── State logic tests ───────────────────────────────────────────────────────

#[test]
fn test_scan_cancelled() {
    let dir = std::env::temp_dir().join("argus_test_cancel");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("test.txt"), "data").unwrap();

    let cancel = AtomicBool::new(true);
    let result = argus_core::scan_path(&dir, &cancel, None, &[]);
    assert!(result.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_snapshot_serialization_roundtrip() {
    let root = make_dir(
        "test",
        vec![make_file("a.txt", 100), make_file("b.txt", 200)],
    );
    let snap = make_snapshot("/tmp/test", root);

    let json = serde_json::to_string_pretty(&snap).unwrap();
    let deserialized: Snapshot = serde_json::from_str(&json).unwrap();

    assert_eq!(snap.root_path, deserialized.root_path);
    assert_eq!(snap.total_size, deserialized.total_size);
    assert_eq!(
        snap.root_node.children.len(),
        deserialized.root_node.children.len()
    );
}

#[test]
fn test_diff_with_threshold() {
    let old_root = make_dir(
        "test",
        vec![
            make_file("small", 10),
            make_file("medium", 100),
            make_file("large", 1000),
        ],
    );
    let new_root = make_dir(
        "test",
        vec![
            make_file("small", 20),
            make_file("medium", 100),
            make_file("large", 1000),
        ],
    );

    let old_snap = make_snapshot("/test", old_root);
    let new_snap = make_snapshot("/test", new_root);

    let diff = argus_core::compare_trees(&old_snap, &new_snap).unwrap();
    let filtered = argus_core::filter_by_threshold(&diff, 50);

    // All changes are below threshold, so result should be None
    assert!(filtered.is_none());
}

#[test]
fn test_compare_trees_added_removed() {
    let old_root = make_dir("test", vec![make_file("old.txt", 100)]);
    let new_root = make_dir("test", vec![make_file("new.txt", 200)]);

    let old_snap = make_snapshot("/test", old_root);
    let new_snap = make_snapshot("/test", new_root);

    let diff = argus_core::compare_trees(&old_snap, &new_snap).unwrap();

    // old.txt removed: delta = -100, current = 0
    let old_node = diff.children.get("old.txt").unwrap();
    assert_eq!(old_node.size_delta, -100);
    assert_eq!(old_node.current_size, 0);

    // new.txt added: delta = 200, current = 200
    let new_node = diff.children.get("new.txt").unwrap();
    assert_eq!(new_node.size_delta, 200);
    assert_eq!(new_node.current_size, 200);
}

// ── TUI-specific state logic tests ──────────────────────────────────────────

#[test]
fn test_snapshot_info_construction() {
    use argus_tui::app::SnapshotInfo;
    use chrono::Utc;

    let now = Utc::now();
    let info = SnapshotInfo::from_scan_timestamp_info(1, now, 1024, 5);
    assert_eq!(info.scan_id, 1);
    assert_eq!(info.timestamp, now);
    assert_eq!(info.total_size, 1024);
    assert_eq!(info.total_files, 5);
}

#[test]
fn test_available_snapshots_sorting() {
    use argus_tui::app::SnapshotInfo;
    use chrono::{TimeZone, Utc};

    let ts1 = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
    let ts2 = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();
    let ts3 = Utc.with_ymd_and_hms(2026, 6, 15, 0, 0, 0).unwrap();

    let mut snapshots = vec![
        SnapshotInfo::from_scan_timestamp_info(3, ts3, 300, 3),
        SnapshotInfo::from_scan_timestamp_info(1, ts1, 100, 1),
        SnapshotInfo::from_scan_timestamp_info(2, ts2, 200, 2),
    ];

    snapshots.sort_by_key(|s| s.timestamp);
    assert_eq!(snapshots[0].scan_id, 1); // June 1
    assert_eq!(snapshots[1].scan_id, 3); // June 15
    assert_eq!(snapshots[2].scan_id, 2); // July 1
}

#[test]
fn test_filter_state_from_empty_to_set() {
    use argus_tui::app::FilterState;

    let mut filter = FilterState {
        from_idx: None,
        to_idx: None,
        threshold: None,
        dirty: false,
        sub_focus: argus_tui::app::FilterFocus::From,
    };
    assert!(filter.from_idx.is_none() && filter.to_idx.is_none());

    filter.from_idx = Some(0);
    filter.to_idx = Some(1);
    assert!(filter.from_idx.is_some() && filter.to_idx.is_some());
    assert!(filter.should_diff());
}

// ── Size formatting tests ───────────────────────────────────────────────────

#[test]
fn test_format_size() {
    assert_eq!(argus_tui::util::format_size(500), "500 B");
    assert_eq!(argus_tui::util::format_size(1024), "1.00 KB");
    assert_eq!(argus_tui::util::format_size(1_048_576), "1.00 MB");
    assert_eq!(argus_tui::util::format_size(1_073_741_824), "1.00 GB");
}

#[test]
fn test_format_delta() {
    assert_eq!(argus_tui::util::format_delta(100), "+100 B");
    assert_eq!(argus_tui::util::format_delta(-100), "-100 B");
    assert_eq!(argus_tui::util::format_delta(0), "+0 B");
}

// ── Protected path tests ────────────────────────────────────────────────────

#[test]
fn test_is_protected_path() {
    // These should be protected on any Unix system.
    // Use canonical paths to handle macOS /private symlinks.
    assert!(argus_tui::util::is_protected_path(std::path::Path::new(
        "/usr/bin"
    )));
    assert!(argus_tui::util::is_protected_path(std::path::Path::new(
        "/usr/lib"
    )));

    // Subpaths under protected dirs should also be protected
    assert!(argus_tui::util::is_protected_path(std::path::Path::new(
        "/usr/bin/ls"
    )));

    // Regular user paths should not be protected
    assert!(!argus_tui::util::is_protected_path(std::path::Path::new(
        "/home/user"
    )));
    assert!(!argus_tui::util::is_protected_path(std::path::Path::new(
        "/tmp"
    )));
}

// ── Config loading tests ────────────────────────────────────────────────────

#[test]
fn test_config_default_values() {
    let config = argus_tui::config::TuiConfig::default();
    assert_eq!(config.keybindings.quit, "q");
    assert_eq!(config.keybindings.help, "?");
    assert_eq!(config.keybindings.move_up, "k");
    assert_eq!(config.keybindings.move_down, "j");
    assert_eq!(config.keybindings.enter_dir, "l");
    assert_eq!(config.keybindings.leave_dir, "h");
    assert_eq!(config.theme.color_scheme, "system");
}
