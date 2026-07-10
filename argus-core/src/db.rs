use std::collections::HashMap;
use std::convert::TryFrom;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::model::{hash_root_path, DiffNode, FileNode, FileType, Snapshot};

#[derive(Debug, Clone)]
pub struct PathRecord {
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub file_type: FileType,
    pub modified: Option<String>,
    pub inode: Option<u64>,
    pub device: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PathDelta {
    pub path: String,
    pub size_from: u64,
    pub size_to: u64,
    pub delta: i64,
    pub is_dir: bool,
    pub file_type: FileType,
    pub exists_from: bool,
    pub exists_to: bool,
}

#[derive(Debug, Clone)]
pub struct RootScanSummary {
    pub root_path: PathBuf,
    pub root_path_hash: String,
    pub scan_count: u64,
    pub latest_timestamp: DateTime<Utc>,
}

#[derive(thiserror::Error, Debug)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("no scan data found for root path: {0}")]
    NoScanFound(String),
    #[error("timestamp parse error: {0}")]
    TimestampParse(String),
    #[error("numeric overflow: {0}")]
    NumericOverflow(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn default_db_path() -> PathBuf {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    config_dir.join("argus").join("argus.db")
}

pub fn open_db(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(path)?;
    init_db(&conn)?;
    Ok(conn)
}

pub fn write_scan(conn: &mut Connection, snapshot: &Snapshot) -> Result<i64, DbError> {
    let root_path = snapshot.root_path.to_string_lossy().to_string();
    let root_path_hash = snapshot.root_path_hash.clone();
    let total_files = count_files(&snapshot.root_node);

    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO scan_events (timestamp, root_path, root_path_hash, total_size, total_files)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            snapshot.timestamp.to_rfc3339(),
            root_path,
            root_path_hash,
            i64_from_u64(snapshot.total_size)?,
            i64_from_u64(total_files)?,
        ],
    )?;
    let scan_id = tx.last_insert_rowid();

    tx.execute(
        "INSERT INTO path_records (
            scan_id, path, size, is_dir, file_type, modified, inode, device
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            scan_id,
            snapshot.root_path.to_string_lossy().to_string(),
            i64_from_u64(snapshot.total_size)?,
            snapshot.root_node.is_dir,
            file_type_to_str(snapshot.root_node.file_type),
            snapshot
                .root_node
                .modified
                .as_ref()
                .map(|dt| dt.to_rfc3339()),
            optional_i64_from_u64(snapshot.root_node.inode)?,
            optional_i64_from_u64(snapshot.root_node.device)?,
        ],
    )?;

    let mut records = Vec::new();
    for child in snapshot.root_node.children.values() {
        let child_path = snapshot.root_path.join(&child.name);
        collect_path_records(&child_path, child, &mut records);
    }

    for record in records {
        tx.execute(
            "INSERT INTO path_records (
                scan_id, path, size, is_dir, file_type, modified, inode, device
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                scan_id,
                record.path,
                i64_from_u64(record.size)?,
                record.is_dir,
                file_type_to_str(record.file_type),
                record.modified,
                optional_i64_from_u64(record.inode)?,
                optional_i64_from_u64(record.device)?,
            ],
        )?;
    }

    tx.commit()?;
    Ok(scan_id)
}

pub fn query_delta(
    conn: &Connection,
    root_path: &Path,
    from_time: &DateTime<Utc>,
    to_time: &DateTime<Utc>,
) -> Result<Vec<PathDelta>, DbError> {
    let root_scan = root_path.to_string_lossy().to_string();
    let root_path_hash = hash_root_path(root_path);
    let from_scan_id = resolve_scan_id(conn, &root_path_hash, from_time)?;
    let to_scan_id = resolve_scan_id(conn, &root_path_hash, to_time)?;

    let from_records = load_scoped_records(conn, from_scan_id, &root_scan)?;
    let to_records = load_scoped_records(conn, to_scan_id, &root_scan)?;

    let mut from_map: HashMap<String, PathRecord> = HashMap::new();
    for record in from_records {
        from_map.insert(relative_record_path(root_path, &record.path), record);
    }

    let mut to_map: HashMap<String, PathRecord> = HashMap::new();
    for record in to_records {
        to_map.insert(relative_record_path(root_path, &record.path), record);
    }

    let mut keys: Vec<String> = from_map.keys().chain(to_map.keys()).cloned().collect();
    keys.sort();
    keys.dedup();

    let mut deltas = Vec::with_capacity(keys.len());
    for key in keys {
        let from = from_map.get(&key);
        let to = to_map.get(&key);
        let size_from = from.map(|r| r.size).unwrap_or(0);
        let size_to = to.map(|r| r.size).unwrap_or(0);
        let file_type = to
            .map(|r| r.file_type)
            .or_else(|| from.map(|r| r.file_type))
            .unwrap_or(FileType::Other);

        deltas.push(PathDelta {
            path: key,
            size_from,
            size_to,
            delta: i64_from_u64(size_to)? - i64_from_u64(size_from)?,
            is_dir: to.map(|r| r.is_dir).unwrap_or_else(|| from.map(|r| r.is_dir).unwrap_or(false)),
            file_type,
            exists_from: from.is_some(),
            exists_to: to.is_some(),
        });
    }

    Ok(deltas)
}

pub fn query_scan_timestamps(
    conn: &Connection,
    root_path: &Path,
) -> Result<Vec<(i64, DateTime<Utc>, u64, u64)>, DbError> {
    let root_path_hash = hash_root_path(root_path);
    let mut stmt = conn.prepare(
        "SELECT id, timestamp, total_size, total_files
         FROM scan_events
         WHERE root_path_hash = ?
         ORDER BY timestamp ASC",
    )?;

    let mut rows = stmt.query(params![root_path_hash])?;
    let mut scans = Vec::new();
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let timestamp: String = row.get(1)?;
        let total_size: i64 = row.get(2)?;
        let total_files: i64 = row.get(3)?;
        scans.push((
            id,
            parse_timestamp(&timestamp)?,
            u64_from_i64(total_size)?,
            u64_from_i64(total_files)?,
        ));
    }
    Ok(scans)
}

pub fn query_root_summaries(conn: &Connection) -> Result<Vec<RootScanSummary>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT root_path, root_path_hash, COUNT(*) AS scan_count, MAX(timestamp) AS latest_timestamp
         FROM scan_events
         GROUP BY root_path_hash, root_path
         ORDER BY root_path ASC",
    )?;

    let mut rows = stmt.query([])?;
    let mut summaries = Vec::new();
    while let Some(row) = rows.next()? {
        let root_path: String = row.get(0)?;
        let root_path_hash: String = row.get(1)?;
        let scan_count: i64 = row.get(2)?;
        let latest_timestamp: String = row.get(3)?;
        summaries.push(RootScanSummary {
            root_path: PathBuf::from(root_path),
            root_path_hash,
            scan_count: u64_from_i64(scan_count)?,
            latest_timestamp: parse_timestamp(&latest_timestamp)?,
        });
    }
    Ok(summaries)
}

pub fn rebuild_snapshot(conn: &Connection, root_path: &Path) -> Result<Snapshot, DbError> {
    let root_path_hash = hash_root_path(root_path);
    let scan_id = latest_scan_id(conn, &root_path_hash)?
        .ok_or_else(|| DbError::NoScanFound(root_path.display().to_string()))?;

    let root_path_str = root_path.to_string_lossy().to_string();
    let root_record = conn
        .query_row(
            "SELECT path, size, is_dir, file_type, modified, inode, device
             FROM path_records
             WHERE scan_id = ?1 AND path = ?2",
            params![scan_id, root_path_str],
            |row| path_record_from_row(row),
        )
        .optional()?
        .ok_or_else(|| DbError::NoScanFound(root_path.display().to_string()))?;

    let mut root_node = record_to_file_node(&root_record);
    let records = load_scan_records(conn, scan_id)?;
    for record in records {
        if record.path == root_record.path {
            continue;
        }
        insert_record(&mut root_node, root_path, &record);
    }

    let timestamp: String = conn.query_row(
        "SELECT timestamp FROM scan_events WHERE id = ?1",
        params![scan_id],
        |row| row.get(0),
    )?;

    Ok(Snapshot {
        version: crate::model::SNAPSHOT_VERSION,
        timestamp: parse_timestamp(&timestamp)?,
        root_path: root_path.to_path_buf(),
        root_path_hash: root_path_hash,
        total_size: root_node.size,
        root_node,
    })
}

pub fn build_diff_tree(records: &[PathDelta], root_name: &str) -> DiffNode {
    let mut root = DiffNode {
        name: root_name.to_string(),
        is_dir: true,
        current_size: 0,
        size_delta: 0,
        children: HashMap::new(),
    };

    for record in records {
        if record.path.is_empty() {
            root.current_size = record.size_to;
            root.size_delta = record.delta;
            continue;
        }
        insert_delta(&mut root, &record.path, record);
    }

    aggregate_diff(&mut root);
    root
}

fn init_db(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS scan_events (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp      TEXT NOT NULL,
            root_path      TEXT NOT NULL,
            root_path_hash TEXT NOT NULL,
            total_size     INTEGER NOT NULL,
            total_files    INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_scan_events_root_hash_ts
            ON scan_events(root_path_hash, timestamp);
        CREATE TABLE IF NOT EXISTS path_records (
            scan_id     INTEGER NOT NULL REFERENCES scan_events(id),
            path        TEXT NOT NULL,
            size        INTEGER NOT NULL,
            is_dir      INTEGER NOT NULL DEFAULT 0,
            file_type   TEXT NOT NULL,
            modified    TEXT,
            inode       INTEGER,
            device      INTEGER,
            PRIMARY KEY (scan_id, path)
        );
        CREATE INDEX IF NOT EXISTS idx_path_records_path
            ON path_records(path);
        ",
    )?;
    Ok(())
}

fn resolve_scan_id(
    conn: &Connection,
    root_path_hash: &str,
    requested_time: &DateTime<Utc>,
) -> Result<i64, DbError> {
    if let Some(id) = conn
        .query_row(
            "SELECT id
             FROM scan_events
             WHERE root_path_hash = ?1 AND timestamp <= ?2
             ORDER BY timestamp DESC
             LIMIT 1",
            params![root_path_hash, requested_time.to_rfc3339()],
            |row| row.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }

    conn.query_row(
        "SELECT id
         FROM scan_events
         WHERE root_path_hash = ?1 AND timestamp >= ?2
         ORDER BY timestamp ASC
         LIMIT 1",
        params![root_path_hash, requested_time.to_rfc3339()],
        |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DbError::NoScanFound(root_path_hash.to_string()))
}

fn latest_scan_id(conn: &Connection, root_path_hash: &str) -> Result<Option<i64>, DbError> {
    Ok(conn
        .query_row(
            "SELECT id
             FROM scan_events
             WHERE root_path_hash = ?1
             ORDER BY timestamp DESC
             LIMIT 1",
            params![root_path_hash],
            |row| row.get(0),
        )
        .optional()?)
}

fn load_scan_records(conn: &Connection, scan_id: i64) -> Result<Vec<PathRecord>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT path, size, is_dir, file_type, modified, inode, device
         FROM path_records
         WHERE scan_id = ?1
         ORDER BY path ASC",
    )?;
    let mut rows = stmt.query(params![scan_id])?;
    let mut records = Vec::new();
    while let Some(row) = rows.next()? {
        records.push(path_record_from_row(row)?);
    }
    Ok(records)
}

fn load_scoped_records(
    conn: &Connection,
    scan_id: i64,
    root_path: &str,
) -> Result<Vec<PathRecord>, DbError> {
    let query = if root_path == "/" {
        "SELECT path, size, is_dir, file_type, modified, inode, device
         FROM path_records
         WHERE scan_id = ?1 AND (path = ?2 OR path LIKE '/%')
         ORDER BY path ASC"
    } else {
        "SELECT path, size, is_dir, file_type, modified, inode, device
         FROM path_records
         WHERE scan_id = ?1 AND (path = ?2 OR path >= ?2 || '/' AND path < ?2 || '0')
         ORDER BY path ASC"
    };
    let mut stmt = conn.prepare(query)?;
    let mut rows = stmt.query(params![scan_id, root_path])?;
    let mut records = Vec::new();
    while let Some(row) = rows.next()? {
        records.push(path_record_from_row(row)?);
    }
    Ok(records)
}

fn path_record_from_row(row: &rusqlite::Row<'_>) -> Result<PathRecord, rusqlite::Error> {
    let file_type: String = row.get(3)?;
    Ok(PathRecord {
        path: row.get(0)?,
        size: row.get::<_, i64>(1)? as u64,
        is_dir: row.get::<_, i64>(2)? != 0,
        file_type: file_type_from_str(&file_type),
        modified: row.get(4)?,
        inode: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
        device: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
    })
}

fn collect_path_records(current_path: &Path, node: &FileNode, records: &mut Vec<PathRecord>) {
    let path = current_path.to_string_lossy().to_string();
    records.push(PathRecord {
        path: path.clone(),
        size: node.size,
        is_dir: node.is_dir,
        file_type: node.file_type,
        modified: node.modified.as_ref().map(|dt| dt.to_rfc3339()),
        inode: node.inode,
        device: node.device,
    });

    for child in node.children.values() {
        let child_path = current_path.join(&child.name);
        collect_path_records(&child_path, child, records);
    }
}

fn count_files(node: &FileNode) -> u64 {
    let mut count = if node.is_dir { 0 } else { 1 };
    for child in node.children.values() {
        count += count_files(child);
    }
    count
}

fn record_to_file_node(record: &PathRecord) -> FileNode {
    FileNode {
        name: Path::new(&record.path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| record.path.clone()),
        is_dir: record.is_dir,
        file_type: record.file_type,
        size: record.size,
        modified: record
            .modified
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        inode: record.inode,
        device: record.device,
        children: HashMap::new(),
    }
}

fn insert_record(root: &mut FileNode, root_path: &Path, record: &PathRecord) {
    let path = Path::new(&record.path);
    let Ok(relative) = path.strip_prefix(root_path) else {
        return;
    };

    let components: Vec<_> = relative.components().collect();
    if components.is_empty() {
        return;
    }

    insert_record_recursive(root, &components, 0, record);
}

fn insert_record_recursive(
    current: &mut FileNode,
    components: &[std::path::Component<'_>],
    index: usize,
    record: &PathRecord,
) {
    let name = components[index].as_os_str().to_string_lossy().to_string();
    if index + 1 == components.len() {
        current.children.insert(name, record_to_file_node(record));
        return;
    }

    let child = current.children.entry(name.clone()).or_insert_with(|| FileNode {
        name,
        is_dir: true,
        file_type: FileType::Directory,
        size: 0,
        modified: None,
        inode: None,
        device: None,
        children: HashMap::new(),
    });
    insert_record_recursive(child, components, index + 1, record);
}

fn insert_delta(root: &mut DiffNode, path: &str, record: &PathDelta) {
    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if components.is_empty() {
        return;
    }
    insert_delta_recursive(root, &components, 0, record);
}

fn insert_delta_recursive(
    current: &mut DiffNode,
    components: &[&str],
    index: usize,
    record: &PathDelta,
) {
    let name = components[index];
    if index + 1 == components.len() {
        current.children.insert(
            name.to_string(),
            DiffNode {
                name: name.to_string(),
                is_dir: record.is_dir,
                current_size: record.size_to,
                size_delta: record.delta,
                children: HashMap::new(),
            },
        );
        return;
    }

    let child = current.children.entry(name.to_string()).or_insert_with(|| DiffNode {
        name: name.to_string(),
        is_dir: true,
        current_size: 0,
        size_delta: 0,
        children: HashMap::new(),
    });
    insert_delta_recursive(child, components, index + 1, record);
}

fn aggregate_diff(node: &mut DiffNode) -> (u64, i64) {
    let mut total_size = node.current_size;
    let mut total_delta = node.size_delta;
    for child in node.children.values_mut() {
        let (child_size, child_delta) = aggregate_diff(child);
        total_size = total_size.saturating_add(child_size);
        total_delta += child_delta;
    }
    if node.current_size == 0 {
        node.current_size = total_size;
    }
    if node.size_delta == 0 {
        node.size_delta = total_delta;
    }
    (node.current_size, node.size_delta)
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, DbError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| DbError::TimestampParse(e.to_string()))
}

fn file_type_to_str(file_type: FileType) -> &'static str {
    match file_type {
        FileType::File => "file",
        FileType::Directory => "directory",
        FileType::Symlink => "symlink",
        FileType::Fifo => "fifo",
        FileType::Socket => "socket",
        FileType::Device => "device",
        FileType::Other => "other",
    }
}

fn file_type_from_str(value: &str) -> FileType {
    match value {
        "file" => FileType::File,
        "directory" => FileType::Directory,
        "symlink" => FileType::Symlink,
        "fifo" => FileType::Fifo,
        "socket" => FileType::Socket,
        "device" => FileType::Device,
        _ => FileType::Other,
    }
}

fn i64_from_u64(value: u64) -> Result<i64, DbError> {
    i64::try_from(value).map_err(|_| DbError::NumericOverflow(value.to_string()))
}

fn optional_i64_from_u64(value: Option<u64>) -> Result<Option<i64>, DbError> {
    value.map(i64_from_u64).transpose()
}

fn u64_from_i64(value: i64) -> Result<u64, DbError> {
    u64::try_from(value).map_err(|_| DbError::NumericOverflow(value.to_string()))
}

fn relative_record_path(root_path: &Path, path: &str) -> String {
    let full = Path::new(path);
    if full == root_path {
        return String::new();
    }
    full.strip_prefix(root_path)
        .unwrap_or(full)
        .to_string_lossy()
        .trim_start_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn file_node(name: &str, size: u64) -> FileNode {
        FileNode {
            name: name.to_string(),
            is_dir: false,
            file_type: FileType::File,
            size,
            modified: None,
            inode: None,
            device: None,
            children: HashMap::new(),
        }
    }

    fn dir_node(name: &str, children: HashMap<String, FileNode>) -> FileNode {
        FileNode {
            name: name.to_string(),
            is_dir: true,
            file_type: FileType::Directory,
            size: 0,
            modified: None,
            inode: None,
            device: None,
            children,
        }
    }

    fn sample_snapshot(root_path: &Path) -> Snapshot {
        let mut dir_children = HashMap::new();
        dir_children.insert("a.txt".to_string(), file_node("a.txt", 100));
        let mut root_children = HashMap::new();
        root_children.insert("dir".to_string(), dir_node("dir", dir_children));
        root_children.insert("b.txt".to_string(), file_node("b.txt", 50));
        let root_name = root_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| root_path.to_string_lossy().to_string());
        let root = dir_node(&root_name, root_children);
        Snapshot::new(root_path.to_path_buf(), root, 150)
    }

    #[test]
    fn test_default_db_path() {
        let path = default_db_path();
        assert!(path.ends_with("argus.db"));
    }

    #[test]
    fn test_write_and_rebuild_snapshot_round_trip() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("argus.db");
        let mut conn = open_db(&db_path).unwrap();
        let snapshot = sample_snapshot(Path::new("/tmp/downloads"));

        write_scan(&mut conn, &snapshot).unwrap();
        let rebuilt = rebuild_snapshot(&conn, Path::new("/tmp/downloads")).unwrap();

        assert_eq!(rebuilt.root_path, snapshot.root_path);
        assert_eq!(rebuilt.total_size, snapshot.total_size);
        assert_eq!(rebuilt.root_node.children.len(), 2);
        assert!(rebuilt.root_node.children.contains_key("dir"));
        assert!(rebuilt.root_node.children.contains_key("b.txt"));
    }

    #[test]
    fn test_query_scan_timestamps_and_root_summaries() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("argus.db");
        let mut conn = open_db(&db_path).unwrap();
        let snapshot = sample_snapshot(Path::new("/tmp/downloads"));
        write_scan(&mut conn, &snapshot).unwrap();
        write_scan(&mut conn, &snapshot).unwrap();

        let scans = query_scan_timestamps(&conn, Path::new("/tmp/downloads")).unwrap();
        assert_eq!(scans.len(), 2);

        let summaries = query_root_summaries(&conn).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].scan_count, 2);
    }

    #[test]
    fn test_query_delta_reports_additions_and_deletions() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("argus.db");
        let mut conn = open_db(&db_path).unwrap();

        let root_path = Path::new("/tmp/downloads");
        let mut old = sample_snapshot(root_path);
        let mut new = sample_snapshot(root_path);
        new.root_node
            .children
            .insert("new.txt".to_string(), file_node("new.txt", 25));
        new.root_node.children.remove("b.txt");

        let mut old_clone = old.clone();
        old_clone.timestamp = Utc::now() - chrono::Duration::days(1);
        let mut new_clone = new.clone();
        new_clone.timestamp = Utc::now();

        write_scan(&mut conn, &old_clone).unwrap();
        write_scan(&mut conn, &new_clone).unwrap();

        let deltas = query_delta(&conn, root_path, &old_clone.timestamp, &new_clone.timestamp).unwrap();
        assert!(deltas.iter().any(|d| d.path == "new.txt" && d.exists_to && !d.exists_from));
        assert!(deltas.iter().any(|d| d.path == "b.txt" && d.exists_from && !d.exists_to));
    }

    #[test]
    fn test_build_diff_tree_uses_root_name() {
        let records = vec![
            PathDelta {
                path: String::new(),
                size_from: 100,
                size_to: 120,
                delta: 20,
                is_dir: true,
                file_type: FileType::Directory,
                exists_from: true,
                exists_to: true,
            },
            PathDelta {
                path: "dir/a.txt".to_string(),
                size_from: 10,
                size_to: 20,
                delta: 10,
                is_dir: false,
                file_type: FileType::File,
                exists_from: true,
                exists_to: true,
            },
        ];

        let tree = build_diff_tree(&records, "downloads");
        assert_eq!(tree.name, "downloads");
        assert!(tree.children.contains_key("dir"));
        assert_eq!(tree.size_delta, 20);
    }
}
