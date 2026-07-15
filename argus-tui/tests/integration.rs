use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};

fn make_file(name: &str, size: u64) -> FileNode {
    FileNode {
        name: name.to_string(),
        parent: None,
        is_dir: false,
        file_type: FileType::File,
        size,
        children: Vec::new(),
    }
}

fn make_dir(name: &str, children: Vec<(&str, NodeIndex)>) -> FileNode {
    FileNode {
        name: name.to_string(),
        parent: None,
        is_dir: true,
        file_type: FileType::Directory,
        size: 0,
        children: children
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

fn make_snapshot(path: &str, arena: Vec<FileNode>) -> Snapshot {
    let size = arena[ROOT_NODE as usize].size;
    Snapshot::new(PathBuf::from(path), arena, size)
}

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
    let mut arena = vec![
        make_dir("test", vec![("a.txt", 1), ("b.txt", 2)]),
        make_file("a.txt", 100),
        make_file("b.txt", 200),
    ];
    arena[ROOT_NODE as usize].size = 300;
    let snap = make_snapshot("/tmp/test", arena);

    let json = serde_json::to_string_pretty(&snap).unwrap();
    let deserialized: Snapshot = serde_json::from_str(&json).unwrap();

    assert_eq!(snap.root_path, deserialized.root_path);
    assert_eq!(snap.total_size, deserialized.total_size);
    assert_eq!(
        snap.node(ROOT_NODE).children.len(),
        deserialized.node(ROOT_NODE).children.len()
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
