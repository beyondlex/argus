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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Fifo,
    Socket,
    Device,
    Other,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileNode {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<NodeIndex>,
    pub is_dir: bool,
    pub file_type: FileType,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<(String, NodeIndex)>,
}

impl FileNode {
    /// Look up child index by name (linear scan — children sets are small)
    pub fn child_idx(&self, name: &str) -> Option<NodeIndex> {
        self.children
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, idx)| *idx)
    }

    /// Sort children by name for binary-search compatible iteration
    pub fn sort_children(&mut self) {
        self.children.sort_by(|a, b| a.0.cmp(&b.0));
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Snapshot {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub root_path: PathBuf,
    pub root_path_hash: String,
    pub total_size: u64,
    pub arena: Vec<FileNode>,
}

impl Snapshot {
    pub fn new(root_path: PathBuf, arena: Vec<FileNode>, total_size: u64) -> Self {
        let root_path_hash = hash_root_path(&root_path);
        Self {
            version: SNAPSHOT_VERSION,
            timestamp: Utc::now(),
            root_path,
            root_path_hash,
            total_size,
            arena,
        }
    }

    pub fn root(&self) -> &FileNode {
        &self.arena[ROOT_NODE as usize]
    }

    pub fn node(&self, idx: NodeIndex) -> &FileNode {
        &self.arena[idx as usize]
    }

    pub fn node_mut(&mut self, idx: NodeIndex) -> &mut FileNode {
        &mut self.arena[idx as usize]
    }

    pub fn root_mut(&mut self) -> &mut FileNode {
        &mut self.arena[ROOT_NODE as usize]
    }

    /// Serialize to compact JSON and gzip-compress the result.
    pub fn to_compact_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        let json = serde_json::to_string(self)?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json.as_bytes())?;
        encoder
            .finish()
            .map_err(|e| SnapshotError::Corrupted(format!("compression failed: {e}")))
    }

    /// Deserialize from bytes, auto-detecting gzip compression by magic bytes (0x1f, 0x8b).
    pub fn from_bytes(data: &[u8]) -> Result<Self, SnapshotError> {
        let json = if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
            let mut decoder = GzDecoder::new(data);
            let mut s = String::new();
            decoder.read_to_string(&mut s)?;
            s
        } else {
            String::from_utf8_lossy(data).to_string()
        };
        Ok(serde_json::from_str(&json)?)
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

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

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
}

pub const SNAPSHOT_VERSION: u32 = 2;

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
    fn test_snapshot_new() {
        let arena = vec![FileNode {
            name: "test".into(),
            parent: None,
            is_dir: true,
            file_type: FileType::Directory,
            size: 100,
            children: Vec::new(),
        }];
        let snap = Snapshot::new(PathBuf::from("/tmp"), arena, 100);
        assert_eq!(snap.version, SNAPSHOT_VERSION);
        assert_eq!(snap.root_path_hash.len(), 8);
    }

    #[test]
    fn test_file_node_serialization() {
        let node = FileNode {
            name: "test.txt".into(),
            parent: None,
            is_dir: false,
            file_type: FileType::File,
            size: 1024,
            children: Vec::new(),
        };
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("test.txt"));
        let deserialized: FileNode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test.txt");
    }
}
