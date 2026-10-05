use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::model::{DeltaEntry, DeltaSummary};

#[derive(thiserror::Error, Debug)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Directory holding all argus state (DB, audit log, TUI config/log, daemon
/// config and PID file): `$XDG_CONFIG_HOME/argus`, falling back to
/// `~/.config/argus`. Resolving this in one place matters — the audit log
/// used to hardcode `~/.config` and ignore a set `XDG_CONFIG_HOME`, so the
/// DB and the audit trail landed in different trees.
pub fn config_dir() -> PathBuf {
    config_dir_from(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
    .join("argus")
}

/// Pure core of [`config_dir`] so the fallback chain is testable without
/// mutating process-wide env vars (tests share one environment).
fn config_dir_from(xdg: Option<&std::ffi::OsStr>, home: Option<&std::ffi::OsStr>) -> PathBuf {
    match xdg {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => match home {
            Some(h) => PathBuf::from(h).join(".config"),
            // No HOME (rare, e.g. stripped service env): stay relative instead
            // of panicking; every caller creates the directory on demand.
            None => PathBuf::from("."),
        },
    }
}

pub fn default_db_path() -> PathBuf {
    config_dir().join("argus.db")
}

pub fn open_db(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    init_db(&conn)?;
    Ok(conn)
}

pub fn init_db(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS delta_events (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            path        TEXT    NOT NULL,
            delta_size  INTEGER NOT NULL,
            event_type  TEXT    NOT NULL,
            timestamp   INTEGER NOT NULL,
            is_agg      INTEGER DEFAULT 0,
            process_info TEXT   DEFAULT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_delta_path_time
            ON delta_events(path, timestamp);

        CREATE INDEX IF NOT EXISTS idx_delta_timestamp
            ON delta_events(timestamp);

        CREATE INDEX IF NOT EXISTS idx_delta_is_agg
            ON delta_events(is_agg);

        -- Speeds GetDelta/detail anti-join over aggregate coverage rows
        -- (is_agg filter + path prefix + time window).
        CREATE INDEX IF NOT EXISTS idx_delta_agg_path_time
            ON delta_events(is_agg, path, timestamp);

        CREATE TABLE IF NOT EXISTS ai_analysis_cache (
            path_hash TEXT PRIMARY KEY,
            path      TEXT NOT NULL,
            data      BLOB NOT NULL,
            created_at INTEGER NOT NULL
        );
        ",
    )?;
    Ok(())
}

/// Stable hash of a path for the AI analysis cache key.
/// `DefaultHasher::new()` uses fixed SipHash keys, so the value is
/// consistent across process restarts (required for a persistent cache).
fn path_hash(path: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish().to_string()
}

pub fn set_ai_analysis(conn: &Connection, path: &str, data: &[u8]) -> Result<(), DbError> {
    let path_hash = path_hash(path);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "INSERT OR REPLACE INTO ai_analysis_cache (path_hash, path, data, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![path_hash, path, data, now],
    )?;
    Ok(())
}

pub fn get_ai_analysis(conn: &Connection, path: &str) -> Result<Option<Vec<u8>>, DbError> {
    let path_hash = path_hash(path);
    // The cache key is a 64-bit hash; the path column double-checks it so a
    // collision cannot silently return (or overwrite) another path's verdict.
    let mut stmt =
        conn.prepare("SELECT data FROM ai_analysis_cache WHERE path_hash = ?1 AND path = ?2")?;
    let mut rows = stmt.query(params![path_hash, path])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

pub fn delete_ai_analysis(conn: &Connection, path: &str) -> Result<(), DbError> {
    let path_hash = path_hash(path);
    conn.execute(
        "DELETE FROM ai_analysis_cache WHERE path_hash = ?1 AND path = ?2",
        params![path_hash, path],
    )?;
    Ok(())
}

/// Load every cached AI analysis as `(path, data)` pairs in one query.
/// Startup uses this instead of listing paths and re-querying each blob.
pub fn load_ai_cache_entries(conn: &Connection) -> Result<Vec<(String, Vec<u8>)>, DbError> {
    let mut stmt = conn.prepare("SELECT path, data FROM ai_analysis_cache")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

pub fn query_delta_total(
    conn: &Connection,
    path: &Path,
    from_ms: u64,
    to_ms: u64,
) -> Result<i64, DbError> {
    let path_str = path.to_string_lossy();
    // Prefix matching must be literal: LIKE would treat `%`/`_` in the path
    // (e.g. `my_projects`) as wildcards and pull in sibling directories such
    // as `my-dir`. `substr(path, 1, length(?2)) = ?2` is an exact byte-for-byte
    // prefix check and is also case-sensitive, unlike SQLite's ASCII-only
    // case-insensitive LIKE.
    let prefix = format!("{}/", path_str);
    // IMPORTANT:
    // `is_agg = 1` rows represent subtree coverage, not extra additive events.
    // If a parent directory already has an aggregate row, descendants covered by
    // that row must not be counted again here, or the TUI will double count.
    // Aggregation is parent-local (see `consolidate_events`): an agg row sums
    // exactly the raw direct-children events that existed when consolidation
    // ran — never deeper descendants, and never events recorded afterwards.
    // The anti-join therefore hides a raw event only when ALL of these hold:
    //   1. it is a *direct child* of an in-subtree agg path. A raw grandchild
    //      under an aggregated directory is NOT in the agg sum; suppressing it
    //      (the old any-depth match) lost those deltas permanently whenever the
    //      grandchild's own parent never consolidated. Agg rows also do not
    //      suppress each other for the same reason.
    //   2. the event's ts <= agg.ts. agg.ts is the max ts of consolidated
    //      children, so anything recorded later arrived after consolidation
    //      and must stay visible instead of waiting (up to a consolidation
    //      interval) to be folded in.
    //   3. agg.timestamp <= ?4 — an agg beyond the window's upper bound is
    //      itself excluded by the outer WHERE, so it must not suppress.
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(delta_size), 0) FROM delta_events
         WHERE (path = ?1 OR substr(path, 1, length(?2)) = ?2)
           AND timestamp >= ?3 AND timestamp <= ?4
           AND NOT EXISTS (
               SELECT 1
               FROM delta_events AS agg
               WHERE agg.is_agg = 1
                 AND delta_events.is_agg = 0
                 AND (agg.path = ?1 OR substr(agg.path, 1, length(?2)) = ?2)
                 AND substr(delta_events.path, 1, length(agg.path) + 1) = agg.path || '/'
                 AND instr(substr(delta_events.path, length(agg.path) + 2), '/') = 0
                 AND delta_events.timestamp <= agg.timestamp
                 AND agg.timestamp <= ?4
           )",
        params![path_str.as_ref(), prefix, from_ms, to_ms],
        |row| row.get(0),
    )?;
    Ok(total)
}

pub fn query_delta_detail(
    conn: &Connection,
    path: &Path,
    from_ms: u64,
    to_ms: u64,
) -> Result<Vec<DeltaEntry>, DbError> {
    let path_str = path.to_string_lossy();
    // Keep this filter in lockstep with `query_delta_total` (same agg
    // direct-children-only suppression invariant and literal substr prefix
    // matching — see the comment there for why LIKE is not used).
    let prefix = format!("{}/", path_str);
    let mut stmt = conn.prepare(
        "SELECT path, delta_size, event_type, timestamp, is_agg FROM delta_events
         WHERE (path = ?1 OR substr(path, 1, length(?2)) = ?2)
           AND timestamp >= ?3 AND timestamp <= ?4
           AND NOT EXISTS (
               SELECT 1
               FROM delta_events AS agg
               WHERE agg.is_agg = 1
                 AND delta_events.is_agg = 0
                 AND (agg.path = ?1 OR substr(agg.path, 1, length(?2)) = ?2)
                 AND substr(delta_events.path, 1, length(agg.path) + 1) = agg.path || '/'
                 AND instr(substr(delta_events.path, length(agg.path) + 2), '/') = 0
                 AND delta_events.timestamp <= agg.timestamp
                 AND agg.timestamp <= ?4
           )
         ORDER BY timestamp ASC",
    )?;

    let entries = stmt
        .query_map(params![path_str.as_ref(), prefix, from_ms, to_ms], |row| {
            Ok(DeltaEntry {
                path: PathBuf::from(row.get::<_, String>(0)?),
                delta_size: row.get(1)?,
                event_type: row.get(2)?,
                timestamp: row.get(3)?,
                is_agg: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(entries)
}

/// Summarize raw delta events for a path prefix and time window.
///
/// This is a diagnostic aggregate: it does not expand row-by-row items, so it is
/// suitable for checking whether deletes were recorded and how much churn a
/// subtree produced.
pub fn query_delta_summary(
    conn: &Connection,
    path: &Path,
    from_ms: u64,
    to_ms: u64,
) -> Result<DeltaSummary, DbError> {
    let path_str = path.to_string_lossy();
    // Literal prefix matching, same rationale as `query_delta_total`.
    let prefix = format!("{}/", path_str);
    conn.query_row(
        "SELECT
            COUNT(*) AS event_count,
            COALESCE(SUM(CASE WHEN event_type = 'create' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN event_type = 'modify' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN event_type = 'delete' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN is_agg = 1 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN delta_size > 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN delta_size < 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN delta_size = 0 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(delta_size), 0),
            COALESCE(SUM(CASE WHEN delta_size > 0 THEN delta_size ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN delta_size < 0 THEN delta_size ELSE 0 END), 0)
         FROM delta_events
         WHERE (path = ?1 OR substr(path, 1, length(?2)) = ?2)
           AND timestamp >= ?3 AND timestamp <= ?4",
        params![path_str.as_ref(), prefix, from_ms, to_ms],
        |row| {
            Ok(DeltaSummary {
                event_count: row.get::<_, u64>(0)?,
                create_count: row.get::<_, u64>(1)?,
                modify_count: row.get::<_, u64>(2)?,
                delete_count: row.get::<_, u64>(3)?,
                agg_count: row.get::<_, u64>(4)?,
                positive_events: row.get::<_, u64>(5)?,
                negative_events: row.get::<_, u64>(6)?,
                zero_events: row.get::<_, u64>(7)?,
                total_delta: row.get::<_, i64>(8)?,
                positive_delta: row.get::<_, i64>(9)?,
                negative_delta: row.get::<_, i64>(10)?,
            })
        },
    )
    .map_err(DbError::from)
}

pub fn insert_events(conn: &mut Connection, events: &[DeltaEntry]) -> Result<(), DbError> {
    if events.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for event in events {
            let path_str = event.path.to_string_lossy();
            stmt.execute(params![
                path_str.as_ref(),
                event.delta_size,
                event.event_type,
                event.timestamp,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn purge_events_before(conn: &Connection, before_ms: u64) -> Result<u64, DbError> {
    let deleted = conn.execute(
        "DELETE FROM delta_events WHERE timestamp < ?1",
        params![before_ms],
    )?;
    Ok(deleted as u64)
}

pub fn query_event_count(conn: &Connection) -> Result<u64, DbError> {
    let count: u64 = conn.query_row("SELECT COUNT(*) FROM delta_events", [], |row| row.get(0))?;
    Ok(count)
}

/// On-disk size of the delta DB, including the WAL sidecar when present.
/// The daemon keeps a connection open in WAL mode, so un-checkpointed pages
/// (routine with a live writer) live in `<path>-wal`; reporting only the main
/// file understated the status panel's "db size" between checkpoints.
pub fn query_db_size(path: &Path) -> Result<u64, DbError> {
    let mut total = fs::metadata(path)?.len();
    if let Ok(wal) = fs::metadata(wal_path(path)) {
        total = total.saturating_add(wal.len());
    }
    Ok(total)
}

fn wal_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push("-wal");
    PathBuf::from(s)
}

pub fn clear_all_events(conn: &Connection) -> Result<u64, DbError> {
    let deleted = conn.execute("DELETE FROM delta_events", [])?;
    Ok(deleted as u64)
}

pub fn consolidate_events(conn: &mut Connection, threshold: u64) -> Result<u64, DbError> {
    // Stream rows into per-parent aggregates (count/sum/max_ts) instead of
    // materializing every event id in RAM.
    let mut parent_map: HashMap<String, (u64, i64, u64)> = HashMap::new();
    {
        let mut stmt =
            conn.prepare("SELECT path, delta_size, timestamp FROM delta_events WHERE is_agg = 0")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })?;
        for row in rows {
            let (path_str, delta, ts) = row?;
            let path = Path::new(&path_str);
            let Some(parent) = path.parent() else {
                continue;
            };
            if parent.as_os_str().is_empty() {
                continue;
            }
            // The filesystem root can never aggregate: the delete below would
            // build the child prefix `//` (matching nothing, leaving every raw
            // event in place) while the insert still wrote an unreachable agg
            // row at "/" (the "/" query prefix is itself "//") and the
            // consolidated count counted rows it never removed. Skip instead;
            // root-level events simply stay raw.
            if parent == Path::new("/") {
                continue;
            }
            let entry = parent_map
                .entry(parent.to_string_lossy().into_owned())
                .or_insert((0, 0, 0));
            entry.0 = entry.0.saturating_add(1);
            entry.1 = entry.1.saturating_add(delta);
            if ts > entry.2 {
                entry.2 = ts;
            }
        }
    }

    let tx = conn.transaction()?;
    let mut total_consolidated: u64 = 0;

    for (parent, &(count, total_delta, max_ts)) in &parent_map {
        if count <= threshold {
            continue;
        }

        // We intentionally keep aggregation local to one parent path.
        // Do not try to infer or merge descendant aggregate rows here; the
        // query layer treats each aggregate row as a subtree-wide coverage value.
        // Direct children are matched with literal substr/instr prefix checks —
        // LIKE here would treat `%`/`_` in the parent path as wildcards and
        // DELETE rows belonging to sibling directories (e.g. `my_dir` eating
        // `my-dir`'s events).
        let child_prefix = format!("{parent}/");
        tx.execute(
            "DELETE FROM delta_events
             WHERE is_agg = 0
               AND substr(path, 1, length(?1)) = ?1
               AND instr(substr(path, length(?1) + 1), '/') = 0",
            params![child_prefix],
        )?;
        total_consolidated = total_consolidated.saturating_add(count);

        match tx.query_row(
            "SELECT id FROM delta_events WHERE path = ?1 AND is_agg = 1",
            params![parent],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(existing_id) => {
                tx.execute(
                    "UPDATE delta_events SET delta_size = delta_size + ?1, timestamp = MAX(timestamp, ?2) WHERE id = ?3",
                    params![total_delta, max_ts, existing_id],
                )?;
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                tx.execute(
                    "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg) VALUES (?1, ?2, 'agg', ?3, 1)",
                    params![parent, total_delta, max_ts],
                )?;
            }
            Err(e) => return Err(DbError::Sqlite(e)),
        }
    }

    tx.commit()?;
    Ok(total_consolidated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_db() -> (Connection, PathBuf) {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("test.db");
        let conn = open_db(&db_path).unwrap();
        (conn, db_path)
    }

    #[test]
    fn test_default_db_path() {
        let path = default_db_path();
        assert!(path.ends_with("argus.db"));
    }

    /// XDG wins over HOME; an empty XDG counts as unset (some launchers
    /// export it as ""). Returns the config *root* — the `argus/` leaf is
    /// appended by [`config_dir`].
    #[test]
    fn test_config_dir_from_prefers_xdg() {
        use std::ffi::OsStr;
        let xdg = OsStr::new("/custom/xdg");
        let home = OsStr::new("/home/u");
        assert_eq!(
            config_dir_from(Some(xdg), Some(home)),
            PathBuf::from("/custom/xdg")
        );
        assert_eq!(
            config_dir_from(Some(OsStr::new("")), Some(home)),
            PathBuf::from("/home/u/.config")
        );
        assert_eq!(
            config_dir_from(None, Some(home)),
            PathBuf::from("/home/u/.config")
        );
        // Neither set: relative fallback instead of a panic.
        assert_eq!(config_dir_from(None, None), PathBuf::from("."));
    }

    #[test]
    fn test_open_db_creates_file() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("argus.db");
        let conn = open_db(&db_path).unwrap();
        let val: i32 = conn.query_row("SELECT 1", [], |r| r.get(0)).unwrap();
        assert_eq!(val, 1);
    }

    #[test]
    fn test_init_db_creates_tables() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("test.db");
        let conn = open_db(&db_path).unwrap();

        let table_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='delta_events'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 1);

        let idx_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_delta_agg_path_time'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 1);
    }

    #[test]
    fn test_insert_and_query_delta_total() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 50,
                event_type: "modify".into(),
                timestamp: 2000,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let total = query_delta_total(&conn, Path::new("/tmp/test.txt"), 0, 3000).unwrap();
        assert_eq!(total, 150);
    }

    #[test]
    fn test_query_delta_total_empty_range() {
        let (conn, _) = setup_db();
        let total = query_delta_total(&conn, Path::new("/tmp/nonexistent"), 0, 3000).unwrap();
        assert_eq!(total, 0);
    }

    #[test]
    fn test_query_delta_detail() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: -50,
                event_type: "modify".into(),
                timestamp: 2000,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let entries = query_delta_detail(&conn, Path::new("/tmp/test.txt"), 0, 3000).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].delta_size, 100);
        assert_eq!(entries[1].delta_size, -50);
    }

    #[test]
    fn test_query_delta_detail_time_bounds() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 50,
                event_type: "modify".into(),
                timestamp: 2000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 200,
                event_type: "modify".into(),
                timestamp: 3000,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let entries = query_delta_detail(&conn, Path::new("/tmp/test.txt"), 1500, 2500).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].delta_size, 50);
    }

    #[test]
    fn test_query_delta_summary() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/file_a.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/file_a.txt"),
                delta_size: 50,
                event_type: "modify".into(),
                timestamp: 2000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/file_b.txt"),
                delta_size: -25,
                event_type: "delete".into(),
                timestamp: 2500,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let summary = query_delta_summary(&conn, Path::new("/tmp/dir"), 0, 3000).unwrap();
        assert_eq!(summary.event_count, 3);
        assert_eq!(summary.create_count, 1);
        assert_eq!(summary.modify_count, 1);
        assert_eq!(summary.delete_count, 1);
        assert_eq!(summary.agg_count, 0);
        assert_eq!(summary.positive_events, 2);
        assert_eq!(summary.negative_events, 1);
        assert_eq!(summary.zero_events, 0);
        assert_eq!(summary.total_delta, 125);
        assert_eq!(summary.positive_delta, 150);
        assert_eq!(summary.negative_delta, -25);
    }

    #[test]
    fn test_insert_multiple_paths() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/a.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/b.txt"),
                delta_size: 200,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let total_a = query_delta_total(&conn, Path::new("/tmp/a.txt"), 0, 3000).unwrap();
        let total_b = query_delta_total(&conn, Path::new("/tmp/b.txt"), 0, 3000).unwrap();
        assert_eq!(total_a, 100);
        assert_eq!(total_b, 200);
    }

    #[test]
    fn test_purge_events_before() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/a.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/b.txt"),
                delta_size: 200,
                event_type: "create".into(),
                timestamp: 3000,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let deleted = purge_events_before(&conn, 2000).unwrap();
        assert_eq!(deleted, 1);

        let remaining = query_delta_detail(&conn, Path::new("/tmp/b.txt"), 0, 5000).unwrap();
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_query_delta_total_prefix_matches_children() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/file_a.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/sub/file_b.txt"),
                delta_size: 200,
                event_type: "create".into(),
                timestamp: 2000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/file_c.txt"),
                delta_size: -50,
                event_type: "delete".into(),
                timestamp: 3000,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let total = query_delta_total(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(total, 250);

        let total_root = query_delta_total(&conn, Path::new("/tmp"), 0, 5000).unwrap();
        assert_eq!(total_root, 250);
    }

    #[test]
    fn test_consolidate_below_threshold_does_nothing() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/a.txt"),
                delta_size: 10,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/b.txt"),
                delta_size: 20,
                event_type: "create".into(),
                timestamp: 2000,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let consolidated = consolidate_events(&mut conn, 10).unwrap();
        assert_eq!(consolidated, 0);

        let remaining = query_delta_detail(&conn, Path::new("/tmp"), 0, 9999).unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn test_consolidate_exceeds_threshold_aggregates() {
        let (mut conn, _) = setup_db();

        let mut events = Vec::new();
        for i in 0..15 {
            events.push(DeltaEntry {
                path: PathBuf::from(format!("/tmp/dir/file_{}.txt", i)),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000 + i as u64,
                is_agg: false,
            });
        }
        insert_events(&mut conn, &events).unwrap();

        let consolidated = consolidate_events(&mut conn, 10).unwrap();
        assert_eq!(consolidated, 15);

        let entries = query_delta_detail(&conn, Path::new("/tmp"), 0, 9999).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/tmp/dir"));
        assert_eq!(entries[0].delta_size, 1500);
    }

    #[test]
    fn test_consolidate_accumulates_into_existing_agg() {
        let (mut conn, _) = setup_db();

        conn.execute(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg) VALUES (?1, ?2, ?3, ?4, 1)",
            params!["/tmp/dir", 500, "agg", 5000],
        ).unwrap();

        let mut events = Vec::new();
        for i in 0..15 {
            events.push(DeltaEntry {
                path: PathBuf::from(format!("/tmp/dir/file_{}.txt", i)),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000 + i as u64,
                is_agg: false,
            });
        }
        insert_events(&mut conn, &events).unwrap();

        let consolidated = consolidate_events(&mut conn, 10).unwrap();
        assert_eq!(consolidated, 15);

        let entries = query_delta_detail(&conn, Path::new("/tmp"), 0, 9999).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/tmp/dir"));
        assert_eq!(entries[0].delta_size, 2000);
        assert!(entries[0].is_agg);
    }

    #[test]
    fn test_consolidate_only_direct_children() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/a.txt"),
                delta_size: 10,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/sub/b.txt"),
                delta_size: 20,
                event_type: "create".into(),
                timestamp: 2000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/sub/c.txt"),
                delta_size: 30,
                event_type: "create".into(),
                timestamp: 3000,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let consolidated = consolidate_events(&mut conn, 1).unwrap();
        assert_eq!(consolidated, 2);

        let entries = query_delta_detail(&conn, Path::new("/tmp"), 0, 9999).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_consolidate_skips_agg_entries() {
        let (mut conn, _) = setup_db();

        conn.execute(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg) VALUES (?1, ?2, ?3, ?4, 1)",
            params!["/tmp/dir", 999, "agg", 5000],
        ).unwrap();

        let consolidated = consolidate_events(&mut conn, 1).unwrap();
        assert_eq!(consolidated, 0);

        let entries = query_delta_detail(&conn, Path::new("/tmp"), 0, 9999).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_consolidate_empty_db() {
        let (mut conn, _) = setup_db();
        let consolidated = consolidate_events(&mut conn, 1).unwrap();
        assert_eq!(consolidated, 0);
    }

    /// An aggregate whose timestamp is beyond the queried window must not
    /// suppress in-window descendant events: the agg itself is excluded by the
    /// outer WHERE, so suppression would silently undercount the window.
    #[test]
    fn test_window_excluding_agg_still_counts_in_window_children() {
        let (mut conn, _) = setup_db();

        conn.execute(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg) VALUES (?1, ?2, ?3, ?4, 1)",
            params!["/tmp/dir", 999, "agg", 5000],
        )
        .unwrap();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/a.bin"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/b.bin"),
                delta_size: 50,
                event_type: "create".into(),
                timestamp: 2000,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let total = query_delta_total(&conn, Path::new("/tmp/dir"), 0, 3000).unwrap();
        assert_eq!(total, 150);

        let entries = query_delta_detail(&conn, Path::new("/tmp/dir"), 0, 3000).unwrap();
        assert_eq!(entries.len(), 2);
    }

    /// Aggregate inside the window keeps suppressing descendants (no double
    /// counting) even when descendants carry later timestamps than the agg.
    #[test]
    fn test_window_including_agg_suppresses_descendants() {
        let (mut conn, _) = setup_db();

        conn.execute(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg) VALUES (?1, ?2, ?3, ?4, 1)",
            params!["/tmp/dir", 300, "agg", 1200],
        )
        .unwrap();

        let events = vec![DeltaEntry {
            path: PathBuf::from("/tmp/dir/a.bin"),
            delta_size: 100,
            event_type: "create".into(),
            timestamp: 1000,
            is_agg: false,
        }];
        insert_events(&mut conn, &events).unwrap();

        let total = query_delta_total(&conn, Path::new("/tmp/dir"), 0, 3000).unwrap();
        assert_eq!(total, 300);
    }

    /// An aggregate row covers exactly its direct-children events: a raw
    /// direct child at or below the agg's ts is suppressed (no double count),
    /// while a raw *grandchild* — never folded into the agg by the
    /// parent-local consolidation — stays visible. The old any-depth
    /// descendant match suppressed the grandchild too, losing its delta for
    /// as long as the agg stayed in the window.
    #[test]
    fn test_agg_covers_direct_children_only() {
        let (mut conn, _) = setup_db();

        conn.execute(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg) VALUES (?1, ?2, ?3, ?4, 1)",
            params!["/tmp/dir", 300, "agg", 1200],
        )
        .unwrap();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/leaf-a.bin"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/dir/nested/leaf-b.bin"),
                delta_size: 200,
                event_type: "create".into(),
                timestamp: 1100,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        // 300 (agg) + 200 (grandchild raw); the covered direct child is hidden.
        let total = query_delta_total(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(total, 500);

        let entries = query_delta_detail(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(entries.len(), 2);
        // Timestamp order: the grandchild raw row precedes the agg row.
        assert_eq!(entries[0].path, PathBuf::from("/tmp/dir/nested/leaf-b.bin"));
        assert!(!entries[0].is_agg);
        assert!(entries[1].is_agg);
        assert_eq!(entries[1].delta_size, 300);
    }

    /// A raw event recorded *after* consolidation ran (ts beyond agg.ts) is
    /// not in the agg sum and must be visible immediately. The old query hid
    /// it until the next consolidation folded it in (up to a full interval of
    /// invisible churn in the TUI).
    #[test]
    fn test_post_consolidation_event_visible_immediately() {
        let (mut conn, _) = setup_db();

        conn.execute(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg) VALUES (?1, ?2, ?3, ?4, 1)",
            params!["/tmp/dir", 300, "agg", 1200],
        )
        .unwrap();

        let events = vec![DeltaEntry {
            path: PathBuf::from("/tmp/dir/new.bin"),
            delta_size: 100,
            event_type: "create".into(),
            timestamp: 2000,
            is_agg: false,
        }];
        insert_events(&mut conn, &events).unwrap();

        let total = query_delta_total(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(total, 400);
    }

    /// End-to-end: consolidate a directory, then record a fresh event under
    /// it. The agg keeps covering the consolidated children while the new
    /// event adds on top — no double count, no visibility lag.
    #[test]
    fn test_consolidate_then_new_event_adds_on_top() {
        let (mut conn, _) = setup_db();

        let mut events = Vec::new();
        for i in 0..15 {
            events.push(DeltaEntry {
                path: PathBuf::from(format!("/tmp/dir/file_{}.txt", i)),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000 + i as u64,
                is_agg: false,
            });
        }
        insert_events(&mut conn, &events).unwrap();
        let consolidated = consolidate_events(&mut conn, 10).unwrap();
        assert_eq!(consolidated, 15);

        // New churn after consolidation, same subtree (ts beyond the agg's
        // max consolidated child ts of 1014).
        let later = vec![DeltaEntry {
            path: PathBuf::from("/tmp/dir/late.txt"),
            delta_size: 250,
            event_type: "create".into(),
            timestamp: 1015,
            is_agg: false,
        }];
        insert_events(&mut conn, &later).unwrap();

        let total = query_delta_total(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(total, 1500 + 250);

        let entries = query_delta_detail(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(entries.len(), 2);
    }

    /// Aggregate rows must not suppress each other. Consolidation is
    /// parent-local: `/tmp/dir`'s agg only sums its direct children, so the
    /// events consolidated into `/tmp/dir/sub`'s own agg are NOT included in
    /// it. If the ancestor agg suppressed the descendant agg, that delta
    /// would vanish permanently from ancestor queries.
    #[test]
    fn test_nested_aggs_both_counted_at_ancestor() {
        let (conn, _) = setup_db();

        for (path, delta, ts) in [
            ("/tmp/dir", 300i64, 1200u64), // direct-children agg for dir
            ("/tmp/dir/sub", 700, 1300),   // direct-children agg for sub
        ] {
            conn.execute(
                "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg)
                 VALUES (?1, ?2, 'agg', ?3, 1)",
                params![path, delta, ts],
            )
            .unwrap();
        }

        let total = query_delta_total(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(total, 1000);

        let entries = query_delta_detail(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(entries.len(), 2);

        // Querying the subtree itself still sees its own agg only.
        let sub_total = query_delta_total(&conn, Path::new("/tmp/dir/sub"), 0, 5000).unwrap();
        assert_eq!(sub_total, 700);
    }

    /// Sibling directories whose names differ only where the queried path
    /// contains a LIKE wildcard (`_` matches any char) must not leak into the
    /// query. `my_projects` and `my-projects` coexisting is ordinary; LIKE
    /// prefix matching pulled `my-projects` events into `my_projects` totals.
    #[test]
    fn test_query_excludes_wildcard_sibling_paths() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/my_projects/a.bin"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/my-projects/b.bin"),
                delta_size: 999,
                event_type: "create".into(),
                timestamp: 1100,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        let total = query_delta_total(&conn, Path::new("/tmp/my_projects"), 0, 5000).unwrap();
        assert_eq!(total, 100);

        let entries = query_delta_detail(&conn, Path::new("/tmp/my_projects"), 0, 5000).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/tmp/my_projects/a.bin"));

        // The underscore path itself must not be suppressed by an aggregate
        // over a wildcard-matching sibling directory.
        conn.execute(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg)
             VALUES (?1, 555, 'agg', 1200, 1)",
            params!["/tmp/my-projects"],
        )
        .unwrap();
        let total = query_delta_total(&conn, Path::new("/tmp/my_projects"), 0, 5000).unwrap();
        assert_eq!(total, 100);
    }

    /// Consolidation deletes only the target directory's direct children;
    /// similarly-named sibling directories must keep their events.
    #[test]
    fn test_consolidate_does_not_touch_wildcard_siblings() {
        let (mut conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/my_dir/a.bin"),
                delta_size: 10,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/my-dir/b.bin"),
                delta_size: 20,
                event_type: "create".into(),
                timestamp: 1100,
                is_agg: false,
            },
        ];
        insert_events(&mut conn, &events).unwrap();

        consolidate_events(&mut conn, 1).unwrap();

        let entries = query_delta_detail(&conn, Path::new("/tmp/my-dir"), 0, 5000).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/tmp/my-dir/b.bin"));
    }

    /// Filesystem-root children must never aggregate: the delete prefix would
    /// be `//` (matching nothing, every raw row stays) while the agg insert
    /// still wrote an aggregate at "/" — a junk row no query can ever reach
    /// (the "/" prefix is "//") plus an over-reported consolidated count.
    #[test]
    fn test_consolidate_skips_filesystem_root_parent() {
        let (mut conn, _) = setup_db();

        let mut events = Vec::new();
        for i in 0..5 {
            events.push(DeltaEntry {
                path: PathBuf::from(format!("/top-{}.bin", i)),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000 + i as u64,
                is_agg: false,
            });
        }
        insert_events(&mut conn, &events).unwrap();

        let consolidated = consolidate_events(&mut conn, 1).unwrap();
        assert_eq!(consolidated, 0, "root parent must not aggregate");

        // Raw rows stay, and no unreachable agg row at "/" appeared.
        let entries = query_delta_detail(&conn, Path::new("/top-0.bin"), 0, 9999).unwrap();
        assert_eq!(entries.len(), 1);

        let agg_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM delta_events WHERE is_agg = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(agg_count, 0, "no aggregate row may be written for /");
    }

    /// The reported size must include the `-wal` sidecar: a live WAL-mode
    /// writer keeps un-checkpointed pages there, so main-file-only reporting
    /// understates the delta log's actual footprint.
    #[test]
    fn test_query_db_size_includes_wal() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("size.db");
        fs::write(&db_path, [0u8; 100]).unwrap();
        assert_eq!(query_db_size(&db_path).unwrap(), 100);

        fs::write(temp.path().join("size.db-wal"), [0u8; 40]).unwrap();
        assert_eq!(query_db_size(&db_path).unwrap(), 140);
    }

    #[test]
    fn test_path_hash_is_stable() {
        assert_eq!(path_hash("/tmp/a"), path_hash("/tmp/a"));
        assert_ne!(path_hash("/tmp/a"), path_hash("/tmp/b"));
    }
}
