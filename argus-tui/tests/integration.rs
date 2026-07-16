use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use argus_core::{FileType, SnapshotBuilder, ROOT_NODE};

// ── State logic tests ───────────────────────────────────────────────────────

#[test]
fn test_scan_cancelled() {
    let dir = std::env::temp_dir().join("argus_test_cancel");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("test.txt"), "data").unwrap();

    let cancel = AtomicBool::new(true);
    let result = argus_core::scan_path(&dir, &cancel, None);
    assert!(result.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_snapshot_serialization_roundtrip() {
    let mut b = SnapshotBuilder::new("test");
    b.push_file(ROOT_NODE, "a.txt", FileType::File, 100, 100);
    b.push_file(ROOT_NODE, "b.txt", FileType::File, 200, 200);
    b.nodes[0].set_size(300);
    let snap = b.finish(PathBuf::from("/tmp/test"), 300, 0);

    let bytes = snap.to_compact_bytes().unwrap();
    let deserialized = argus_core::Snapshot::from_bytes(&bytes).unwrap();

    assert_eq!(snap.root_path, deserialized.root_path);
    assert_eq!(snap.total_size, deserialized.total_size);
    assert_eq!(
        snap.children_len(ROOT_NODE),
        deserialized.children_len(ROOT_NODE)
    );
    assert_eq!(deserialized.name(1), "a.txt");
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
