use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use jwalk::WalkDir;

use crate::model::{FileNode, FileType, NodeIndex, ScanError, Snapshot, ROOT_NODE};

// Parallel walk via jwalk: workers stat in parallel, main thread inserts
// into TreeBuilder sequentially (TreeBuilder and seen_inodes are not
// thread-safe).

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub file_count: u64,
    pub total_bytes: u64,
    pub current_path: Option<String>,
}

struct ProgressTracker {
    file_count: u64,
    total_bytes: u64,
    last_reported_file_count: u64,
    last_reported_total_bytes: u64,
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
            last_reported_file_count: 0,
            last_reported_total_bytes: 0,
            current_path: None,
            progress_tx,
        }
    }

    fn record(&mut self, files: u64, bytes: u64, path: Option<String>) {
        self.file_count = self.file_count.saturating_add(files);
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        if path.is_some() {
            self.current_path = path;
        }
        self.maybe_report();
    }

    fn record_files_only(&mut self, files: u64) {
        self.record(files, 0, None);
    }

    fn maybe_report(&mut self) {
        let file_delta = self
            .file_count
            .saturating_sub(self.last_reported_file_count);
        let size_delta = self
            .total_bytes
            .saturating_sub(self.last_reported_total_bytes);
        if file_delta >= PROGRESS_FILE_BATCH || size_delta >= PROGRESS_BYTES_BATCH {
            self.last_reported_file_count = self.file_count;
            self.last_reported_total_bytes = self.total_bytes;
            if let Some(ref tx) = self.progress_tx {
                let _ = tx.send(ProgressUpdate {
                    file_count: self.file_count,
                    total_bytes: self.total_bytes,
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
                current_path: None,
            });
        }
    }
}

struct TreeBuilder {
    arena: Vec<FileNode>,
    /// Scan-time O(1) child lookup; children Vec remains the public layout.
    child_lookup: HashMap<NodeIndex, HashMap<String, NodeIndex>>,
}

impl TreeBuilder {
    fn new(root_name: &str) -> Self {
        let mut arena = Vec::with_capacity(4096);
        arena.push(FileNode {
            name: root_name.to_string(),
            parent: None,
            is_dir: true,
            file_type: FileType::Directory,
            size: 0,
            children: Vec::new(),
        });
        Self {
            arena,
            child_lookup: HashMap::new(),
        }
    }

    fn find_or_create_child(&mut self, parent: NodeIndex, name: &str) -> NodeIndex {
        if let Some(&idx) = self.child_lookup.get(&parent).and_then(|m| m.get(name)) {
            return idx;
        }
        let new_idx = self.arena.len() as NodeIndex;
        self.arena.push(FileNode {
            name: name.to_string(),
            parent: Some(parent),
            is_dir: true,
            file_type: FileType::Directory,
            size: 0,
            children: Vec::new(),
        });
        self.arena[parent as usize]
            .children
            .push((name.to_string(), new_idx));
        self.child_lookup
            .entry(parent)
            .or_default()
            .insert(name.to_string(), new_idx);
        new_idx
    }

    fn ensure_dir_path(&mut self, rel: &Path) {
        let components: Vec<_> = rel.components().collect();
        let mut parent = ROOT_NODE;
        for comp in &components {
            let name = comp.as_os_str().to_string_lossy();
            parent = self.find_or_create_child(parent, &name);
        }
    }

    fn insert_file(&mut self, path: &Path, root_path: &Path, mut node: FileNode) {
        let rel = path.strip_prefix(root_path).unwrap_or(path);
        let components: Vec<_> = rel.components().collect();
        if components.is_empty() {
            return;
        }
        let parent = if components.len() > 1 {
            let mut parent = ROOT_NODE;
            for comp in components.iter().take(components.len() - 1) {
                let name = comp.as_os_str().to_string_lossy();
                parent = self.find_or_create_child(parent, &name);
            }
            parent
        } else {
            ROOT_NODE
        };
        node.parent = Some(parent);
        let new_idx = self.arena.len() as NodeIndex;
        let name = node.name.clone();
        self.arena.push(node);
        self.arena[parent as usize]
            .children
            .push((name.clone(), new_idx));
        self.child_lookup
            .entry(parent)
            .or_default()
            .insert(name, new_idx);
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

    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut progress = ProgressTracker::new(progress_tx);

    let root_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let mut tree = TreeBuilder::new(&root_name);

    for result in WalkDir::new(path).follow_links(false).skip_hidden(false) {
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }

        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.depth() == 0 {
            continue;
        }

        if entry.file_type().is_dir() {
            let entry_path = entry.path();
            let rel = entry_path.strip_prefix(path).unwrap_or(path);
            if !rel.as_os_str().is_empty() {
                tree.ensure_dir_path(rel);
            }
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
                progress.record(1, meta.len(), current_path);
            } else {
                progress.record_files_only(1);
            }
        }

        let node = create_file_node(&entry_path, &meta);
        tree.insert_file(&entry_path, path, node);
    }

    compute_size(&mut tree.arena);
    let total_size = tree.arena[ROOT_NODE as usize].size;
    let snapshot = Snapshot::new(path.to_path_buf(), tree.arena, total_size);

    progress.finish();

    Ok(snapshot)
}

fn create_file_node(path: &Path, meta: &std::fs::Metadata) -> FileNode {
    let is_dir = meta.is_dir();
    let file_type = if is_dir {
        FileType::Directory
    } else if meta.is_symlink() {
        FileType::Symlink
    } else {
        detect_file_type(meta)
    };

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
        parent: None,
        is_dir,
        file_type,
        size,
        children: Vec::new(),
    }
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

    let mut arena = vec![FileNode {
        name,
        parent: None,
        is_dir: true,
        file_type: FileType::Directory,
        size: 0,
        children: Vec::new(),
    }];

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let meta = match std::fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mut node = create_file_node(&entry.path(), &meta);
        node.parent = Some(ROOT_NODE);
        let name = entry.file_name().to_string_lossy().to_string();
        let new_idx = arena.len() as NodeIndex;
        arena.push(node);
        arena[ROOT_NODE as usize].children.push((name, new_idx));
    }

    let total_size = arena[ROOT_NODE as usize]
        .children
        .iter()
        .map(|(_, idx)| arena[*idx as usize].size)
        .sum();

    Ok(Snapshot::new(path.to_path_buf(), arena, total_size))
}

fn compute_size(arena: &mut [FileNode]) {
    for i in (1..arena.len()).rev() {
        let size = arena[i].size;
        if let Some(parent) = arena[i].parent {
            arena[parent as usize].size = arena[parent as usize].size.saturating_add(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    fn child<'a>(snap: &'a Snapshot, parent: NodeIndex, name: &str) -> &'a FileNode {
        let idx = snap.node(parent).child_idx(name).unwrap();
        snap.node(idx)
    }

    fn child_idx(snap: &Snapshot, parent: NodeIndex, name: &str) -> NodeIndex {
        snap.node(parent).child_idx(name).unwrap()
    }

    #[test]
    fn test_scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let cancel = AtomicBool::new(false);
        let result = scan_path(dir.path(), &cancel, None);
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        assert!(snapshot.node(ROOT_NODE).is_dir);
        assert!(snapshot.node(ROOT_NODE).children.is_empty());
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
        let node = child(&snapshot, ROOT_NODE, "test.txt");
        assert!(!node.is_dir);
        assert_eq!(node.size, 11);
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
        let a = child(&snapshot, ROOT_NODE, "a");
        let b = child(&snapshot, a_idx, "b");
        let c = child(&snapshot, b_idx, "c");
        let file = child(&snapshot, c_idx, "file.txt");
        assert_eq!(file.size, 7);
        assert_eq!(c.size, 7);
        assert!(b.size >= 7);
        assert!(a.size >= 7);
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
        let mut arena = vec![
            FileNode {
                name: "parent".into(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 0,
                children: Vec::new(),
            },
            FileNode {
                name: "child1".into(),
                parent: Some(0),
                is_dir: false,
                file_type: FileType::File,
                size: 100,
                children: Vec::new(),
            },
            FileNode {
                name: "child2".into(),
                parent: Some(0),
                is_dir: false,
                file_type: FileType::File,
                size: 200,
                children: Vec::new(),
            },
        ];
        arena[0].children.push(("child1".into(), 1));
        arena[0].children.push(("child2".into(), 2));

        compute_size(&mut arena);
        assert_eq!(arena[0].size, 300);
        assert_eq!(arena[0].size, 300);
    }

    #[test]
    fn test_list_dir_empty_directory() {
        let dir = TempDir::new().unwrap();
        let result = list_dir(dir.path());
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        assert!(snapshot.node(ROOT_NODE).is_dir);
        assert!(snapshot.node(ROOT_NODE).children.is_empty());
    }

    #[test]
    fn test_list_dir_with_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::write(dir.path().join("b.txt"), "world").unwrap();

        let result = list_dir(dir.path());
        assert!(result.is_ok());
        let snapshot = result.unwrap();
        let a = child(&snapshot, ROOT_NODE, "a.txt");
        assert!(!a.is_dir);
        assert_eq!(a.size, 5);
        let b = child(&snapshot, ROOT_NODE, "b.txt");
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
        let snapshot = result.unwrap();
        let sub = child(&snapshot, ROOT_NODE, "sub");
        assert!(sub.is_dir);
        assert_eq!(sub.size, 0);
        assert!(sub.children.is_empty());
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
        let linked = child(&snapshot, ROOT_NODE, "linked.txt");
        assert_eq!(linked.file_type, FileType::Symlink);
        assert!(!linked.is_dir);
        assert_eq!(linked.size, 0);
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
        let node = child(&snapshot, ROOT_NODE, "linked.txt");
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
    fn test_find_or_create_child_many_siblings() {
        let mut tree = TreeBuilder::new("root");
        let n = 200;
        for i in 0..n {
            let name = format!("file_{i:03}");
            let idx = tree.find_or_create_child(ROOT_NODE, &name);
            assert_eq!(tree.arena[idx as usize].name, name);
        }
        assert_eq!(tree.arena[ROOT_NODE as usize].children.len(), n);
        // Idempotent: same names resolve to existing nodes
        for i in 0..n {
            let name = format!("file_{i:03}");
            let idx = tree.find_or_create_child(ROOT_NODE, &name);
            assert_eq!(tree.arena[idx as usize].name, name);
        }
        assert_eq!(tree.arena[ROOT_NODE as usize].children.len(), n);
    }

    #[test]
    fn test_scan_empty_subdirectory_included_in_arena() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("empty_sub")).unwrap();
        fs::write(dir.path().join("file.txt"), "data").unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();

        let root = snapshot.node(ROOT_NODE);
        assert!(root.is_dir);
        assert_eq!(root.children.len(), 2);

        let empty = child(&snapshot, ROOT_NODE, "empty_sub");
        assert!(empty.is_dir);
        assert_eq!(empty.size, 0);
        assert!(empty.children.is_empty());

        let file = child(&snapshot, ROOT_NODE, "file.txt");
        assert!(!file.is_dir);
        assert_eq!(file.size, 4);

        assert_eq!(root.size, 4);
    }

    #[test]
    fn test_scan_deeply_nested_empty_directories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c")).unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();

        let root = snapshot.node(ROOT_NODE);
        assert!(root.is_dir);
        assert_eq!(root.size, 0);
        assert_eq!(root.children.len(), 1);

        let a_idx = child_idx(&snapshot, ROOT_NODE, "a");
        let a = snapshot.node(a_idx);
        assert!(a.is_dir);
        assert_eq!(a.size, 0);
        assert_eq!(a.children.len(), 1);

        let b_idx = child_idx(&snapshot, a_idx, "b");
        let b = snapshot.node(b_idx);
        assert!(b.is_dir);
        assert_eq!(b.size, 0);
        assert_eq!(b.children.len(), 1);

        let c_idx = child_idx(&snapshot, b_idx, "c");
        let c = snapshot.node(c_idx);
        assert!(c.is_dir);
        assert_eq!(c.size, 0);
        assert!(c.children.is_empty());
    }

    #[test]
    fn test_scan_mixed_empty_and_nonempty_subdirectories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("empty")).unwrap();
        fs::create_dir_all(dir.path().join("full/sub")).unwrap();
        fs::write(dir.path().join("full/sub/file.txt"), "data").unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None).unwrap();

        let root = snapshot.node(ROOT_NODE);
        assert_eq!(root.children.len(), 2);

        let empty = child(&snapshot, ROOT_NODE, "empty");
        assert!(empty.is_dir);
        assert_eq!(empty.size, 0);
        assert!(empty.children.is_empty());

        let full_idx = child_idx(&snapshot, ROOT_NODE, "full");
        let full = snapshot.node(full_idx);
        assert!(full.is_dir);
        assert_eq!(full.size, 4);

        let sub = child(&snapshot, full_idx, "sub");
        assert!(sub.is_dir);
        assert_eq!(sub.size, 4);

        let file = snapshot.node(sub.child_idx("file.txt").unwrap());
        assert!(!file.is_dir);
        assert_eq!(file.size, 4);

        assert_eq!(root.size, 4);
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
            .node(ROOT_NODE)
            .children
            .iter()
            .map(|(n, _)| n.as_str())
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

        let root = snapshot.node(ROOT_NODE);
        assert_eq!(root.children.len(), 3);
        let mut names: Vec<&str> = root.children.iter().map(|(n, _)| n.as_str()).collect();
        names.sort();
        assert_eq!(names, vec![".dotfile.txt", ".hidden", "visible.txt"]);
        assert_eq!(snapshot.total_files, 3);
    }
}
