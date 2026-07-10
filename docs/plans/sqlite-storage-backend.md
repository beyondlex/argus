# SQLite Storage Backend: Primary Scan History Store

## Status: Draft

This plan defines SQLite as the primary storage layer for Argus scan history and time-based delta queries.
It replaces the JSON snapshot persistence design for the current implementation track.

## 1. Motivation

### 1.1 Why SQLite

| Problem with file snapshots | SQLite benefit |
|---|---|
| Every diff loads full trees into memory | Query only the rows needed for the current subtree |
| Time selection is limited to preexisting files | Query arbitrary `from` / `to` timestamps and snap to nearest scan |
| Startup must scan the snapshot directory | Startup can read structured scan metadata directly from the database |
| Diff data is pairwise and file-based | Delta becomes a first-class time-series query |

### 1.2 Core Query Shape

The TUI needs to answer:

> For the subtree currently being browsed, what changed between two timestamps?

That is a range query over scan history, filtered by path prefix.

## 2. Data Model

### 2.1 Tables

```sql
CREATE TABLE IF NOT EXISTS scan_events (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp      TEXT NOT NULL,  -- RFC3339 UTC
    root_path      TEXT NOT NULL,  -- absolute path of the scanned root
    root_path_hash TEXT NOT NULL,  -- first 8 hex chars of SHA256(root_path)
    total_size     INTEGER NOT NULL,
    total_files    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_scan_events_root_hash_ts
    ON scan_events(root_path_hash, timestamp);

CREATE TABLE IF NOT EXISTS path_records (
    scan_id     INTEGER NOT NULL REFERENCES scan_events(id),
    path        TEXT NOT NULL,   -- absolute path, normalized without trailing slash
    size        INTEGER NOT NULL,
    is_dir      INTEGER NOT NULL DEFAULT 0,
    file_type   TEXT NOT NULL,   -- matches FileType enum
    modified    TEXT,            -- RFC3339 UTC, nullable
    inode       INTEGER,
    device      INTEGER,
    PRIMARY KEY (scan_id, path)
);

CREATE INDEX IF NOT EXISTS idx_path_records_path
    ON path_records(path);
```

### 2.2 Design Notes

| Decision | Reason |
|---|---|
| `PRIMARY KEY (scan_id, path)` | One record per path per scan. The PK already covers scan-scoped lookups. |
| Absolute `TEXT` paths | Simple prefix matching and direct reconstruction of the tree. |
| `file_type`, `modified`, `inode`, `device` | Preserve the current `FileNode` semantics and keep hard-link / special-file metadata available. |
| `root_path_hash` | Fast grouping and stable root identity without recomputing hashes on every query. |

### 2.3 Storage Location

```text
~/.config/argus/argus.db
```

The path is configurable via `config.toml`.

## 3. Query Patterns

### 3.1 Resolve Scan IDs for Time Points

For each timestamp, the database should snap to the nearest scan:

1. Try the latest scan with `timestamp <= requested_time`.
2. If none exists, fall back to the earliest scan with `timestamp >= requested_time`.

This resolution is implemented in Rust with two small queries, backed by
`idx_scan_events_root_hash_ts`.

### 3.2 Delta Query

The delta query must include:

- the root path itself
- added paths
- deleted paths
- modified paths

```sql
WITH from_scan AS (
    SELECT id
    FROM scan_events
    WHERE root_path_hash = ?2 AND timestamp <= ?1
    ORDER BY timestamp DESC
    LIMIT 1
),
to_scan AS (
    SELECT id
    FROM scan_events
    WHERE root_path_hash = ?2 AND timestamp <= ?3
    ORDER BY timestamp DESC
    LIMIT 1
),
scoped_from AS (
    SELECT p.*
    FROM path_records p
    JOIN from_scan fs ON p.scan_id = fs.id
    WHERE p.path = ?4
       OR (p.path >= ?4 || '/' AND p.path < ?4 || '0')
),
scoped_to AS (
    SELECT p.*
    FROM path_records p
    JOIN to_scan ts ON p.scan_id = ts.id
    WHERE p.path = ?4
       OR (p.path >= ?4 || '/' AND p.path < ?4 || '0')
),
all_paths AS (
    SELECT path FROM scoped_from
    UNION
    SELECT path FROM scoped_to
)
SELECT
    ap.path,
    COALESCE(sf.size, 0) AS size_from,
    COALESCE(st.size, 0) AS size_to,
    CAST(COALESCE(st.size, 0) AS INTEGER) - CAST(COALESCE(sf.size, 0) AS INTEGER) AS delta,
    COALESCE(sf.is_dir, st.is_dir, 0) AS is_dir,
    COALESCE(sf.file_type, st.file_type, 'other') AS file_type,
    sf.path IS NOT NULL AS exists_from,
    st.path IS NOT NULL AS exists_to
FROM all_paths ap
LEFT JOIN scoped_from sf ON sf.path = ap.path
LEFT JOIN scoped_to st ON st.path = ap.path
ORDER BY ap.path ASC;
```

### 3.3 Scan Timestamp Listing

```sql
SELECT id, timestamp, total_size, total_files
FROM scan_events
WHERE root_path_hash = ?
ORDER BY timestamp ASC;
```

### 3.4 Root Summary Listing

Used by `list-scans` without `--path`.

```sql
SELECT
    root_path,
    root_path_hash,
    COUNT(*) AS scan_count,
    MAX(timestamp) AS latest_timestamp
FROM scan_events
GROUP BY root_path_hash, root_path
ORDER BY root_path ASC;
```

### 3.5 Materialize Latest Snapshot

```sql
SELECT p.path, p.size, p.is_dir, p.file_type, p.modified, p.inode, p.device
FROM path_records p
JOIN (
    SELECT id
    FROM scan_events
    WHERE root_path_hash = ?
    ORDER BY timestamp DESC
    LIMIT 1
) s ON p.scan_id = s.id
ORDER BY p.path ASC;
```

This is used to rebuild an in-memory `Snapshot` for the currently viewed root.

## 4. Writing Scan Data

### 4.1 Write Flow

```text
scan_path(path)
  -> build FileNode tree in memory
  -> return Snapshot { root_path, timestamp, root_node, total_size }

write_scan(conn, snapshot)
  -> BEGIN TRANSACTION
  -> INSERT scan_events row
  -> flatten the tree into PathRecord rows
  -> INSERT path_records rows
  -> COMMIT
```

Every scan write is atomic. If the process stops mid-write, SQLite rolls the transaction back.

### 4.2 Record Flattening

`collect_path_records` must preserve the full tree shape:

```rust
fn collect_path_records(node: &FileNode, full_path: &Path, records: &mut Vec<PathRecord>) {
    records.push(PathRecord {
        path: full_path.to_string_lossy().to_string(),
        size: node.size,
        is_dir: node.is_dir,
        file_type: node.file_type,
        modified: node.modified.map(|dt| dt.to_rfc3339()),
        inode: node.inode,
        device: node.device,
    });

    for child in node.children.values() {
        let child_path = full_path.join(&child.name);
        collect_path_records(child, &child_path, records);
    }
}
```

The root node is stored as a record too.

## 5. Rust API

### 5.1 Module: `argus-core/src/db.rs`

```rust
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
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn default_db_path() -> PathBuf;
pub fn open_db(path: &Path) -> Result<Connection, DbError>;
pub fn write_scan(conn: &Connection, snapshot: &Snapshot) -> Result<i64, DbError>;
pub fn query_delta(
    conn: &Connection,
    root_path: &Path,
    from_time: &DateTime<Utc>,
    to_time: &DateTime<Utc>,
) -> Result<Vec<PathDelta>, DbError>;
pub fn query_scan_timestamps(
    conn: &Connection,
    root_path: &Path,
) -> Result<Vec<(i64, DateTime<Utc>, u64, u64)>, DbError>;
pub fn query_root_summaries(conn: &Connection) -> Result<Vec<RootScanSummary>, DbError>;
pub fn rebuild_snapshot(conn: &Connection, root_path: &Path) -> Result<Snapshot, DbError>;
pub fn build_diff_tree(records: &[PathDelta], root_name: &str) -> DiffNode;
```

### 5.2 `rebuild_snapshot` Algorithm

Input: flat `path_records` for the latest scan of one root.

1. Sort rows by path length so parents are inserted before children.
2. Create the root `FileNode` from the record whose path equals `root_path`.
3. For every other record, split the absolute path into components under the root.
4. Walk or create intermediate directory nodes.
5. Attach the stored node data at the leaf.
6. Return the reconstructed `Snapshot`.

Directory sizes are taken from the stored records, not recomputed during reconstruction.

### 5.3 `build_diff_tree` Algorithm

1. Insert every `PathDelta` into a synthetic tree keyed by absolute path.
2. Use `exists_from` / `exists_to` to preserve add/remove semantics.
3. Aggregate directory `delta` bottom-up from child nodes.
4. Keep the root node as the tree root for the current view path.

## 6. CLI Changes

### 6.1 `scan`

```bash
argus scan --path /home/user/Downloads
```

Scans the target path and writes the result to SQLite.

### 6.2 `diff`

```bash
argus diff --path /home/user/Downloads \
           --from 2026-06-01T00:00:00Z \
           --to 2026-06-15T00:00:00Z
```

Time-based diff becomes the primary mode. The command queries SQLite, builds `DiffNode`, and renders it.

### 6.3 `list-scans`

```bash
argus list-scans --path /home/user/Downloads
argus list-scans
```

With `--path`, list all timestamps for that root. Without `--path`, list all roots with scan counts.

### 6.4 Removed Commands

- No JSON migration command in the current plan.
- No file-based `diff --old/--new` in the current plan.

## 7. TUI Changes

### 7.1 Connection Model

`rusqlite::Connection` is not stored inside `App`.

Instead:

1. `App` stores `db_path: PathBuf`.
2. Background tasks open a short-lived connection through `open_db()`.
3. Query and write operations stay inside blocking tasks.

This keeps the UI thread free of SQLite borrowing and thread-safety issues.

### 7.2 Startup

On startup:

1. Open or create the database.
2. Load `query_root_summaries()` into the TUI root index.
3. Materialize the latest snapshot for the current working directory if it exists.
4. Otherwise render the filesystem listing for the current directory.

### 7.3 Delta Rendering

`trigger_diff_if_ready()` becomes a database-backed query:

1. Resolve the selected time range to two scan IDs.
2. Run `query_delta()`.
3. Pass the records to `build_diff_tree()`.
4. Render the resulting `DiffNode`.

### 7.4 Scan Completion

When a scan completes:

1. Update the in-memory cache for the current root.
2. Write the snapshot to SQLite in a background blocking task.
3. Refresh the current view if the scanned path matches the active root.

## 8. Implementation Order

```text
Phase A: argus-core database module
  A.1  Add rusqlite = { version = "0.31", features = ["bundled"] }
  A.2  Add db.rs with DDL, open_db, write_scan
  A.3  Add scan resolution helpers and query_delta
  A.4  Add query_scan_timestamps, query_root_summaries, rebuild_snapshot
  A.5  Add build_diff_tree and export the db module
  A.6  Unit tests for round-trip writes, added/deleted paths, empty trees, and root queries

Phase B: CLI
  B.1  Update scan to write SQLite
  B.2  Update diff to use --from / --to / --path
  B.3  Add list-scans
  B.4  Remove JSON-specific CLI paths from the current track

Phase C: TUI
  C.1  Load root summaries from SQLite at startup
  C.2  Materialize latest snapshots for active roots
  C.3  Use query_delta + build_diff_tree for diff mode
  C.4  Write completed scans back to SQLite

Final: cargo test && cargo clippy && cargo fmt --check
```

## 9. Testing Strategy

| Layer | What | How |
|---|---|---|
| Unit | `write_scan` / `rebuild_snapshot` round trip | In-memory SQLite, write a mock snapshot, rebuild it, compare tree shape |
| Unit | Delta query with additions and deletions | Build two scans with mismatched path sets, assert `exists_from` / `exists_to` |
| Unit | Empty tree and single-file tree | Verify root handling and zero-child behavior |
| Unit | Timestamp resolution fallback | Test `<=` path first, then `>=` fallback |
| Unit | `query_root_summaries` | Verify grouping and latest timestamp selection |
| Integration | CLI `scan` → `diff` → `list-scans` | Temporary DB, scan test directory, query by time, verify output |
| TUI | Startup and root switching | Verify the current cwd materializes correctly from SQLite or falls back to live FS listing |

## 10. Future Considerations

- WAL mode for daemon-era concurrent reads and writes
- Retention policy for old scans
- Optional export to JSON or other formats if the project later needs it
- Schema versioning via `PRAGMA user_version`
