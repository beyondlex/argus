# SQLite Storage Backend: Replace JSON Snapshots with Time-Series Database

## Status: Draft

This document supersedes the JSON snapshot persistence design in `08-data-model.md` §3.
The JSON file format is retained as a read-only fallback for backward compatibility (see §5).
The SQLite time-series model from `08-data-model.md` §6 (labeled Phase 3+) is now promoted to the primary storage layer.

## 1. Motivation

### 1.1 Problems with the JSON Snapshot Model

| Problem | Impact |
|---------|--------|
| **Full-file diff only** — TUI must load two complete JSON snapshots into memory and run full tree merge, even when only browsing a small subtree | High memory/IO waste, slow startup with many snapshots |
| **No arbitrary time queries** — user must select two specific scan events; cannot pick free-form from/to timestamps | Poor UX in TUI filter bar |
| **No global index** — TUI startup must scan the snapshots directory, parse every filename, and deserialize every file to build the snapshot index | Slow startup, ~O(n) in number of snapshots |
| **CLI `diff` is file-based** — requires explicit `--old` and `--new` file paths instead of `--from`/`--to` timestamps | Cumbersome workflow |
| **No incremental query** — every diff requires loading two complete trees regardless of how small the visible subtree is | Prevents efficient server-mode queries in Phase 3 |

### 1.2 TUI Delta Query Model

The TUI needs to answer this question:

> For the file tree currently being browsed (rooted at `/home/user/Downloads`),
> what is the size delta of each path between two arbitrary time points?

This is fundamentally a **time-series range query** with path-prefix filtering — not a pairwise file comparison.

## 2. Data Model

### 2.1 Tables

```sql
-- Each scan event: one row per scan operation
CREATE TABLE IF NOT EXISTS scan_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp       TEXT NOT NULL,       -- ISO 8601 UTC
    root_path       TEXT NOT NULL,       -- absolute path of scan root
    root_path_hash  TEXT NOT NULL,       -- first 8 hex chars of SHA256(root_path)
    total_size      INTEGER NOT NULL,    -- total size in bytes
    total_files     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_scan_events_hash
    ON scan_events(root_path_hash);
CREATE INDEX IF NOT EXISTS idx_scan_events_ts
    ON scan_events(timestamp);

-- Per-path size record at each scan
CREATE TABLE IF NOT EXISTS path_records (
    scan_id     INTEGER NOT NULL REFERENCES scan_events(id),
    path        TEXT NOT NULL,           -- absolute path
    size        INTEGER NOT NULL,        -- size in bytes
    is_dir      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (scan_id, path)
);

-- Composite index: scan_id first for filtering, path second for prefix matching
CREATE INDEX IF NOT EXISTS idx_path_records_scan_path
    ON path_records(scan_id, path);

-- Standalone path index for cross-scan JOIN performance
CREATE INDEX IF NOT EXISTS idx_path_records_path
    ON path_records(path);
```

### 2.2 Key Design Rationale

| Decision | Rationale |
|----------|-----------|
| `PRIMARY KEY (scan_id, path)` | Enforces one record per path per scan. The implicit unique index on (scan_id, path) also serves the most common query pattern: filter by scan_id, then range-scan by path prefix. |
| `path` stored as full absolute TEXT | Simpler than adjacency lists or nested sets. SQLite's B-tree index on TEXT supports efficient prefix range scans (`>= '/x/y/' AND < '/x/y0'`). |
| `root_path_hash` denormalized into `scan_events` | Avoids recomputing SHA256 on every query. Matches the existing `hash_root_path()` function used by JSON filenames. |
| `is_dir` on each record | Enables directory aggregation at query time without joining back to scan_events or a separate table. |
| No separate `file_metadata` table (modified time, inode, etc.) | YAGNI: these fields are not needed for delta queries and are not displayed in the TUI. Can be added later if needed. |

### 2.3 Storage Location

```
~/.config/argus/argus.db
```

Path is configurable via `config.toml`:

```toml
[database]
path = "~/.config/argus/argus.db"
```

Default resolved by `argus_core::db::default_db_path()` which returns `{config_dir}/argus/argus.db`.

## 3. Query Patterns

### 3.1 Find Scan IDs for Time Points

For a given "from" time and "to" time selected by the user, find the nearest scan that is **not later than** each time point:

```sql
WITH from_scan AS (
    SELECT id, timestamp FROM scan_events
    WHERE timestamp <= ?1 AND root_path_hash = ?2
    ORDER BY timestamp DESC LIMIT 1
),
to_scan AS (
    SELECT id, timestamp FROM scan_events
    WHERE timestamp <= ?3 AND root_path_hash = ?2
    ORDER BY timestamp DESC LIMIT 1
)
```

This handles the "arbitrary time point → closest scan data" mapping. If no scan is found before the given time, the scan with the smallest timestamp >= the given time is used as a fallback.

### 3.2 Path Delta Query

```sql
SELECT
    p1.path,
    p1.size           AS size_from,
    p2.size           AS size_to,
    CAST(p2.size AS INTEGER) - CAST(p1.size AS INTEGER) AS delta,
    p1.is_dir
FROM path_records p1
JOIN path_records p2 ON p1.path = p2.path
CROSS JOIN from_scan fs
CROSS JOIN to_scan ts
WHERE p1.scan_id = fs.id
  AND p2.scan_id = ts.id
  AND p1.path >= ?4 || '/'
  AND p1.path < ?4 || '0'
```

**Performance characteristics**:
- `p1.scan_id = fs.id` — B-tree range scan on `idx_path_records_scan_path`, returning all rows for that scan
- `p1.path >= 'X/' AND p1.path < 'X0'` — same index, range-scan to a contiguous subset of those rows (no additional lookup)
- `p2.path = p1.path` — index lookup on `idx_path_records_path` for the second scan
- Result: ~O(rows_in_subtree) per query, typically sub-10ms for trees with <100K paths

### 3.3 List Available Scan Timestamps for a Root Path

```sql
SELECT id, timestamp, total_size, total_files
FROM scan_events
WHERE root_path_hash = ?
ORDER BY timestamp ASC
```

Used by the TUI filter bar to show available time points.

### 3.4 Materialize Latest Snapshot for a Root Path

```sql
SELECT p.path, p.size, p.is_dir
FROM path_records p
JOIN (
    SELECT id FROM scan_events
    WHERE root_path_hash = ?
    ORDER BY timestamp DESC LIMIT 1
) s ON p.scan_id = s.id
ORDER BY p.path ASC
```

Used to rebuild the in-memory `Snapshot` / `FileNode` tree for the current view root when scan data exists.

## 4. Writing Scan Data

### 4.1 Write Flow

```
scan_path(path)
    → builds FileNode tree in memory (existing logic unchanged)
    → returns Snapshot { root_path, timestamp, root_node, total_size }

write_scan(conn, &snapshot)
    → BEGIN TRANSACTION
    → INSERT INTO scan_events (timestamp, root_path, root_path_hash, total_size, total_files)
    → walk FileNode recursively, collect (path, size, is_dir) pairs
    → INSERT INTO path_records (scan_id, path, size, is_dir) for each pair
    → COMMIT
```

The `FileNode` tree walker flattens the recursive structure into flat path records:

```rust
fn collect_path_records(node: &FileNode, prefix: &Path, records: &mut Vec<PathRecord>) {
    let full_path = prefix.join(&node.name);
    records.push(PathRecord {
        path: full_path.to_string_lossy().to_string(),
        size: node.size,
        is_dir: node.is_dir as u8,
    });
    for child in node.children.values() {
        collect_path_records(child, &full_path, records);
    }
}
```

### 4.2 Transactional Guarantee

A scan write is wrapped in a single transaction. If interrupted, the partial scan is rolled back — no inconsistent state. This is a key advantage over the JSON approach where a partial write could produce a corrupt `.json.gz` file.

## 5. Rust API

### 5.1 Module: `argus-core/src/db.rs`

```rust
/// Result of a path delta query
#[derive(Debug, Clone)]
pub struct PathDelta {
    pub path: String,
    pub size_from: u64,
    pub size_to: u64,
    pub delta: i64,
    pub is_dir: bool,
}

/// Errors from database operations
#[derive(thiserror::Error, Debug)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("no scan data found: {0}")]
    NoScanFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Get the default database path (~/.config/argus/argus.db)
pub fn default_db_path() -> PathBuf;

/// Open (or create) the database at the given path, running DDL if needed.
pub fn open_db(path: &Path) -> Result<Connection, DbError>;

/// Write a completed scan into the database.
/// Returns the scan_event.id of the newly inserted row.
pub fn write_scan(conn: &Connection, snapshot: &Snapshot) -> Result<i64, DbError>;

/// Query delta for all paths under `root_path` between two time points.
/// Each time point snaps to the nearest scan at or before that timestamp.
pub fn query_delta(
    conn: &Connection,
    root_path: &Path,
    from_time: &DateTime<Utc>,
    to_time: &DateTime<Utc>,
) -> Result<Vec<PathDelta>, DbError>;

/// List all scan timestamps for a given root path (for filter bar).
pub fn query_scan_timestamps(
    conn: &Connection,
    root_path: &Path,
) -> Result<Vec<(i64, DateTime<Utc>, u64, u64)>, DbError>;

/// Rebuild a full Snapshot from the latest scan data for a given root path.
pub fn rebuild_snapshot(
    conn: &Connection,
    root_path: &Path,
) -> Result<Snapshot, DbError>;

/// Build a DiffNode tree from flat PathDelta records.
/// The tree is built bottom-up: directory deltas are aggregated from children.
pub fn build_diff_tree(records: &[PathDelta], root_name: &str) -> DiffNode;

/// Import all JSON snapshots from a directory into the database.
pub fn migrate_from_json(conn: &Connection, snapshots_dir: &Path) -> Result<u64, DbError>;
```

### 5.2 `build_diff_tree` Algorithm

```
Input: flat [(path, size_from, size_to, delta, is_dir)] records
       root_name (e.g. "Downloads")
Output: DiffNode tree

1. Parse each path into components by '/'
2. Build a tree of intermediate nodes:
   - Each node has: name, children: HashMap<String, Node>
   - Attach leaf data (size_from, size_to, delta) to leaf nodes
   - Create directory nodes for intermediate path components
3. Post-order traverse the tree:
   - For each directory: delta = sum(child.delta)
   - For each directory: current_size = sum(child.current_size)
4. Return root DiffNode

Note: size_delta of directory nodes is the sum of children's deltas,
NOT the delta of the directory entry itself (which may differ in some filesystems).
```

### 5.3 Backward Compatibility

```rust
/// Check if a database exists at the given path.
pub fn db_exists(path: &Path) -> bool;

/// Fallback: load all JSON snapshots from directory into scan_cache (existing logic).
/// Used when no argus.db exists.
pub fn load_json_snapshots(snapshots_dir: &Path) -> (HashMap<PathBuf, Snapshot>, HashMap<String, Vec<SnapshotInfo>>);
```

### 5.4 Exports

Updated `argus-core/src/lib.rs`:

```rust
pub mod db;
pub use db::{
    open_db, write_scan, query_delta, query_scan_timestamps,
    rebuild_snapshot, build_diff_tree, migrate_from_json,
    default_db_path, db_exists,
    PathDelta, DbError,
};
```

## 6. CLI Changes

### 6.1 `scan` Command

```bash
# Current
argus scan --path /home/user/Downloads --output snap_a.json.gz

# New (backward-compatible)
argus scan --path /home/user/Downloads
# Writes to SQLite by default. Optional --db flag.
# --output still supported for JSON export.
```

The `scan` command writes to SQLite as the primary store. If `--output` is specified, also writes a JSON file for backward compatibility.

### 6.2 `diff` Command

```bash
# Current (file-based)
argus diff --old snap_a.json.gz --new snap_b.json.gz

# New (time-based, primary)
argus diff --path /home/user/Downloads \
           --from 2026-06-01T00:00:00Z \
           --to 2026-06-15T00:00:00Z

# Legacy file-based still supported for JSON files
argus diff --old snap_a.json.gz --new snap_b.json.gz
```

The time-based mode queries SQLite directly, builds the DiffNode tree, and renders it identically to the current file-based diff output.

### 6.3 `migrate` Command

```bash
argus migrate
# Imports all JSON snapshots from ~/.config/argus/snapshots/ into argus.db
# Reports: "Migrated 5 snapshots (12583 path records)"
```

### 6.4 `list-scans` Command (New)

```bash
argus list-scans --path /home/user/Downloads
# Lists all scan timestamps for the given path

argus list-scans
# Lists all root paths with their scan counts
```

## 7. TUI Changes

### 7.1 App State Changes

```rust
pub struct App {
    // ... existing fields ...

    // NEW: database connection (None if no argus.db exists, fallback to JSON)
    pub db: Option<Connection>,
}
```

### 7.2 Snapshot Loading

`load_all_snapshots()` is updated:

1. Try to open `argus.db`
2. If success:
   - Query `scan_events` for all root paths → build `snapshot_index`
   - For each root path, rebuild the latest `Snapshot` → populate `scan_cache`
3. If no argus.db:
   - Fall back to JSON directory scan (existing behavior)

### 7.3 Delta Query

`trigger_diff_if_ready()` is updated:

```rust
fn trigger_diff_if_ready(app: &mut App) {
    let db = match &app.db {
        Some(db) => db,
        None => return trigger_diff_from_json(app),  // fallback
    };

    let from_ts = app.available_snapshots[from_idx].timestamp;
    let to_ts = app.available_snapshots[to_idx].timestamp;

    match query_delta(db, &app.view_root_path, &from_ts, &to_ts) {
        Ok(records) => {
            let root_name = app.view_root_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let diff = build_diff_tree(&records, &root_name);
            app.tx.blocking_send(AppMessage::DiffComplete(diff));
        }
        Err(e) => {
            app.tx.blocking_send(AppMessage::Error(format!("delta query failed: {e}")));
        }
    }
}
```

### 7.4 Scan Completion

`handle_message(AppMessage::ScanComplete)` is updated to also write to SQLite:

```rust
AppMessage::ScanComplete(snapshot) => {
    // ... existing logic (update scan_cache, snapshot_index) ...

    // NEW: write to SQLite
    if let Some(db) = &self.db {
        if let Err(e) = write_scan(db, &snapshot) {
            // log warning but don't fail — JSON backup may exist
        }
    }
}
```

## 8. Implementation Order

```
Phase A: argus-core database module
  A.1  Add rusqlite dependency to argus-core/Cargo.toml
  A.2  Create argus-core/src/db.rs with DDL + write_scan + query_delta
  A.3  Add build_diff_tree (flat PathDelta → DiffNode tree)
  A.4  Add rebuild_snapshot + query_scan_timestamps + migrate_from_json
  A.5  Export db module from lib.rs
  A.6  Unit tests for all db functions
       cargo test -p argus-core

Phase B: CLI migration
  B.1  Update scan command: write to SQLite (--output keeps JSON)
  B.2  Update diff command: support --from/--to/--path time-based mode
  B.3  Add migrate command: JSON → SQLite import
  B.4  Add list-scans command
       cargo test -p argus-cli --test integration

Phase C: TUI migration
  C.1  App state: add db: Option<Connection>
  C.2  load_all_snapshots: try SQLite first, fallback to JSON
  C.3  trigger_diff_if_ready: use query_delta + build_diff_tree
  C.4  handle_message(ScanComplete): write to SQLite
       cargo test -p argus-tui

Phase D: Documentation
  D.1  Update docs/requirements/08-data-model.md (replace §3, promote §6)
  D.2  Update README.md if CLI interface changed

Final: cargo clippy && cargo fmt --check
```

## 9. Testing Strategy

| Layer | What | How |
|-------|------|-----|
| Unit | `write_scan`/`query_delta` round-trip | Create in-memory SQLite, write mock Snapshot, query back, verify PathDelta correctness |
| Unit | `build_diff_tree` from flat records | Flat input → verify DiffNode tree structure + aggregated deltas |
| Unit | Edge cases: empty tree, single file, deep nesting, no matching scans | In-memory SQLite, assert correct errors or empty results |
| Unit | `rebuild_snapshot` from path_records | Write 3 scans, rebuild latest, verify FileNode structure matches original |
| Unit | `migrate_from_json` | Write mock `.json.gz` files, migrate, verify DB contents |
| Integration | CLI `scan` → `diff --from/--to` | Create temporary DB, scan a test dir, diff by time, verify output |
| Integration | CLI `migrate` → `list-scans` | Create mock JSON files, migrate, list, verify counts |
| TUI state | `load_all_snapshots` with DB vs JSON | Mock DB path, verify snapshot_index populated correctly |

## 10. Future Considerations

- **WAL mode**: For daemon-mode (Phase 3), enable `PRAGMA journal_mode=WAL` for concurrent reads/writes
- **Retention policy**: Auto-prune old scan events after configurable days (Phase 3)
- **Full-text path search**: Enable FTS5 on path column for TUI path search (Phase 2 enhancement)
- **Incremental scan**: In daemon mode, only insert/update changed paths instead of rewriting all records (Phase 3)
- **Database migrations**: Version the schema in `scan_events` or a separate `schema_version` table for future format changes