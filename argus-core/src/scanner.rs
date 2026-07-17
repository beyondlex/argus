use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use jwalk::{Parallelism, WalkDir};

use crate::bloom::SeenInodes;
use crate::model::{
    FileNode, FileType, NodeIndex, ScanError, Snapshot, SnapshotBuilder, ROOT_NODE,
};

// Parallel walk is disabled (Serial): TreeBuilder / walk stack is single-threaded.
// Walk depth stack replaces child_lookup for O(1) parent attachment.

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub file_count: u64,
    pub total_bytes: u64,
    pub total_disk_bytes: u64,
    pub current_path: Option<String>,
}

struct ProgressTracker {
    file_count: u64,
    total_bytes: u64,
    total_disk_bytes: u64,
    last_reported_file_count: u64,
    last_reported_total_bytes: u64,
    last_reported_total_disk_bytes: u64,
    current_path: Option<String>,
    progress_tx: Option<mpsc::Sender<ProgressUpdate>>,
}

const PROGRESS_FILE_BATCH: u64 = 32;
const PROGRESS_BYTES_BATCH: u64 = 1024 * 1024;

impl ProgressTracker {
    fn new(progress_tx: Option<mpsc::Sender<ProgressUpdate>>) -> Self {
        Self {
            file_count: 0,
            total_bytes: 0,
            total_disk_bytes: 0,
            last_reported_file_count: 0,
            last_reported_total_bytes: 0,
            last_reported_total_disk_bytes: 0,
            current_path: None,
            progress_tx,
        }
    }

    fn record(&mut self, files: u64, bytes: u64, disk_bytes: u64, path: Option<String>) {
        self.file_count = self.file_count.saturating_add(files);
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.total_disk_bytes = self.total_disk_bytes.saturating_add(disk_bytes);
        if path.is_some() {
            self.current_path = path;
        }
        self.maybe_report();
    }

    fn record_files_only(&mut self, files: u64) {
        self.record(files, 0, 0, None);
    }

    fn maybe_report(&mut self) {
        let file_delta = self
            .file_count
            .saturating_sub(self.last_reported_file_count);
        let size_delta = self
            .total_bytes
            .saturating_sub(self.last_reported_total_bytes);
        let disk_delta = self
            .total_disk_bytes
            .saturating_sub(self.last_reported_total_disk_bytes);
        if file_delta >= PROGRESS_FILE_BATCH
            || size_delta >= PROGRESS_BYTES_BATCH
            || disk_delta >= PROGRESS_BYTES_BATCH
        {
            self.last_reported_file_count = self.file_count;
            self.last_reported_total_bytes = self.total_bytes;
            self.last_reported_total_disk_bytes = self.total_disk_bytes;
            if let Some(ref tx) = self.progress_tx {
                let _ = tx.send(ProgressUpdate {
                    file_count: self.file_count,
                    total_bytes: self.total_bytes,
                    total_disk_bytes: self.total_disk_bytes,
                    current_path: self.current_path.clone(),
                });
            }
        }
    }

    fn is_active(&self) -> bool {
        self.progress_tx.is_some()
    }

    fn finish(&mut self) {
        if let Some(ref tx) = self.progress_tx {
            let _ = tx.send(ProgressUpdate {
                file_count: self.file_count,
                total_bytes: self.total_bytes,
                total_disk_bytes: self.total_disk_bytes,
                current_path: None,
            });
        }
    }
}

pub fn scan_path(
    path: &Path,
    cancel: &AtomicBool,
    progress_tx: Option<mpsc::Sender<ProgressUpdate>>,
) -> Result<Snapshot, ScanError> {
    if !path.exists() {
        return Err(ScanError::PathNotFound(path.to_path_buf()));
    }

    let mut seen_inodes = SeenInodes::new();
    let mut progress = ProgressTracker::new(progress_tx);

    let root_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let mut builder = SnapshotBuilder::new(&root_name);
    // stack[d] = NodeIndex of directory at walk depth d (depth 0 = root).
    let mut stack: Vec<NodeIndex> = vec![ROOT_NODE];

    for result in WalkDir::new(path)
        .follow_links(false)
        .skip_hidden(false)
        .parallelism(Parallelism::Serial)
    {
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }

        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };

        let depth = entry.depth();
        if depth == 0 {
            continue;
        }

        // Pop stack to parent of current entry.
        while stack.len() > depth {
            stack.pop();
        }
        let parent = match stack.last().copied() {
            Some(p) => p,
            None => ROOT_NODE,
        };

        let name = entry.file_name().to_string_lossy();

        if entry.file_type().is_dir() {
            let idx = builder.push_dir(parent, &name);
            stack.push(idx);
            progress.record_files_only(1);
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let entry_path = entry.path();

        if meta.is_file() || meta.is_symlink() {
            if let (Ok(device), Ok(inode)) = (get_device(&meta), get_inode(&meta)) {
                if !seen_inodes.insert((device, inode)) {
                    continue;
                }
            }
            if meta.is_file() {
                let current_path = progress
                    .is_active()
                    .then(|| entry_path.to_string_lossy().to_string());
                let du = get_disk_usage(&meta);
                progress.record(1, meta.len(), du, current_path);
            } else {
                progress.record_files_only(1);
            }
        }

        let (file_type, size, disk_usage) = node_meta(&meta);
        builder.push_file(parent, &name, file_type, size, disk_usage);
    }

    compute_size(&mut builder.nodes);
    compute_disk_usage(&mut builder.nodes);
    let total_size = builder.nodes[ROOT_NODE as usize].size();
    let total_disk_usage = builder.nodes[ROOT_NODE as usize].disk_usage();
    let snapshot = builder.finish(path.to_path_buf(), total_size, total_disk_usage);

    progress.finish();

    Ok(snapshot)
}

fn node_meta(meta: &std::fs::Metadata) -> (FileType, u64, u64) {
    let file_type = if meta.is_dir() {
        FileType::Directory
    } else if meta.is_symlink() {
        FileType::Symlink
    } else {
        detect_file_type(meta)
    };

    let size = if file_type == FileType::Directory || meta.is_symlink() {
        0
    } else {
        meta.len()
    };

    let disk_usage = if file_type == FileType::Directory || meta.is_symlink() {
        0
    } else {
        get_disk_usage(meta)
    };

    (file_type, size, disk_usage)
}

#[cfg(unix)]
fn get_disk_usage(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

#[cfg(not(unix))]
fn get_disk_usage(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

fn detect_file_type(meta: &std::fs::Metadata) -> FileType {
    let ft = meta.file_type();
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if ft.is_fifo() {
            return FileType::Fifo;
        }
        if ft.is_socket() {
            return FileType::Socket;
        }
        if ft.is_char_device() || ft.is_block_device() {
            return FileType::Device;
        }
    }
    if ft.is_symlink() {
        return FileType::Symlink;
    }
    if ft.is_dir() {
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

pub fn list_dir(path: &Path) -> Result<Snapshot, ScanError> {
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

    let mut builder = SnapshotBuilder::new(&name);

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let meta = match std::fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if meta.is_dir() {
            builder.push_dir(ROOT_NODE, &name);
        } else {
            let (file_type, size, disk_usage) = node_meta(&meta);
            builder.push_file(ROOT_NODE, &name, file_type, size, disk_usage);
        }
    }

    let total_size: u64 = builder.nodes.iter().skip(1).map(|n| n.size()).sum();
    let total_disk_usage: u64 = builder.nodes.iter().skip(1).map(|n| n.disk_usage()).sum();
    // Roll up root totals for shallow list.
    if let Some(root) = builder.nodes.get_mut(ROOT_NODE as usize) {
        root.set_size(total_size);
        root.set_disk_usage(total_disk_usage);
    }

    Ok(builder.finish(path.to_path_buf(), total_size, total_disk_usage))
}

fn compute_size(nodes: &mut [FileNode]) {
    for i in (1..nodes.len()).rev() {
        let size = nodes[i].size();
        if let Some(parent) = nodes[i].parent() {
            let total = nodes[parent as usize].size().saturating_add(size);
            nodes[parent as usize].set_size(total);
        }
    }
}

fn compute_disk_usage(nodes: &mut [FileNode]) {
    for i in (1..nodes.len()).rev() {
        let du = nodes[i].disk_usage();
        if let Some(parent) = nodes[i].parent() {
            let total = nodes[parent as usize].disk_usage().saturating_add(du);
            nodes[parent as usize].set_disk_usage(total);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Snapshot;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    fn child_idx(snap: &Snapshot, parent: NodeIndex, name: &str) -> NodeIndex {
        snap.child_idx(parent, name).unwrap()
    }

    fn child_name<'a>(snap: &'a Snapshot, parent: NodeIndex, name: &str) -> &'a str {
        let idx = child_idx(snap, parent, name);
        snap.name(idx)
    }

    #[test]
    fn test_scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let cancel = AtomicBool::new(false);
        let result = scan_path(dir.path(), &cancel, None);
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        assert!(snapshot.node(ROOT_NODE).is_dir());
        assert!(snapshot.children_is_empty(ROOT_NODE));
        assert_eq!(snapshot.total_size, 0);
    }

    #[test]
    fn test_scan_single_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let cancel = AtomicBool::new(false);
        let result = scan_path(dir.path(), &cancel, None);
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        let idx = child_idx(&snapshot, ROOT_NODE, "test.txt");
        assert!(!snapshot.node(idx).is_dir());
        assert_eq!(snapshot.node(idx).size(), 11);
    }

    #[test]
    fn test_scan_nested_directories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
        fs::write(dir.path().join("a/b/c/file.txt"), "content").unwrap();

        let cancel = AtomicBool::new(false);
        let result = scan_path(dir.path(), &cancel, None);
        assert!(result.is_ok());
        let snapshot = result.unwrap();

        let a_idx = child_idx(&snapshot, ROOT_NODE, "a");
        let b_idx = child_idx(&snapshot, a_idx, "b");
        let c_idx = child_idx(&snapshot, b_idx, "c");
        let file_idx = child_idx(&snapshot, c_idx, "file.txt");
        assert_eq!(snapshot.node(file_idx).size(), 7);
        assert_eq!(snapshot.node(c_idx).size(), 7);
        assert!(snapshot.node(b_idx).size() >= 7);
        assert!(snapshot.node(a_idx).size() >= 7);
    }

    #[test]
    fn test_scan_path_not_found() {
        let cancel = AtomicBool::new(false);
        let result = scan_path(Path::new("/nonexistent/path"), &cancel, None);
        assert!(matches!(result, Err(ScanError::PathNotFound(_))));
    }

    #[test]
    fn test_scan_cancelled() {
        let dir = TempDir::new().unwrap();
        for i in 0..10 {
            fs::write(dir.path().join(format!("file_{}.txt", i)), "data").unwrap();
        }
        let cancel = AtomicBool::new(true);
        let result = scan_path(dir.path(), &cancel, None);
        assert!(matches!(result, Err(ScanError::Cancelled)));
    }

    #[test]
    fn test_compute_size() {
        let mut nodes: Vec<FileNode> = {
            let mut b = SnapshotBuilder::new("parent");
            b.push_file(ROOT_NODE, "child1", FileType::File, 100, 100);
            b.push_file(ROOT_NODE, "child2", FileType::File, 200, 200);
            b.nodes
        };
        // parents already set by builder
        compute_size(&mut nodes);
        assert_eq!(nodes[0].size(), 300);
    }

    #[test]
    fn test_list_dir_empty_directory() {
        let dir = TempDir::new().unwrap();
        let result = list_dir(dir.path());
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        assert!(snapshot.node(ROOT_NODE).is_dir());
        assert!(snapshot.children_is_empty(ROOT_NODE));
    }

    #[test]
    fn test_list_dir_with_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::write(dir.path().join("b.txt"), "world").unwrap();

        let result = list_dir(dir.path());
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        let a = child_idx(&snapshot, ROOT_NODE, "a.txt");
        assert!(!snapshot.node(a).is_dir());
        assert_eq!(snapshot.node(a).size(), 5);
        let b = child_idx(&snapshot, ROOT_NODE, "b.txt");
        assert!(!snapshot.node(b).is_dir());
        assert_eq!(snapshot.node(b).size(), 5);
    }

    #[test]
    fn test_list_dir_with_subdirectory() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("file.txt"), "data").unwrap();

        let result = list_dir(dir.path());
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        let sub = child_idx(&snapshot, ROOT_NODE, "sub");
        assert!(snapshot.node(sub).is_dir());
        assert_eq!(snapshot.node(sub).size(), 0);
        assert!(snapshot.children_is_empty(sub));
    }

    #[cfg(unix)]
    #[test]
    fn test_list_dir_preserves_symlink_type() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("linked.txt");
        fs::write(&target, "content").unwrap();
        symlink(&target, &link).unwrap();

        let result = list_dir(dir.path());
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        let linked = child_idx(&snapshot, ROOT_NODE, "linked.txt");
        assert!(!snapshot.node(linked).is_dir());
        assert_eq!(snapshot.node(linked).size(), 0);
        assert_eq!(snapshot.node(linked).file_type(), FileType::Symlink);
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
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();
        let idx = child_idx(&snapshot, ROOT_NODE, "linked.txt");
        assert!(!snapshot.node(idx).is_dir());
        assert_eq!(snapshot.node(idx).size(), 0);
        assert!(snapshot.children_is_empty(idx));
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
    fn test_scan_many_siblings() {
        let dir = TempDir::new().unwrap();
        let n = 200;
        for i in 0..n {
            fs::write(dir.path().join(format!("file_{i:03}")), "x").unwrap();
        }
        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();
        assert_eq!(snapshot.children_len(ROOT_NODE), n);
        for i in 0..n {
            let name = format!("file_{i:03}");
            assert_eq!(child_name(&snapshot, ROOT_NODE, &name), name);
        }
    }

    #[test]
    fn test_scan_empty_subdirectory_included_in_arena() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("empty_sub")).unwrap();
        fs::write(dir.path().join("file.txt"), "data").unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();

        assert!(snapshot.node(ROOT_NODE).is_dir());
        assert_eq!(snapshot.children_len(ROOT_NODE), 2);

        let empty = child_idx(&snapshot, ROOT_NODE, "empty_sub");
        assert!(snapshot.node(empty).is_dir());
        assert_eq!(snapshot.node(empty).size(), 0);
        assert!(snapshot.children_is_empty(empty));

        let file = child_idx(&snapshot, ROOT_NODE, "file.txt");
        assert!(!snapshot.node(file).is_dir());
        assert_eq!(snapshot.node(file).size(), 4);

        assert_eq!(snapshot.node(ROOT_NODE).size(), 4);
    }

    #[test]
    fn test_scan_deeply_nested_empty_directories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c")).unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();

        assert!(snapshot.node(ROOT_NODE).is_dir());
        assert_eq!(snapshot.node(ROOT_NODE).size(), 0);
        assert_eq!(snapshot.children_len(ROOT_NODE), 1);

        let a_idx = child_idx(&snapshot, ROOT_NODE, "a");
        assert!(snapshot.node(a_idx).is_dir());
        assert_eq!(snapshot.node(a_idx).size(), 0);
        assert_eq!(snapshot.children_len(a_idx), 1);

        let b_idx = child_idx(&snapshot, a_idx, "b");
        assert!(snapshot.node(b_idx).is_dir());
        assert_eq!(snapshot.node(b_idx).size(), 0);
        assert_eq!(snapshot.children_len(b_idx), 1);

        let c_idx = child_idx(&snapshot, b_idx, "c");
        assert!(snapshot.node(c_idx).is_dir());
        assert_eq!(snapshot.node(c_idx).size(), 0);
        assert!(snapshot.children_is_empty(c_idx));
    }

    #[test]
    fn test_scan_mixed_empty_and_nonempty_subdirectories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("empty")).unwrap();
        fs::create_dir_all(dir.path().join("full/sub")).unwrap();
        fs::write(dir.path().join("full/sub/file.txt"), "data").unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();

        assert_eq!(snapshot.children_len(ROOT_NODE), 2);

        let empty = child_idx(&snapshot, ROOT_NODE, "empty");
        assert!(snapshot.node(empty).is_dir());
        assert_eq!(snapshot.node(empty).size(), 0);
        assert!(snapshot.children_is_empty(empty));

        let full_idx = child_idx(&snapshot, ROOT_NODE, "full");
        assert!(snapshot.node(full_idx).is_dir());
        assert_eq!(snapshot.node(full_idx).size(), 4);

        let sub_idx = snapshot.child_idx(full_idx, "sub").unwrap();
        assert!(snapshot.node(sub_idx).is_dir());
        assert_eq!(snapshot.node(sub_idx).size(), 4);

        let file = snapshot.child_idx(sub_idx, "file.txt").unwrap();
        assert!(!snapshot.node(file).is_dir());
        assert_eq!(snapshot.node(file).size(), 4);

        assert_eq!(snapshot.node(ROOT_NODE).size(), 4);
    }

    #[test]
    fn test_scan_children_all_present() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("z.txt"), "z").unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("m.txt"), "m").unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();
        let mut names: Vec<&str> = snapshot
            .children(ROOT_NODE)
            .iter()
            .map(|&idx| snapshot.name(idx))
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.txt", "m.txt", "z.txt"]);
    }

    #[test]
    fn test_scan_progress_counts_dirs_and_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("readme.md"), "project").unwrap();

        let nm = dir.path().join("node_modules");
        fs::create_dir_all(nm.join("pkg/src")).unwrap();
        fs::write(nm.join("pkg").join("index.js"), "module.exports = 1").unwrap();
        fs::write(nm.join("pkg/src").join("lib.js"), "function f() {}").unwrap();

        let cancel = AtomicBool::new(false);
        let (tx, rx) = std::sync::mpsc::channel();
        let snapshot = scan_path(dir.path(), &cancel, Some(tx)).unwrap();

        let final_update = rx.try_iter().last().unwrap();
        assert_eq!(final_update.total_bytes, snapshot.total_size);
        assert_eq!(final_update.total_bytes, 7 + 18 + 15);
        assert_eq!(final_update.total_disk_bytes, snapshot.total_disk_usage);
        // file_count: readme.md + node_modules + pkg + src + index.js + lib.js
        assert_eq!(final_update.file_count, 6);
    }

    #[test]
    fn test_scan_includes_hidden_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".hidden"), "secret").unwrap();
        fs::write(dir.path().join(".dotfile.txt"), "data").unwrap();
        fs::write(dir.path().join("visible.txt"), "hello").unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();

        assert_eq!(snapshot.children_len(ROOT_NODE), 3);
        let mut names: Vec<&str> = snapshot
            .children(ROOT_NODE)
            .iter()
            .map(|&idx| snapshot.name(idx))
            .collect();
        names.sort();
        assert_eq!(names, vec![".dotfile.txt", ".hidden", "visible.txt"]);
        assert_eq!(snapshot.total_files, 3);
    }
}
