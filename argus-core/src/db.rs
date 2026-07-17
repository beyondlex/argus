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
        ",
    )?;
    Ok(())
}

pub fn query_delta_total(
    conn: &Connection,
    path: &Path,
    from_ms: u64,
    to_ms: u64,
) -> Result<i64, DbError> {
    let path_str = path.to_string_lossy();
    let prefix = format!("{}/%", path_str);
    // IMPORTANT:
    // `is_agg = 1` rows represent subtree coverage, not extra additive events.
    // If a parent directory already has an aggregate row, descendants covered by
    // that row must not be counted again here, or the TUI will double count.
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(delta_size), 0) FROM delta_events
         WHERE (path = ?1 OR path LIKE ?2)
           AND timestamp >= ?3 AND timestamp <= ?4
           AND NOT EXISTS (
               SELECT 1
               FROM delta_events AS agg
               WHERE agg.is_agg = 1
                 AND (agg.path = ?1 OR agg.path LIKE ?2)
                 AND agg.path <> delta_events.path
                 AND delta_events.path LIKE (agg.path || '/%')
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
    let prefix = format!("{}/%", path_str);
    // Keep this filter in lockstep with `query_delta_total`.
    // The UI expects both calls to expose the same subtree coverage semantics.
    let mut stmt = conn.prepare(
        "SELECT path, delta_size, event_type, timestamp, is_agg FROM delta_events
         WHERE (path = ?1 OR path LIKE ?2)
           AND timestamp >= ?3 AND timestamp <= ?4
           AND NOT EXISTS (
               SELECT 1
               FROM delta_events AS agg
               WHERE agg.is_agg = 1
                 AND (agg.path = ?1 OR agg.path LIKE ?2)
                 AND agg.path <> delta_events.path
                 AND delta_events.path LIKE (agg.path || '/%')
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
    let prefix = format!("{}/%", path_str);
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
         WHERE (path = ?1 OR path LIKE ?2)
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

pub fn query_db_size(path: &Path) -> Result<u64, DbError> {
    let meta = fs::metadata(path)?;
    Ok(meta.len())
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
        let parent_prefix = format!("{}/%", parent);
        let nested_prefix = format!("{}/%/", parent);
        tx.execute(
            "DELETE FROM delta_events WHERE is_agg = 0 AND path LIKE ?1 AND path NOT LIKE ?2",
            params![parent_prefix, nested_prefix],
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

    #[test]
    fn test_query_delta_detail_prefers_agg_over_descendants() {
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

        let total = query_delta_total(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(total, 300);

        let entries = query_delta_detail(&conn, Path::new("/tmp/dir"), 0, 5000).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].delta_size, 300);
        assert!(entries[0].is_agg);
    }
}
