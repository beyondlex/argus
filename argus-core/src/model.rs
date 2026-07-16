use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type NodeIndex = u32;

pub const ROOT_NODE: NodeIndex = 0;
pub const NO_PARENT: NodeIndex = NodeIndex::MAX;

/// Compact snapshot format version (bincode + optional gzip). No legacy JSON.
pub const SNAPSHOT_VERSION: u32 = 4;

/// Names with length ≤ this are stored inline in `FileNode::inline_name` (Phase 4).
pub const INLINE_NAME_MAX: usize = 12;

const FLAG_DIR: u16 = 1 << 0;
const FLAG_INLINE_NAME: u16 = 1 << 1;
const FLAG_TYPE_SHIFT: u16 = 2;
const FLAG_TYPE_MASK: u16 = 0b111 << FLAG_TYPE_SHIFT;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    File = 0,
    Directory = 1,
    Symlink = 2,
    Fifo = 3,
    Socket = 4,
    Device = 5,
    Other = 6,
}

impl FileType {
    fn from_bits(bits: u16) -> Self {
        match bits {
            0 => FileType::File,
            1 => FileType::Directory,
            2 => FileType::Symlink,
            3 => FileType::Fifo,
            4 => FileType::Socket,
            5 => FileType::Device,
            _ => FileType::Other,
        }
    }

    fn to_bits(self) -> u16 {
        match self {
            FileType::File => 0,
            FileType::Directory => 1,
            FileType::Symlink => 2,
            FileType::Fifo => 3,
            FileType::Socket => 4,
            FileType::Device => 5,
            FileType::Other => 6,
        }
    }
}

/// Fixed-width tree node. Names live in `Snapshot::names` or `inline_name`.
///
/// Layout (40 bytes, `repr(C)`): sizes first so no padding before `u64` fields.
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[repr(C)]
pub struct FileNode {
    pub size: u64,
    pub disk_usage: u64,
    pub parent: NodeIndex,
    /// Byte offset into `Snapshot::names` when not inline.
    pub name_off: u32,
    pub name_len: u16,
    pub flags: u16,
    /// Inline short name bytes (valid when `FLAG_INLINE_NAME` is set).
    pub inline_name: [u8; INLINE_NAME_MAX],
}

impl FileNode {
    pub fn is_dir(&self) -> bool {
        self.flags & FLAG_DIR != 0
    }

    pub fn file_type(&self) -> FileType {
        FileType::from_bits((self.flags & FLAG_TYPE_MASK) >> FLAG_TYPE_SHIFT)
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn set_size(&mut self, s: u64) {
        self.size = s;
    }

    pub fn disk_usage(&self) -> u64 {
        self.disk_usage
    }

    pub fn set_disk_usage(&mut self, d: u64) {
        self.disk_usage = d;
    }

    pub fn parent(&self) -> Option<NodeIndex> {
        if self.parent == NO_PARENT {
            None
        } else {
            Some(self.parent)
        }
    }

    pub fn set_parent(&mut self, p: Option<NodeIndex>) {
        self.parent = p.unwrap_or(NO_PARENT);
    }

    pub fn has_inline_name(&self) -> bool {
        self.flags & FLAG_INLINE_NAME != 0
    }
}

fn pack_flags(is_dir: bool, file_type: FileType, inline_name: bool) -> u16 {
    let mut f = file_type.to_bits() << FLAG_TYPE_SHIFT;
    if is_dir {
        f |= FLAG_DIR;
    }
    if inline_name {
        f |= FLAG_INLINE_NAME;
    }
    f
}

/// In-memory file tree: compact nodes + name blob + CSR child edges.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Snapshot {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub root_path: PathBuf,
    pub root_path_hash: String,
    pub total_size: u64,
    #[serde(default)]
    pub total_disk_usage: u64,
    #[serde(default)]
    pub total_files: u64,
    /// Compact node table (index = NodeIndex).
    pub nodes: Vec<FileNode>,
    /// Packed UTF-8 (lossy) names for non-inline nodes.
    pub names: Vec<u8>,
    /// Flat child indices (CSR column indices).
    pub child_index: Vec<NodeIndex>,
    /// `child_index` start offset for each node.
    pub child_start: Vec<u32>,
    /// Number of children for each node (0 for files).
    pub child_count: Vec<u32>,
}

impl Snapshot {
    /// Build from a finished [`SnapshotBuilder`].
    pub fn from_builder(
        root_path: PathBuf,
        builder: SnapshotBuilder,
        total_size: u64,
        total_disk_usage: u64,
    ) -> Self {
        builder.finish(root_path, total_size, total_disk_usage)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn root(&self) -> &FileNode {
        &self.nodes[ROOT_NODE as usize]
    }

    pub fn node(&self, idx: NodeIndex) -> &FileNode {
        &self.nodes[idx as usize]
    }

    pub fn node_mut(&mut self, idx: NodeIndex) -> &mut FileNode {
        &mut self.nodes[idx as usize]
    }

    pub fn root_mut(&mut self) -> &mut FileNode {
        &mut self.nodes[ROOT_NODE as usize]
    }

    /// Name of node `idx` (zero-copy into blob or inline buffer).
    pub fn name(&self, idx: NodeIndex) -> &str {
        let node = &self.nodes[idx as usize];
        if node.has_inline_name() {
            let len = node.name_len as usize;
            std::str::from_utf8(&node.inline_name[..len.min(INLINE_NAME_MAX)]).unwrap_or("")
        } else {
            let start = node.name_off as usize;
            let end = start + node.name_len as usize;
            std::str::from_utf8(self.names.get(start..end).unwrap_or(&[])).unwrap_or("")
        }
    }

    pub fn children(&self, idx: NodeIndex) -> &[NodeIndex] {
        let i = idx as usize;
        if i >= self.child_start.len() {
            return &[];
        }
        let start = self.child_start[i] as usize;
        let count = self.child_count[i] as usize;
        let end = start.saturating_add(count);
        if end > self.child_index.len() {
            return &[];
        }
        &self.child_index[start..end]
    }

    pub fn children_len(&self, idx: NodeIndex) -> usize {
        self.child_count.get(idx as usize).copied().unwrap_or(0) as usize
    }

    pub fn children_is_empty(&self, idx: NodeIndex) -> bool {
        self.children_len(idx) == 0
    }

    pub fn children_clone(&self, idx: NodeIndex) -> Vec<NodeIndex> {
        self.children(idx).to_vec()
    }

    /// Look up child index by name (linear scan over CSR range).
    pub fn child_idx(&self, parent: NodeIndex, name: &str) -> Option<NodeIndex> {
        self.children(parent)
            .iter()
            .find(|&&idx| self.name(idx) == name)
            .copied()
    }

    /// Sort children of `parent` by name (in-place within CSR range).
    pub fn sort_children(&mut self, parent: NodeIndex) {
        let start = self.child_start[parent as usize] as usize;
        let count = self.child_count[parent as usize] as usize;
        if count <= 1 {
            return;
        }
        let end = start + count;
        let mut pairs: Vec<(NodeIndex, String)> = self.child_index[start..end]
            .iter()
            .map(|&idx| (idx, self.name(idx).to_string()))
            .collect();
        pairs.sort_by(|a, b| a.1.cmp(&b.1));
        for (i, (idx, _)) in pairs.into_iter().enumerate() {
            self.child_index[start + i] = idx;
        }
    }

    /// Remove child at position `pos` within parent's CSR range (swap-remove).
    pub fn swap_remove_child(&mut self, parent: NodeIndex, pos: usize) -> Option<NodeIndex> {
        let start = self.child_start[parent as usize] as usize;
        let count = self.child_count[parent as usize] as usize;
        if pos >= count {
            return None;
        }
        let last = start + count - 1;
        let removed = self.child_index[start + pos];
        self.child_index[start + pos] = self.child_index[last];
        self.child_count[parent as usize] = (count - 1) as u32;
        Some(removed)
    }

    /// Append a child index to parent. Relocates the parent's range to the end if needed.
    pub fn push_child(&mut self, parent: NodeIndex, child: NodeIndex) {
        let p = parent as usize;
        let start = self.child_start[p] as usize;
        let count = self.child_count[p] as usize;
        let end = start + count;
        if end == self.child_index.len() {
            self.child_index.push(child);
            self.child_count[p] = (count + 1) as u32;
            return;
        }
        // Relocate range to end, then push.
        let slice: Vec<NodeIndex> = self.child_index[start..end].to_vec();
        let new_start = self.child_index.len() as u32;
        self.child_index.extend(slice);
        self.child_index.push(child);
        self.child_start[p] = new_start;
        self.child_count[p] = (count + 1) as u32;
    }

    /// Replace parent's children with `new_children` (appended at end of `child_index`).
    pub fn set_children(&mut self, parent: NodeIndex, new_children: &[NodeIndex]) {
        let p = parent as usize;
        let new_start = self.child_index.len() as u32;
        self.child_index.extend_from_slice(new_children);
        self.child_start[p] = new_start;
        self.child_count[p] = new_children.len() as u32;
    }

    /// Encode a name into this snapshot (inline if short).
    pub fn alloc_name(&mut self, name: &str) -> (u32, u16, u16, [u8; INLINE_NAME_MAX]) {
        alloc_name_into(&mut self.names, name)
    }

    /// Append a new node. Extends CSR tables with empty children.
    pub fn push_node(&mut self, node: FileNode) -> NodeIndex {
        let idx = self.nodes.len() as NodeIndex;
        self.nodes.push(node);
        self.child_start.push(0);
        self.child_count.push(0);
        idx
    }

    /// Copy a node from `other` (re-encoding its name into this snapshot).
    pub fn clone_node_from(
        &mut self,
        other: &Snapshot,
        idx: NodeIndex,
        parent: NodeIndex,
    ) -> NodeIndex {
        let src = other.node(idx);
        let name = other.name(idx);
        let (off, len, name_flags, inline) = self.alloc_name(name);
        let flags =
            pack_flags(src.is_dir(), src.file_type(), false) | (name_flags & FLAG_INLINE_NAME);
        let node = FileNode {
            size: src.size,
            disk_usage: src.disk_usage,
            parent,
            name_off: off,
            name_len: len,
            flags,
            inline_name: inline,
        };
        self.push_node(node)
    }

    /// Graft all direct children of `listed`'s root under `target` in this snapshot.
    pub fn graft_children_from(&mut self, target: NodeIndex, listed: &Snapshot) {
        for &child_idx in listed.children(ROOT_NODE) {
            let new_idx = self.clone_node_from(listed, child_idx, target);
            self.push_child(target, new_idx);
        }
    }

    /// Serialize with bincode + gzip.
    pub fn to_compact_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        let raw = bincode::serialize(self)
            .map_err(|e| SnapshotError::Corrupted(format!("bincode serialize: {e}")))?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw)?;
        encoder
            .finish()
            .map_err(|e| SnapshotError::Corrupted(format!("compression failed: {e}")))
    }

    /// Deserialize bincode (optionally gzip-wrapped). Rejects wrong version.
    pub fn from_bytes(data: &[u8]) -> Result<Self, SnapshotError> {
        let raw: Vec<u8> = if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
            let mut decoder = GzDecoder::new(data);
            let mut buf = Vec::new();
            decoder.read_to_end(&mut buf)?;
            buf
        } else {
            data.to_vec()
        };
        let snap: Self = bincode::deserialize(&raw)
            .map_err(|e| SnapshotError::Corrupted(format!("bincode deserialize: {e}")))?;
        if snap.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::VersionMismatch {
                expected: SNAPSHOT_VERSION,
                actual: snap.version,
            });
        }
        Ok(snap)
    }

    /// Walk along path components starting at `idx` (first component must match node name).
    pub fn find_node(&self, idx: NodeIndex, target_path: &[String]) -> Option<NodeIndex> {
        let (head, tail) = target_path.split_first()?;
        if self.name(idx) != *head {
            return None;
        }
        if tail.is_empty() {
            return Some(idx);
        }
        let child_idx = self.child_idx(idx, &tail[0])?;
        self.find_node(child_idx, tail)
    }
}

/// Encode `name` into `names` blob or inline buffer.
/// Returns `(name_off, name_len, flags_with_inline_bit, inline_name)`.
pub fn alloc_name_into(names: &mut Vec<u8>, name: &str) -> (u32, u16, u16, [u8; INLINE_NAME_MAX]) {
    let bytes = name.as_bytes();
    let len = bytes.len().min(u16::MAX as usize) as u16;
    let mut inline = [0u8; INLINE_NAME_MAX];
    if bytes.len() <= INLINE_NAME_MAX {
        inline[..bytes.len()].copy_from_slice(bytes);
        return (0, len, FLAG_INLINE_NAME, inline);
    }
    let off = names.len() as u32;
    names.extend_from_slice(bytes);
    (off, len, 0, inline)
}

/// Incremental builder used by scanner and tests.
/// Children stored as flat `(parent, child)` pairs, converted to CSR on finish.
/// Avoids `Vec<Vec<NodeIndex>>` overhead (2.4M empty Vecs × 24B = ~57 MB peak saved).
pub struct SnapshotBuilder {
    pub nodes: Vec<FileNode>,
    pub names: Vec<u8>,
    child_pairs: Vec<(u32, u32)>,
}

impl SnapshotBuilder {
    pub fn new(root_name: &str) -> Self {
        let mut names = Vec::new();
        let (off, len, name_flags, inline) = alloc_name_into(&mut names, root_name);
        let flags = pack_flags(true, FileType::Directory, false) | (name_flags & FLAG_INLINE_NAME);
        let root = FileNode {
            size: 0,
            disk_usage: 0,
            parent: NO_PARENT,
            name_off: off,
            name_len: len,
            flags,
            inline_name: inline,
        };
        Self {
            nodes: vec![root],
            names,
            child_pairs: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn name(&self, idx: NodeIndex) -> &str {
        let node = &self.nodes[idx as usize];
        if node.has_inline_name() {
            let len = node.name_len as usize;
            std::str::from_utf8(&node.inline_name[..len.min(INLINE_NAME_MAX)]).unwrap_or("")
        } else {
            let start = node.name_off as usize;
            let end = start + node.name_len as usize;
            std::str::from_utf8(self.names.get(start..end).unwrap_or(&[])).unwrap_or("")
        }
    }

    pub fn push_dir(&mut self, parent: NodeIndex, name: &str) -> NodeIndex {
        self.push_node(parent, name, true, FileType::Directory, 0, 0)
    }

    pub fn push_file(
        &mut self,
        parent: NodeIndex,
        name: &str,
        file_type: FileType,
        size: u64,
        disk_usage: u64,
    ) -> NodeIndex {
        self.push_node(parent, name, false, file_type, size, disk_usage)
    }

    fn push_node(
        &mut self,
        parent: NodeIndex,
        name: &str,
        is_dir: bool,
        file_type: FileType,
        size: u64,
        disk_usage: u64,
    ) -> NodeIndex {
        let (off, len, name_flags, inline) = alloc_name_into(&mut self.names, name);
        let flags = pack_flags(is_dir, file_type, false) | (name_flags & FLAG_INLINE_NAME);
        let idx = self.nodes.len() as NodeIndex;
        self.nodes.push(FileNode {
            size,
            disk_usage,
            parent,
            name_off: off,
            name_len: len,
            flags,
            inline_name: inline,
        });
        self.child_pairs.push((parent, idx));
        idx
    }

    pub fn finish(
        mut self,
        root_path: PathBuf,
        total_size: u64,
        total_disk_usage: u64,
    ) -> Snapshot {
        let n = self.nodes.len();

        let mut child_count = vec![0u32; n];
        for &(parent, _) in &self.child_pairs {
            child_count[parent as usize] += 1;
        }

        let mut child_start = Vec::with_capacity(n);
        let mut total_children = 0u32;
        for &count in &child_count {
            child_start.push(total_children);
            total_children += count;
        }

        let mut child_index = vec![0u32; total_children as usize];
        let mut write_pos = child_start.clone();
        for &(parent, child) in &self.child_pairs {
            let p = parent as usize;
            child_index[write_pos[p] as usize] = child;
            write_pos[p] += 1;
        }

        drop(self.child_pairs);
        self.nodes.shrink_to_fit();
        self.names.shrink_to_fit();
        child_index.shrink_to_fit();

        let total_files = self.nodes.iter().filter(|n| !n.is_dir()).count() as u64;
        Snapshot {
            version: SNAPSHOT_VERSION,
            timestamp: Utc::now(),
            root_path_hash: hash_root_path(&root_path),
            root_path,
            total_size,
            total_disk_usage,
            total_files,
            nodes: self.nodes,
            names: self.names,
            child_index,
            child_start,
            child_count,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ScanError {
    #[error("path not found: {0}")]
    PathNotFound(PathBuf),

    #[error("permission denied: {0}")]
    PermissionDenied(PathBuf),

    #[error("scan cancelled by user")]
    Cancelled,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum SnapshotError {
    #[error("snapshot version mismatch: expected v{expected}, got v{actual}")]
    VersionMismatch { expected: u32, actual: u32 },

    #[error("snapshot file corrupted: {0}")]
    Corrupted(String),

    #[error("snapshot file not found: {0}")]
    NotFound(PathBuf),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum ParseSizeError {
    #[error("invalid size format: {0}")]
    InvalidFormat(String),

    #[error("numeric overflow")]
    Overflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaEvent {
    pub path: PathBuf,
    pub delta_size: i64,
    pub event_type: String,
    pub timestamp: u64,
    pub is_agg: bool,
    pub process_info: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaEntry {
    pub path: PathBuf,
    pub delta_size: i64,
    pub event_type: String,
    pub timestamp: u64,
    pub is_agg: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeltaSummary {
    pub event_count: u64,
    pub create_count: u64,
    pub modify_count: u64,
    pub delete_count: u64,
    pub agg_count: u64,
    pub positive_events: u64,
    pub negative_events: u64,
    pub zero_events: u64,
    pub total_delta: i64,
    pub positive_delta: i64,
    pub negative_delta: i64,
}

pub fn hash_root_path(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let result = hasher.finalize();
    hex_encode(&result[..4])
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn parse_human_size(input: &str) -> Result<u64, ParseSizeError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(ParseSizeError::InvalidFormat("empty string".into()));
    }

    let (num_str, unit) = split_number_unit(input);
    let value: f64 = num_str
        .parse()
        .map_err(|_| ParseSizeError::InvalidFormat(input.to_string()))?;

    let multiplier = match unit {
        "B" | "" => 1u64,
        "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        "TB" => 1024u64 * 1024 * 1024 * 1024,
        _ => return Err(ParseSizeError::InvalidFormat(input.to_string())),
    };

    Ok((value * multiplier as f64) as u64)
}

fn split_number_unit(s: &str) -> (&str, &str) {
    let unit_start = s.find(|c: char| !(c.is_ascii_digit() || c == '.'));
    match unit_start {
        Some(idx) => {
            let num_part = &s[..idx];
            let unit_part = &s[idx..];
            if num_part.is_empty() {
                ("0", unit_part)
            } else {
                (num_part, unit_part)
            }
        }
        None => (s, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_root_path() {
        let path = PathBuf::from("/home/user");
        let hash = hash_root_path(&path);
        assert_eq!(hash.len(), 8);
    }

    #[test]
    fn test_parse_human_size_bytes() {
        assert_eq!(parse_human_size("500B").unwrap(), 500);
    }

    #[test]
    fn test_parse_human_size_kb() {
        assert_eq!(parse_human_size("10KB").unwrap(), 10240);
    }

    #[test]
    fn test_parse_human_size_mb() {
        assert_eq!(parse_human_size("50MB").unwrap(), 52_428_800);
    }

    #[test]
    fn test_parse_human_size_gb() {
        assert_eq!(parse_human_size("2.5GB").unwrap(), 2_684_354_560);
    }

    #[test]
    fn test_parse_human_size_zero() {
        assert_eq!(parse_human_size("0").unwrap(), 0);
    }

    #[test]
    fn test_parse_human_size_invalid() {
        assert!(parse_human_size("xyz").is_err());
    }

    #[test]
    fn test_parse_human_size_empty() {
        assert!(parse_human_size("").is_err());
    }

    #[test]
    fn test_snapshot_builder_and_names() {
        let mut b = SnapshotBuilder::new("root");
        let f = b.push_file(ROOT_NODE, "a.txt", FileType::File, 10, 10);
        let d = b.push_dir(ROOT_NODE, "sub");
        b.push_file(d, "b.txt", FileType::File, 20, 20);
        let snap = b.finish(PathBuf::from("/tmp"), 30, 30);
        assert_eq!(snap.version, SNAPSHOT_VERSION);
        assert_eq!(snap.name(ROOT_NODE), "root");
        assert_eq!(snap.name(f), "a.txt");
        assert_eq!(snap.children_len(ROOT_NODE), 2);
        assert_eq!(snap.children_len(d), 1);
        assert!(snap.node(f).has_inline_name() || snap.name(f) == "a.txt");
    }

    #[test]
    fn test_long_name_goes_to_blob() {
        let long = "a".repeat(INLINE_NAME_MAX + 5);
        let mut b = SnapshotBuilder::new("root");
        let idx = b.push_file(ROOT_NODE, &long, FileType::File, 1, 1);
        let snap = b.finish(PathBuf::from("/tmp"), 1, 1);
        assert!(!snap.node(idx).has_inline_name());
        assert_eq!(snap.name(idx), long);
        assert!(!snap.names.is_empty());
    }

    #[test]
    fn test_compact_bytes_roundtrip() {
        let mut b = SnapshotBuilder::new("root");
        b.push_file(ROOT_NODE, "x.bin", FileType::File, 99, 99);
        let snap = b.finish(PathBuf::from("/data"), 99, 99);
        let bytes = snap.to_compact_bytes().unwrap();
        let back = Snapshot::from_bytes(&bytes).unwrap();
        assert_eq!(back.name(ROOT_NODE), "root");
        assert_eq!(back.total_size, 99);
        assert_eq!(back.children_len(ROOT_NODE), 1);
    }

    #[test]
    fn test_file_node_size_of_reasonable() {
        let sz = std::mem::size_of::<FileNode>();
        // 4+4+2+2+8+8+12 = 40
        assert_eq!(sz, 40, "FileNode should be 40 bytes, got {sz}");
    }
}
