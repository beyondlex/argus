use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use chrono::DateTime;
use ignore::WalkBuilder;

use crate::model::{FileNode, FileType, ScanError, Snapshot};

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub file_count: u64,
    pub total_bytes: u64,
}

pub fn scan_path(
    path: &Path,
    cancel: &AtomicBool,
    progress_tx: Option<mpsc::Sender<ProgressUpdate>>,
    skip_dirs: &[String],
) -> Result<Snapshot, ScanError> {
    if !path.exists() {
        return Err(ScanError::PathNotFound(path.to_path_buf()));
    }

    let mut entries: Vec<(PathBuf, std::fs::Metadata)> = Vec::new();
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut file_count = 0u64;
    let mut total_bytes = 0u64;

    // Collect paths of directories matching skip patterns so we can
    // walk them fully for size counting after the main walk (no tree
    // expansion inside skipped directories — keeps memory low).
    let skipped = Arc::new(Mutex::new(Vec::new()));
    let skipped_clone = skipped.clone();
    let skip_patterns: Vec<String> = skip_dirs.to_vec();

    let walker = WalkBuilder::new(path)
        .follow_links(false)
        .git_ignore(false) // Don't silently skip gitignored dirs; our
        // filter_entry handles which to skip
        .filter_entry(move |entry| {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy();
                if skip_patterns.iter().any(|s| name.as_ref() == s.as_str()) {
                    skipped_clone
                        .lock()
                        .unwrap()
                        .push(entry.path().to_path_buf());
                    return false;
                }
            }
            true
        })
        .build();

    for result in walker {
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }

        let entry = match result {
            Ok(e) => e,
            Err(err) => {
                if let Some(io_err) = err.io_error() {
                    if io_err.kind() == std::io::ErrorKind::PermissionDenied {
                        return Err(ScanError::PermissionDenied(path.to_path_buf()));
                    }
                }
                continue;
            }
        };

        let entry_path = entry.path().to_path_buf();

        let meta = match std::fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(ScanError::PermissionDenied(entry_path));
            }
            Err(_) => continue,
        };

        if meta.is_file() || meta.is_symlink() {
            if let (Ok(device), Ok(inode)) = (get_device(&meta), get_inode(&meta)) {
                if !seen_inodes.insert((device, inode)) {
                    continue;
                }
            }
            if meta.is_file() {
                total_bytes = total_bytes.saturating_add(meta.len());
            }
        }

        entries.push((entry_path, meta));
        file_count += 1;

        if file_count.is_multiple_of(1_000) {
            if let Some(ref tx) = progress_tx {
                let _ = tx.send(ProgressUpdate {
                    file_count,
                    total_bytes,
                });
            }
        }
    }

    entries.sort_by(|a, b| {
        let depth_a = a.0.components().count();
        let depth_b = b.0.components().count();
        depth_b.cmp(&depth_a)
    });

    let root_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let mut root_node = FileNode {
        name: root_name,
        is_dir: true,
        file_type: FileType::Directory,
        size: 0,
        modified: None,
        inode: None,
        device: None,
        has_metadata: true,
        children: HashMap::new(),
    };

    for (entry_path, meta) in &entries {
        if meta.is_dir() {
            continue;
        }

        let rel_path = entry_path.strip_prefix(path).unwrap_or(entry_path);
        let components: Vec<std::path::Component> = rel_path.components().collect();
        if components.is_empty() {
            continue;
        }

        let node = create_file_node(entry_path, meta);
        insert_node(&mut root_node, &components, node);
    }

    // Walk skipped directories fully for accurate size counting.  Also
    // record the directory structure — immediate children get their total
    // recursive size computed (one level deep), while deeper structure
    // tracks names only (shown as "..." in the UI).
    for skip_path in skipped.lock().unwrap().drain(..) {
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }

        let (size, count) = walk_dir_size(&skip_path, cancel, &mut seen_inodes);
        total_bytes = total_bytes.saturating_add(size);
        file_count += count;

        let dir_name = skip_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut structure = walk_dir_structure(&skip_path);

        // Compute total sizes for each immediate child of the skipped dir
        // (one level deep).  Uses a fresh inode set to avoid the parent
        // walk_dir_size having already recorded these inodes.
        let mut child_seen_inodes: HashSet<(u64, u64)> = HashSet::new();
        if let Ok(read_dir) = std::fs::read_dir(&skip_path) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(child) = structure.get_mut(&name) {
                    if child.is_dir {
                        let (child_size, _) =
                            walk_dir_size(&entry.path(), cancel, &mut child_seen_inodes);
                        child.size = child_size;
                        child.has_metadata = true;
                    } else {
                        if let Ok(meta) = entry.metadata() {
                            child.size = meta.len();
                            child.has_metadata = true;
                        }
                    }
                }
            }
        }

        let dir_node = FileNode {
            name: dir_name,
            is_dir: true,
            file_type: FileType::Directory,
            size,
            modified: None,
            inode: None,
            device: None,
            has_metadata: true,
            children: structure,
        };

        let rel_path = skip_path.strip_prefix(path).unwrap_or(&skip_path);
        let components: Vec<std::path::Component> = rel_path.components().collect();
        if !components.is_empty() {
            insert_node(&mut root_node, &components, dir_node);
        }
    }

    compute_size(&mut root_node);

    let snapshot = Snapshot::new(path.to_path_buf(), root_node, total_bytes);

    if let Some(ref tx) = progress_tx {
        let _ = tx.send(ProgressUpdate {
            file_count,
            total_bytes,
        });
    }

    Ok(snapshot)
}

fn create_file_node(path: &Path, meta: &std::fs::Metadata) -> FileNode {
    let is_dir = meta.is_dir();
    let file_type = if is_dir {
        FileType::Directory
    } else if meta.is_symlink() {
        FileType::Symlink
    } else {
        detect_file_type(path)
    };

    let modified = meta.modified().ok().map(|t| {
        let duration = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        DateTime::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos())
            .unwrap_or_default()
    });

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let size = if is_dir || meta.is_symlink() {
        0
    } else {
        meta.len()
    };

    FileNode {
        name,
        is_dir,
        file_type,
        size,
        modified,
        inode: get_inode(meta).ok(),
        device: get_device(meta).ok(),
        has_metadata: true,
        children: HashMap::new(),
    }
}

fn detect_file_type(path: &Path) -> FileType {
    let Some(meta) = path.symlink_metadata().ok() else {
        return FileType::File;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if meta.file_type().is_fifo() {
            return FileType::Fifo;
        }
        if meta.file_type().is_socket() {
            return FileType::Socket;
        }
        if meta.file_type().is_char_device() || meta.file_type().is_block_device() {
            return FileType::Device;
        }
    }
    if meta.is_symlink() {
        return FileType::Symlink;
    }
    if meta.is_dir() {
        return FileType::Directory;
    }
    FileType::File
}

#[cfg(unix)]
fn get_inode(meta: &std::fs::Metadata) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(meta.ino())
}

#[cfg(not(unix))]
fn get_inode(_meta: &std::fs::Metadata) -> std::io::Result<u64> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "inode not supported",
    ))
}

#[cfg(unix)]
fn get_device(meta: &std::fs::Metadata) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(meta.dev())
}

#[cfg(not(unix))]
fn get_device(_meta: &std::fs::Metadata) -> std::io::Result<u64> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "device not supported",
    ))
}

fn insert_node(parent: &mut FileNode, components: &[std::path::Component], node: FileNode) {
    if components.is_empty() {
        return;
    }

    let comp_name = components[0].as_os_str().to_string_lossy().to_string();

    if components.len() == 1 {
        parent.children.insert(comp_name, node);
    } else {
        let entry = parent.children.entry(comp_name).or_insert_with(|| {
            let name = components[0].as_os_str().to_string_lossy().to_string();
            FileNode {
                name,
                is_dir: true,
                file_type: FileType::Directory,
                size: 0,
                modified: None,
                inode: None,
                device: None,
                has_metadata: true,
                children: HashMap::new(),
            }
        });
        if !entry.is_dir {
            return;
        }
        insert_node(entry, &components[1..], node);
    }
}

pub fn list_dir(path: &Path) -> Result<FileNode, ScanError> {
    if !path.exists() {
        return Err(ScanError::PathNotFound(path.to_path_buf()));
    }
    if !path.is_dir() {
        return Err(ScanError::PathNotFound(path.to_path_buf()));
    }

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let read_dir = match std::fs::read_dir(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(ScanError::PermissionDenied(path.to_path_buf()));
        }
        Err(e) => return Err(ScanError::Io(e)),
    };

    let mut children = HashMap::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let node = create_file_node(&entry.path(), &meta);
        children.insert(node.name.clone(), node);
    }

    Ok(FileNode {
        name,
        is_dir: true,
        file_type: FileType::Directory,
        size: 0,
        modified: None,
        inode: None,
        device: None,
        has_metadata: true,
        children,
    })
}

fn compute_size(node: &mut FileNode) -> u64 {
    if node.children.is_empty() {
        return node.size;
    }

    // If all direct children lack metadata, this node's size was
    // pre-computed (e.g., skipped dir with structural-only children).
    if node.children.values().all(|c| !c.has_metadata) {
        return node.size;
    }

    let mut total = 0u64;
    for child in node.children.values_mut() {
        total = total.saturating_add(compute_size(child));
    }
    node.size = total;
    total
}

/// Walk a directory tree fully counting file sizes, without building a tree.
/// Returns (total_bytes, file_count).  Inode dedup uses the same
/// `seen_inodes` set as the parent scan to avoid double-counting across
/// hard links shared between skipped and non-skipped parts of the tree.
fn walk_dir_size(
    path: &Path,
    cancel: &AtomicBool,
    seen_inodes: &mut HashSet<(u64, u64)>,
) -> (u64, u64) {
    let mut total = 0u64;
    let mut count = 0u64;

    let walker = WalkBuilder::new(path).follow_links(false).build();

    for result in walker {
        if cancel.load(Ordering::Relaxed) {
            return (total, count);
        }

        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };

        let meta = match std::fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if meta.is_file() {
            if let (Ok(device), Ok(inode)) = (get_device(&meta), get_inode(&meta)) {
                if !seen_inodes.insert((device, inode)) {
                    continue;
                }
            }
            total = total.saturating_add(meta.len());
            count += 1;
        }
    }

    (total, count)
}

/// Walk a directory tree recording its structure (names, directory nesting)
/// without individual file sizes.  All returned nodes have `has_metadata: false`
/// and `size: 0`, signalling to the UI that individual size data is unavailable.
fn walk_dir_structure(path: &Path) -> HashMap<String, FileNode> {
    let mut children = HashMap::new();
    let Ok(read_dir) = std::fs::read_dir(path) else {
        return children;
    };
    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let file_type = if is_dir {
            FileType::Directory
        } else {
            detect_file_type(&entry.path())
        };

        let node = FileNode {
            name: name.clone(),
            is_dir,
            file_type,
            size: 0,
            modified: None,
            inode: None,
            device: None,
            has_metadata: false,
            children: if is_dir {
                walk_dir_structure(&entry.path())
            } else {
                HashMap::new()
            },
        };
        children.insert(name, node);
    }
    children
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    #[test]
    fn test_scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let cancel = AtomicBool::new(false);
        let result = scan_path(dir.path(), &cancel, None, &[]);
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        assert!(snapshot.root_node.is_dir);
        assert!(snapshot.root_node.children.is_empty());
        assert_eq!(snapshot.total_size, 0);
    }

    #[test]
    fn test_scan_single_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let cancel = AtomicBool::new(false);
        let result = scan_path(dir.path(), &cancel, None, &[]);
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        assert_eq!(snapshot.root_node.children.len(), 1);
        let child = snapshot.root_node.children.get("test.txt").unwrap();
        assert!(!child.is_dir);
        assert_eq!(child.size, 11);
    }

    #[test]
    fn test_scan_nested_directories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
        fs::write(dir.path().join("a/b/c/file.txt"), "content").unwrap();

        let cancel = AtomicBool::new(false);
        let result = scan_path(dir.path(), &cancel, None, &[]);
        assert!(result.is_ok());
        let snapshot = result.unwrap();

        let a = snapshot.root_node.children.get("a").unwrap();
        let b = a.children.get("b").unwrap();
        let c = b.children.get("c").unwrap();
        let file = c.children.get("file.txt").unwrap();
        assert_eq!(file.size, 7);
        assert_eq!(c.size, 7);
        assert!(b.size >= 7);
        assert!(a.size >= 7);
    }

    #[test]
    fn test_scan_path_not_found() {
        let cancel = AtomicBool::new(false);
        let result = scan_path(Path::new("/nonexistent/path"), &cancel, None, &[]);
        assert!(matches!(result, Err(ScanError::PathNotFound(_))));
    }

    #[test]
    fn test_scan_cancelled() {
        let dir = TempDir::new().unwrap();
        for i in 0..10 {
            fs::write(dir.path().join(format!("file_{}.txt", i)), "data").unwrap();
        }

        let cancel = AtomicBool::new(true);
        let result = scan_path(dir.path(), &cancel, None, &[]);
        assert!(matches!(result, Err(ScanError::Cancelled)));
    }

    #[test]
    fn test_create_file_node_regular_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "data").unwrap();
        let meta = file_path.metadata().unwrap();

        let node = create_file_node(&file_path, &meta);
        assert_eq!(node.name, "test.txt");
        assert!(!node.is_dir);
        assert_eq!(node.file_type, FileType::File);
        assert_eq!(node.size, 4);
    }

    #[test]
    fn test_compute_size() {
        let mut parent = FileNode {
            name: "parent".into(),
            is_dir: true,
            file_type: FileType::Directory,
            size: 0,
            modified: None,
            inode: None,
            device: None,
            has_metadata: true,
            children: HashMap::new(),
        };

        parent.children.insert(
            "child1".into(),
            FileNode {
                name: "child1".into(),
                is_dir: false,
                file_type: FileType::File,
                size: 100,
                modified: None,
                inode: None,
                device: None,
                has_metadata: true,
                children: HashMap::new(),
            },
        );

        parent.children.insert(
            "child2".into(),
            FileNode {
                name: "child2".into(),
                is_dir: false,
                file_type: FileType::File,
                size: 200,
                modified: None,
                inode: None,
                device: None,
                has_metadata: true,
                children: HashMap::new(),
            },
        );

        let total = compute_size(&mut parent);
        assert_eq!(total, 300);
        assert_eq!(parent.size, 300);
    }

    #[test]
    fn test_list_dir_empty_directory() {
        let dir = TempDir::new().unwrap();
        let result = list_dir(dir.path());
        assert!(result.is_ok());
        let node = result.unwrap();
        assert!(node.is_dir);
        assert!(node.children.is_empty());
    }

    #[test]
    fn test_list_dir_with_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::write(dir.path().join("b.txt"), "world").unwrap();

        let result = list_dir(dir.path());
        assert!(result.is_ok());
        let node = result.unwrap();
        assert_eq!(node.children.len(), 2);

        let a = node.children.get("a.txt").unwrap();
        assert!(!a.is_dir);
        assert_eq!(a.size, 5);

        let b = node.children.get("b.txt").unwrap();
        assert!(!b.is_dir);
        assert_eq!(b.size, 5);
    }

    #[test]
    fn test_list_dir_with_subdirectory() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("file.txt"), "data").unwrap();

        let result = list_dir(dir.path());
        assert!(result.is_ok());
        let node = result.unwrap();
        assert_eq!(node.children.len(), 1);

        let sub = node.children.get("sub").unwrap();
        assert!(sub.is_dir);
        assert_eq!(sub.size, 0); // Not recursively summed
        assert!(sub.children.is_empty()); // Not populated
    }

    #[cfg(unix)]
    #[test]
    fn test_scan_preserves_symlink_type() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("linked.txt");
        fs::write(&target, "content").unwrap();
        symlink(&target, &link).unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None, &[]).unwrap();
        let node = snapshot.root_node.children.get("linked.txt").unwrap();

        assert_eq!(node.file_type, FileType::Symlink);
        assert_eq!(node.size, 0);
        assert!(node.children.is_empty());
    }

    #[test]
    fn test_list_dir_path_not_found() {
        let result = list_dir(Path::new("/nonexistent/path"));
        assert!(matches!(result, Err(ScanError::PathNotFound(_))));
    }

    #[test]
    fn test_list_dir_file_not_dir() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "content").unwrap();
        let result = list_dir(&file_path);
        assert!(matches!(result, Err(ScanError::PathNotFound(_))));
    }

    #[test]
    fn test_hash_root_path_consistency() {
        let path = PathBuf::from("/home/user");
        let hash1 = crate::model::hash_root_path(&path);
        let hash2 = crate::model::hash_root_path(&path);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_walk_dir_size_counts_nested_files() {
        let dir = TempDir::new().unwrap();
        // Create nested structure inside a subdirectory
        let sub = dir.path().join("sub");
        fs::create_dir_all(sub.join("deep")).unwrap();
        fs::write(sub.join("a.txt"), "hello").unwrap();
        fs::write(sub.join("deep").join("b.txt"), "world!").unwrap();

        let cancel = AtomicBool::new(false);
        let mut seen = HashSet::new();
        let (size, count) = walk_dir_size(&sub, &cancel, &mut seen);
        // "hello" = 5 bytes, "world!" = 6 bytes
        assert_eq!(size, 11);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_scan_with_skip_dir_includes_full_size() {
        let dir = TempDir::new().unwrap();
        // Normal file outside skip directory
        fs::write(dir.path().join("readme.md"), "project").unwrap();
        // Create a "node_modules" dir with deeply nested content
        let nm = dir.path().join("node_modules");
        fs::create_dir_all(nm.join("pkg/src")).unwrap();
        fs::write(nm.join("pkg").join("index.js"), "module.exports = 1").unwrap();
        fs::write(nm.join("pkg/src").join("lib.js"), "function f() {}").unwrap();

        let cancel = AtomicBool::new(false);
        let skip = vec!["node_modules".to_string()];
        let snapshot = scan_path(dir.path(), &cancel, None, &skip).unwrap();

        // "project" = 7 bytes, "module.exports = 1" = 18, "function f() {}" = 15
        assert_eq!(snapshot.total_size, 7 + 18 + 15);

        let nm_node = snapshot.root_node.children.get("node_modules").unwrap();
        assert!(nm_node.is_dir);
        assert!(!nm_node.children.is_empty());
        assert!(nm_node.has_metadata);
        // Immediate children get accurate sizes (one level deep)
        let pkg = nm_node.children.get("pkg").unwrap();
        assert!(pkg.has_metadata);
        assert_eq!(pkg.size, 18 + 15);
        // Deeper levels (pkg/src) have no metadata
        let src = pkg.children.get("src").unwrap();
        assert!(!src.has_metadata);
        assert_eq!(nm_node.size, 18 + 15); // Full recursive size, not just one level
    }

    #[test]
    fn test_scan_skip_gitignored_dirs() {
        // Simulate a Rust project: "target" dir with files + .gitignore that
        // ignores it.  Without git_ignore(false) the walker would silently skip
        // "target" before filter_entry ever sees it.
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap(); // 12 bytes

        let tg = dir.path().join("target");
        fs::create_dir_all(tg.join("debug")).unwrap();
        fs::write(tg.join("debug").join("a.o"), "AAAA").unwrap(); // 4 bytes
        fs::write(tg.join("b.o"), "BBBBBB").unwrap(); // 6 bytes

        let cancel = AtomicBool::new(false);
        let skip = vec!["target".to_string()];
        let snapshot = scan_path(dir.path(), &cancel, None, &skip).unwrap();

        // Total should include both: main.rs + inside target
        assert_eq!(snapshot.total_size, 12 + 4 + 6);

        let tg_node = snapshot.root_node.children.get("target").unwrap();
        assert!(tg_node.is_dir);
        assert_eq!(tg_node.size, 4 + 6); // Full recursive size
    }
}
