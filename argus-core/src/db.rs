use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::model::DeltaEntry;

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
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(delta_size), 0) FROM delta_events
         WHERE path = ?1 AND timestamp >= ?2 AND timestamp <= ?3",
        params![path_str.as_ref(), from_ms, to_ms],
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
    let mut stmt = conn.prepare(
        "SELECT path, delta_size, event_type, timestamp FROM delta_events
         WHERE path = ?1 AND timestamp >= ?2 AND timestamp <= ?3
         ORDER BY timestamp ASC",
    )?;

    let entries = stmt
        .query_map(params![path_str.as_ref(), from_ms, to_ms], |row| {
            Ok(DeltaEntry {
                path: PathBuf::from(row.get::<_, String>(0)?),
                delta_size: row.get(1)?,
                event_type: row.get(2)?,
                timestamp: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(entries)
}

pub fn insert_events(conn: &Connection, events: &[DeltaEntry]) -> Result<(), DbError> {
    let mut stmt = conn.prepare(
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

    Ok(())
}

pub fn purge_events_before(conn: &Connection, before_ms: u64) -> Result<u64, DbError> {
    let deleted = conn.execute(
        "DELETE FROM delta_events WHERE timestamp < ?1",
        params![before_ms],
    )?;
    Ok(deleted as u64)
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
    }

    #[test]
    fn test_insert_and_query_delta_total() {
        let (conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 50,
                event_type: "modify".into(),
                timestamp: 2000,
            },
        ];
        insert_events(&conn, &events).unwrap();

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
        let (conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: -50,
                event_type: "modify".into(),
                timestamp: 2000,
            },
        ];
        insert_events(&conn, &events).unwrap();

        let entries = query_delta_detail(&conn, Path::new("/tmp/test.txt"), 0, 3000).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].delta_size, 100);
        assert_eq!(entries[1].delta_size, -50);
    }

    #[test]
    fn test_query_delta_detail_time_bounds() {
        let (conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 50,
                event_type: "modify".into(),
                timestamp: 2000,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 200,
                event_type: "modify".into(),
                timestamp: 3000,
            },
        ];
        insert_events(&conn, &events).unwrap();

        let entries = query_delta_detail(&conn, Path::new("/tmp/test.txt"), 1500, 2500).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].delta_size, 50);
    }

    #[test]
    fn test_insert_multiple_paths() {
        let (conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/a.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/b.txt"),
                delta_size: 200,
                event_type: "create".into(),
                timestamp: 1000,
            },
        ];
        insert_events(&conn, &events).unwrap();

        let total_a = query_delta_total(&conn, Path::new("/tmp/a.txt"), 0, 3000).unwrap();
        let total_b = query_delta_total(&conn, Path::new("/tmp/b.txt"), 0, 3000).unwrap();
        assert_eq!(total_a, 100);
        assert_eq!(total_b, 200);
    }

    #[test]
    fn test_purge_events_before() {
        let (conn, _) = setup_db();

        let events = vec![
            DeltaEntry {
                path: PathBuf::from("/tmp/a.txt"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
            },
            DeltaEntry {
                path: PathBuf::from("/tmp/b.txt"),
                delta_size: 200,
                event_type: "create".into(),
                timestamp: 3000,
            },
        ];
        insert_events(&conn, &events).unwrap();

        let deleted = purge_events_before(&conn, 2000).unwrap();
        assert_eq!(deleted, 1);

        let remaining = query_delta_detail(&conn, Path::new("/tmp/b.txt"), 0, 5000).unwrap();
        assert_eq!(remaining.len(), 1);
    }
}
