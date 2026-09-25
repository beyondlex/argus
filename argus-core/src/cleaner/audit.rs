use std::io::{BufRead, Write};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditOp {
    Clean,
    Uninstall,
    Purge,
    Delete,
}

impl std::fmt::Display for AuditOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditOp::Clean => write!(f, "clean"),
            AuditOp::Uninstall => write!(f, "uninstall"),
            AuditOp::Purge => write!(f, "purge"),
            AuditOp::Delete => write!(f, "delete"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: DateTime<Utc>,
    pub operation: AuditOp,
    pub paths: Vec<PathBuf>,
    pub total_bytes: u64,
    pub success: bool,
    pub error: Option<String>,
}

fn audit_log_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "HOME not set".to_string())?;
    let mut dir = PathBuf::from(&home);
    dir.push(".config/argus");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create audit dir: {e}"))?;
    dir.push("audit.log");
    Ok(dir)
}

pub fn log_operation(entry: &AuditEntry) -> Result<(), String> {
    let path = audit_log_path()?;
    write_audit_entry(&path, entry)
}

pub fn read_audit_log(limit: usize) -> Result<Vec<AuditEntry>, String> {
    let path = audit_log_path()?;
    read_audit_from(&path, limit)
}

fn write_audit_entry(path: &std::path::Path, entry: &AuditEntry) -> Result<(), String> {
    let line = serde_json::to_string(entry).map_err(|e| format!("serialize audit entry: {e}"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open audit log: {e}"))?;
    writeln!(file, "{}", line).map_err(|e| format!("write audit log: {e}"))?;
    Ok(())
}

/// Read up to `limit` entries, keeping the *most recent* ones. The log is
/// append-only, so the head of the file holds the oldest operations; an
/// audit "show last N" query that returned the head would show the first
/// operations ever recorded instead of the latest ones.
/// Returned in chronological order (oldest of the tail first).
fn read_audit_from(path: &std::path::Path, limit: usize) -> Result<Vec<AuditEntry>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(path).map_err(|e| format!("open audit log: {e}"))?;
    let reader = std::io::BufReader::new(file);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read audit line: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(entry) => entries.push(entry),
            Err(_) => continue,
        }
    }
    if entries.len() > limit {
        entries.drain(..entries.len() - limit);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_log_and_read() {
        let dir = std::env::temp_dir().join("_argus_audit_test2");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let log_path = dir.join("audit.log");

        let entry = AuditEntry {
            timestamp: Utc::now(),
            operation: AuditOp::Clean,
            paths: vec![Path::new("/tmp/test").to_path_buf()],
            total_bytes: 1024,
            success: true,
            error: None,
        };
        write_audit_entry(&log_path, &entry).unwrap();

        let entries = read_audit_from(&log_path, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].operation.to_string(), "clean");
        assert_eq!(entries[0].total_bytes, 1024);
        assert!(entries[0].success);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_empty_log() {
        let dir = std::env::temp_dir().join("_argus_audit_empty2");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let log_path = dir.join("audit.log");
        let entries = read_audit_from(&log_path, 10).unwrap();
        assert!(entries.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `limit` selects the most recent entries (tail of the append-only log),
    /// not the first ones ever written.
    #[test]
    fn test_read_limit_returns_most_recent_entries() {
        let dir = std::env::temp_dir().join("_argus_audit_limit");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let log_path = dir.join("audit.log");

        for i in 0..5u64 {
            let entry = AuditEntry {
                timestamp: Utc::now(),
                operation: AuditOp::Clean,
                paths: vec![Path::new(&format!("/tmp/item-{i}")).to_path_buf()],
                total_bytes: i,
                success: true,
                error: None,
            };
            write_audit_entry(&log_path, &entry).unwrap();
        }

        let entries = read_audit_from(&log_path, 2).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].total_bytes, 3);
        assert_eq!(entries[1].total_bytes, 4);

        // A limit larger than the log returns everything, in order.
        let all = read_audit_from(&log_path, 10).unwrap();
        assert_eq!(all.len(), 5);
        assert_eq!(all[0].total_bytes, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
