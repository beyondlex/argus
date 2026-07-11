use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info};

use argus_core::{insert_events, DeltaEntry, DeltaEvent};

struct DebounceEntry {
    event: DeltaEntry,
    expires_at: Instant,
}

pub struct DebounceEngine {
    pending: HashMap<PathBuf, DebounceEntry>,
    window: Duration,
    event_rx: mpsc::Receiver<DeltaEvent>,
    db: Arc<Mutex<Connection>>,
}

impl DebounceEngine {
    pub fn new(
        window: Duration,
        event_rx: mpsc::Receiver<DeltaEvent>,
        db: Arc<Mutex<Connection>>,
    ) -> Self {
        Self {
            pending: HashMap::new(),
            window,
            event_rx,
            db,
        }
    }

    fn merge(&mut self, event: DeltaEvent) {
        let path = event.path.clone();
        let expires_at = Instant::now() + self.window;

        match self.pending.get_mut(&path) {
            Some(entry) => {
                let merged = match (entry.event.event_type.as_str(), event.event_type.as_str()) {
                    ("create", "modify") => DeltaEntry {
                        delta_size: entry.event.delta_size + event.delta_size,
                        event_type: "create".into(),
                        timestamp: event.timestamp,
                        ..entry.event.clone()
                    },
                    ("modify", "modify") => DeltaEntry {
                        delta_size: entry.event.delta_size + event.delta_size,
                        event_type: "modify".into(),
                        timestamp: event.timestamp,
                        ..entry.event.clone()
                    },
                    ("create", "delete") => {
                        self.pending.remove(&path);
                        return;
                    }
                    ("modify", "delete") => DeltaEntry {
                        delta_size: -entry.event.delta_size.abs(),
                        event_type: "delete".into(),
                        timestamp: event.timestamp,
                        path: entry.event.path.clone(),
                    },
                    _ => DeltaEntry {
                        path: event.path,
                        delta_size: event.delta_size,
                        event_type: event.event_type,
                        timestamp: event.timestamp,
                    },
                };
                entry.event = merged;
                entry.expires_at = expires_at;
            }
            None => {
                self.pending.insert(
                    path,
                    DebounceEntry {
                        event: DeltaEntry {
                            path: event.path,
                            delta_size: event.delta_size,
                            event_type: event.event_type,
                            timestamp: event.timestamp,
                        },
                        expires_at,
                    },
                );
            }
        }
    }

    async fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        let events: Vec<DeltaEntry> = self.pending.drain().map(|(_, entry)| entry.event).collect();

        let conn = self.db.lock().await;
        if let Err(e) = insert_events(&conn, &events) {
            error!("failed to insert debounced events: {e}");
            // Re-insert pending entries on failure
            for event in events {
                self.pending.insert(
                    event.path.clone(),
                    DebounceEntry {
                        expires_at: Instant::now() + Duration::from_secs(3),
                        event,
                    },
                );
            }
        } else {
            info!("flushed {} debounced events", events.len());
        }
    }

    async fn flush_expired(&mut self) {
        let now = Instant::now();
        let expired: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(path, _)| path.clone())
            .collect();

        if expired.is_empty() {
            return;
        }

        let events: Vec<DeltaEntry> = expired
            .iter()
            .filter_map(|path| self.pending.remove(path))
            .map(|entry| entry.event)
            .collect();

        let conn = self.db.lock().await;
        if let Err(e) = insert_events(&conn, &events) {
            error!("failed to insert expired events: {e}");
            for event in events {
                self.pending.insert(
                    event.path.clone(),
                    DebounceEntry {
                        expires_at: Instant::now() + Duration::from_secs(3),
                        event,
                    },
                );
            }
        }
    }

    pub async fn run(&mut self) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.tick().await;

        loop {
            tokio::select! {
                Some(event) = self.event_rx.recv() => {
                    self.merge(event);
                }
                _ = tick.tick() => {
                    self.flush_expired().await;
                }
                _ = tokio::time::sleep(Duration::from_millis(200)) => {
                    if crate::SHOULD_QUIT.load(Ordering::Relaxed) {
                        break;
                    }
                }
                else => {
                    break;
                }
            }
        }

        self.flush().await;
    }
}

pub fn start_debounce(
    event_rx: mpsc::Receiver<DeltaEvent>,
    db: Arc<Mutex<Connection>>,
    window: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut engine = DebounceEngine::new(window, event_rx, db);
        engine.run().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::tempdir;

    fn entry(path: &str, delta: i64, etype: &str, ts: u64) -> DeltaEvent {
        DeltaEvent {
            path: PathBuf::from(path),
            delta_size: delta,
            event_type: etype.into(),
            timestamp: ts,
            is_agg: false,
            process_info: None,
        }
    }

    #[test]
    fn test_create_then_modify_merges_to_create() {
        let (_tx, rx) = mpsc::channel(16);
        let db = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let mut engine = DebounceEngine::new(Duration::from_secs(10), rx, db);

        engine.merge(entry("/tmp/a.txt", 100, "create", 1000));
        engine.merge(entry("/tmp/a.txt", 50, "modify", 2000));

        assert_eq!(engine.pending.len(), 1);
        let e = &engine
            .pending
            .get(&PathBuf::from("/tmp/a.txt"))
            .unwrap()
            .event;
        assert_eq!(e.event_type, "create");
        assert_eq!(e.delta_size, 150);
        assert_eq!(e.timestamp, 2000);
    }

    #[test]
    fn test_modify_then_modify_accumulates() {
        let (_tx, rx) = mpsc::channel(16);
        let db = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let mut engine = DebounceEngine::new(Duration::from_secs(10), rx, db);

        engine.merge(entry("/tmp/b.txt", 100, "modify", 1000));
        engine.merge(entry("/tmp/b.txt", 50, "modify", 2000));

        let e = &engine
            .pending
            .get(&PathBuf::from("/tmp/b.txt"))
            .unwrap()
            .event;
        assert_eq!(e.event_type, "modify");
        assert_eq!(e.delta_size, 150);
    }

    #[test]
    fn test_create_then_delete_cancels() {
        let (_tx, rx) = mpsc::channel(16);
        let db = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let mut engine = DebounceEngine::new(Duration::from_secs(10), rx, db);

        engine.merge(entry("/tmp/c.txt", 100, "create", 1000));
        engine.merge(entry("/tmp/c.txt", -100, "delete", 2000));

        assert!(engine.pending.is_empty());
    }

    #[test]
    fn test_modify_then_delete_becomes_delete() {
        let (_tx, rx) = mpsc::channel(16);
        let db = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let mut engine = DebounceEngine::new(Duration::from_secs(10), rx, db);

        engine.merge(entry("/tmp/d.txt", 100, "modify", 1000));
        engine.merge(entry("/tmp/d.txt", -100, "delete", 2000));

        let e = &engine
            .pending
            .get(&PathBuf::from("/tmp/d.txt"))
            .unwrap()
            .event;
        assert_eq!(e.event_type, "delete");
    }

    #[test]
    fn test_different_paths_independent() {
        let (_tx, rx) = mpsc::channel(16);
        let db = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let mut engine = DebounceEngine::new(Duration::from_secs(10), rx, db);

        engine.merge(entry("/tmp/a.txt", 100, "create", 1000));
        engine.merge(entry("/tmp/b.txt", 200, "create", 1000));

        assert_eq!(engine.pending.len(), 2);
    }

    #[test]
    fn test_flush_writes_to_db() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let temp = tempdir().unwrap();
            let db_path = temp.path().join("test.db");
            let conn = Connection::open(&db_path).unwrap();
            argus_core::init_db(&conn).unwrap();
            let db = Arc::new(Mutex::new(conn));
            let (_tx, rx) = mpsc::channel(16);

            let mut engine = DebounceEngine::new(Duration::from_secs(10), rx, db.clone());
            engine.merge(entry("/tmp/flush_test.txt", 100, "create", 1000));
            engine.flush().await;

            let conn = db.lock().await;
            let total =
                argus_core::query_delta_total(&conn, Path::new("/tmp/flush_test.txt"), 0, 9999)
                    .unwrap();
            assert_eq!(total, 100);
        });
    }
}
