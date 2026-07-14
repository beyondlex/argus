use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use ignore::WalkBuilder;

use crate::model::{FileNode, FileType, NodeIndex, ScanError, Snapshot, ROOT_NODE};

// P9: skipped aggressive `WalkBuilder::build_parallel` — arena inserts and
// hardlink `seen_inodes` are sequential; parallel path collect would need a
// larger rewrite. Instead: reuse DirEntry metadata when available, and sort
// children by name before snapshot finalize for deterministic order.

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub file_count: u64,
    pub total_bytes: u64,
}

struct ProgressTracker {
    file_count: u64,
    total_bytes: u64,
    last_reported_file_count: u64,
    last_reported_total_bytes: u64,
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
            progress_tx,
        }
    }

    fn record(&mut self, files: u64, bytes: u64) {
        self.file_count = self.file_count.saturating_add(files);
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.maybe_report();
    }

    fn record_files_only(&mut self, files: u64) {
        self.record(files, 0);
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
                });
            }
        }
    }

    fn finish(&mut self) {
        if let Some(ref tx) = self.progress_tx {
            let _ = tx.send(ProgressUpdate {
                file_count: self.file_count,
                total_bytes: self.total_bytes,
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
        Self {
            arena: vec![FileNode {
                name: root_name.to_string(),
                parent: None,
                is_dir: true,
                file_type: FileType::Directory,
                size: 0,
                children: Vec::new(),
            }],
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

    fn insert_file(&mut self, path: &Path, root_path: &Path, node: FileNode) {
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
        let name = components.last().unwrap().as_os_str().to_string_lossy();
        let new_idx = self.arena.len() as NodeIndex;
        self.arena.push(node);
        self.arena[parent as usize]
            .children
            .push((name.to_string(), new_idx));
        self.child_lookup
            .entry(parent)
            .or_default()
            .insert(name.to_string(), new_idx);
    }

    fn ensure_path(&mut self, path: &Path, root_path: &Path) -> NodeIndex {
        let rel = path.strip_prefix(root_path).unwrap_or(path);
        let components: Vec<_> = rel.components().collect();
        let mut idx = ROOT_NODE;
        for comp in components.iter() {
            let name = comp.as_os_str().to_string_lossy();
            idx = self.find_or_create_child(idx, &name);
        }
        idx
    }
}

/// One FS walk for a skipped directory: skeleton dirs+files, hardlink-deduped
/// size rollup, and progress updates (replaces walk_dir_size + skeleton_walk +
/// patch_skeleton_sizes).
fn walk_skip_dir(
    arena: &mut Vec<FileNode>,
    parent_idx: NodeIndex,
    path: &Path,
    cancel: &AtomicBool,
    seen_inodes: &mut HashSet<(u64, u64)>,
    progress: &mut ProgressTracker,
) -> u64 {
    if cancel.load(Ordering::Relaxed) {
        return 0;
    }

    let Ok(read_dir) = std::fs::read_dir(path) else {
        arena[parent_idx as usize].size = 0;
        return 0;
    };

    let mut total = 0u64;
    for entry in read_dir.flatten() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        let entry_path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        if is_dir {
            let new_idx = arena.len() as NodeIndex;
            arena.push(FileNode {
                name: name.clone(),
                parent: Some(parent_idx),
                is_dir: true,
                file_type: FileType::Directory,
                size: 0,
                children: Vec::new(),
            });
            arena[parent_idx as usize].children.push((name, new_idx));
            let sub = walk_skip_dir(arena, new_idx, &entry_path, cancel, seen_inodes, progress);
            total = total.saturating_add(sub);
            continue;
        }

        let file_type = dir_entry_file_type(&entry);
        let meta = std::fs::symlink_metadata(&entry_path).ok();
        let (size, contribute) = match &meta {
            Some(m) if m.is_file() => {
                let mut contribute = true;
                if let (Ok(device), Ok(inode)) = (get_device(m), get_inode(m)) {
                    if !seen_inodes.insert((device, inode)) {
                        contribute = false;
                    }
                }
                (m.len(), contribute)
            }
            Some(m) if m.is_symlink() => (0, false),
            Some(m) => (m.len(), false),
            None => (0, false),
        };

        let new_idx = arena.len() as NodeIndex;
        arena.push(FileNode {
            name: name.clone(),
            parent: Some(parent_idx),
            is_dir: false,
            file_type,
            size,
            children: Vec::new(),
        });
        arena[parent_idx as usize].children.push((name, new_idx));

        if contribute {
            total = total.saturating_add(size);
            progress.record(1, size);
        }
    }

    arena[parent_idx as usize].size = total;
    total
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

    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut progress = ProgressTracker::new(progress_tx);

    let skipped = Arc::new(Mutex::new(Vec::new()));
    let skipped_clone = skipped.clone();
    let skip_patterns: Vec<String> = skip_dirs.to_vec();

    let walker = WalkBuilder::new(path)
        .follow_links(false)
        .git_ignore(false)
        .hidden(false)
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

    let root_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let mut tree = TreeBuilder::new(&root_name);

    for result in walker {
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }

        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        if is_dir {
            continue;
        }

        // Prefer walk-cached metadata (follow_links=false → symlink meta) to
        // avoid a redundant symlink_metadata syscall per file.
        let meta = match entry
            .metadata()
            .or_else(|_| std::fs::symlink_metadata(entry.path()))
        {
            Ok(m) => m,
            Err(_) => continue,
        };

        if meta.is_file() || meta.is_symlink() {
            if let (Ok(device), Ok(inode)) = (get_device(&meta), get_inode(&meta)) {
                if !seen_inodes.insert((device, inode)) {
                    continue;
                }
            }
            if meta.is_file() {
                progress.record(1, meta.len());
            } else {
                progress.record_files_only(1);
            }
        }

        let node = create_file_node(entry.path(), &meta);
        tree.insert_file(entry.path(), path, node);
    }

    for skip_path in skipped.lock().unwrap().drain(..) {
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }

        let parent_idx = tree.ensure_path(&skip_path, path);
        walk_skip_dir(
            &mut tree.arena,
            parent_idx,
            &skip_path,
            cancel,
            &mut seen_inodes,
            &mut progress,
        );
    }

    compute_size(&mut tree.arena, ROOT_NODE);
    sort_children_by_name(&mut tree.arena);
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

fn dir_entry_file_type(entry: &std::fs::DirEntry) -> FileType {
    match entry.file_type() {
        Ok(ft) => {
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
                FileType::Symlink
            } else if ft.is_dir() {
                FileType::Directory
            } else {
                FileType::File
            }
        }
        Err(_) => FileType::File,
    }
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

fn compute_size(arena: &mut [FileNode], idx: NodeIndex) -> u64 {
    if arena[idx as usize].children.is_empty() {
        return arena[idx as usize].size;
    }

    // Nodes already sized by walk_skip_dir: don't recompute
    if arena[idx as usize].size > 0 {
        return arena[idx as usize].size;
    }

    let child_ids: Vec<NodeIndex> = arena[idx as usize]
        .children
        .iter()
        .map(|(_, idx)| *idx)
        .collect();
    let mut total = 0u64;
    for &child_idx in &child_ids {
        total = total.saturating_add(compute_size(arena, child_idx));
    }
    arena[idx as usize].size = total;
    total
}

fn sort_children_by_name(arena: &mut [FileNode]) {
    for node in arena.iter_mut() {
        node.children.sort_by(|a, b| a.0.cmp(&b.0));
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
        let result = scan_path(dir.path(), &cancel, None, &[]);
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
        let result = scan_path(dir.path(), &cancel, None, &[]);
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
        let result = scan_path(dir.path(), &cancel, None, &[]);
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

        let total = compute_size(&mut arena, ROOT_NODE);
        assert_eq!(total, 300);
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
        let snapshot = scan_path(dir.path(), &cancel, None, &[]).unwrap();
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
    fn test_walk_skip_dir_counts_nested_files() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir_all(sub.join("deep")).unwrap();
        fs::write(sub.join("a.txt"), "hello").unwrap();
        fs::write(sub.join("deep").join("b.txt"), "world!").unwrap();

        let cancel = AtomicBool::new(false);
        let mut seen = HashSet::new();
        let mut progress = ProgressTracker::new(None);
        let mut arena = vec![FileNode {
            name: "sub".into(),
            parent: None,
            is_dir: true,
            file_type: FileType::Directory,
            size: 0,
            children: Vec::new(),
        }];
        let size = walk_skip_dir(
            &mut arena,
            ROOT_NODE,
            &sub,
            &cancel,
            &mut seen,
            &mut progress,
        );
        assert_eq!(size, 11);
        assert_eq!(arena[ROOT_NODE as usize].size, 11);
        // skeleton includes dirs + files
        assert!(arena.len() >= 4);
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
    fn test_scan_children_sorted_by_name() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("z.txt"), "z").unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("m.txt"), "m").unwrap();

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(dir.path(), &cancel, None, &[]).unwrap();
        let names: Vec<&str> = snapshot
            .node(ROOT_NODE)
            .children
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(names, vec!["a.txt", "m.txt", "z.txt"]);
    }

    #[test]
    fn test_scan_progress_does_not_double_count_skipped_dir_children() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("readme.md"), "project").unwrap();

        let nm = dir.path().join("node_modules");
        fs::create_dir_all(nm.join("pkg/src")).unwrap();
        fs::write(nm.join("pkg").join("index.js"), "module.exports = 1").unwrap();
        fs::write(nm.join("pkg/src").join("lib.js"), "function f() {}").unwrap();

        let cancel = AtomicBool::new(false);
        let (tx, rx) = std::sync::mpsc::channel();
        let skip = vec!["node_modules".to_string()];
        let snapshot = scan_path(dir.path(), &cancel, Some(tx), &skip).unwrap();

        let final_update = rx.try_iter().last().unwrap();
        assert_eq!(final_update.total_bytes, snapshot.total_size);
        assert_eq!(final_update.total_bytes, 7 + 18 + 15);
        assert_eq!(final_update.file_count, 3);
    }

    #[test]
    fn test_scan_with_skip_dir_includes_full_size() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("readme.md"), "project").unwrap();
        let nm = dir.path().join("node_modules");
        fs::create_dir_all(nm.join("pkg/src")).unwrap();
        fs::write(nm.join("pkg").join("index.js"), "module.exports = 1").unwrap();
        fs::write(nm.join("pkg/src").join("lib.js"), "function f() {}").unwrap();

        let cancel = AtomicBool::new(false);
        let skip = vec!["node_modules".to_string()];
        let snapshot = scan_path(dir.path(), &cancel, None, &skip).unwrap();

        assert_eq!(snapshot.total_size, 7 + 18 + 15);

        let nm_idx = child_idx(&snapshot, ROOT_NODE, "node_modules");
        let pkg_idx = child_idx(&snapshot, nm_idx, "pkg");
        let nm_node = child(&snapshot, ROOT_NODE, "node_modules");
        assert!(nm_node.is_dir);
        assert!(!nm_node.children.is_empty());

        let pkg = child(&snapshot, nm_idx, "pkg");
        assert_eq!(pkg.size, 18 + 15);

        let _src = child(&snapshot, pkg_idx, "src");
        assert_eq!(nm_node.size, 18 + 15);
    }

    #[test]
    fn test_scan_skip_gitignored_dirs() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let tg = dir.path().join("target");
        fs::create_dir_all(tg.join("debug")).unwrap();
        fs::write(tg.join("debug").join("a.o"), "AAAA").unwrap();
        fs::write(tg.join("b.o"), "BBBBBB").unwrap();

        let cancel = AtomicBool::new(false);
        let skip = vec!["target".to_string()];
        let snapshot = scan_path(dir.path(), &cancel, None, &skip).unwrap();

        assert_eq!(snapshot.total_size, 8 + 12 + 4 + 6);
        assert_eq!(snapshot.node(ROOT_NODE).size, snapshot.total_size);

        let tg_node = child(&snapshot, ROOT_NODE, "target");
        assert!(tg_node.is_dir);
        assert_eq!(tg_node.size, 4 + 6);
    }
}
