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
