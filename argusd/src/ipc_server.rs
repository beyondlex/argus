use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use argus_core::{
    clear_all_events, consolidate_events, query_db_size, query_delta_detail, query_delta_total,
    query_event_count, DaemonRequest, DaemonResponse,
};

#[derive(Clone)]
pub struct ServerConfig {
    pub watch_dirs: Vec<PathBuf>,
    pub log_level: Option<String>,
    pub debounce_seconds: u64,
    pub delta_retention_days: u64,
    pub db_path: PathBuf,
}

pub fn start_ipc_server(
    uds_path: &str,
    db: Arc<Mutex<Connection>>,
    cfg: ServerConfig,
) -> tokio::task::JoinHandle<()> {
    let path = uds_path.to_string();
    let start_time = Instant::now();
    let start_time_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    tokio::spawn(async move {
        if let Err(e) = run_ipc_server(&path, db, start_time, start_time_secs, cfg).await {
            error!("IPC server error: {e}");
        }
    })
}

async fn run_ipc_server(
    uds_path: &str,
    db: Arc<Mutex<Connection>>,
    start_time: Instant,
    start_time_secs: u64,
    cfg: ServerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(uds_path);

    if path.exists() {
        std::fs::remove_file(&path)?;
        info!("removed stale socket at {uds_path}");
    }

    let listener = UnixListener::bind(&path)?;
    info!("IPC server listening on {uds_path}");

    loop {
        tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((mut stream, _addr)) => {
                                let db = db.clone();
                                let start = start_time;
                                let cfg = cfg.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = handle_connection(&mut stream, db, start, start_time_secs, cfg).await {
                                        warn!("connection error: {e}");
                                    }
                                });
                            }
                            Err(e) => {
                                error!("accept error: {e}");
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {
                        if crate::SHOULD_QUIT.load(Ordering::Relaxed) {
                            info!("IPC server shutting down");
        break Ok(());
                        }
                    }
                }
    }
}

async fn handle_connection(
    stream: &mut tokio::net::UnixStream,
    db: Arc<Mutex<Connection>>,
    start_time: Instant,
    start_time_secs: u64,
    cfg: ServerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let payload_len = u32::from_be_bytes(len_buf) as usize;

        let mut payload = vec![0u8; payload_len];
        stream.read_exact(&mut payload).await?;

        let request: DaemonRequest = bincode::deserialize(&payload)?;

        let response = match request {
            DaemonRequest::Ping => DaemonResponse::Pong,
            DaemonRequest::GetStatus => {
                let uptime = start_time.elapsed().as_secs();
                let conn = db.lock().await;
                let db_event_count = query_event_count(&conn).unwrap_or(0);
                drop(conn);
                let db_size_bytes = query_db_size(&cfg.db_path).unwrap_or(0);
                DaemonResponse::Status {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    watch_dirs: cfg.watch_dirs.clone(),
                    uptime_secs: uptime,
                    start_time_secs,
                    log_level: cfg.log_level.clone(),
                    debounce_seconds: cfg.debounce_seconds,
                    delta_retention_days: cfg.delta_retention_days,
                    db_event_count,
                    db_size_bytes,
                }
            }
            DaemonRequest::GetDelta {
                path,
                from_ms,
                to_ms,
                include_entries,
            } => handle_get_delta(&db, path, from_ms, to_ms, include_entries).await,
            DaemonRequest::GetDeltaDetail {
                path,
                from_ms,
                to_ms,
            } => {
                let conn = db.lock().await;
                match query_delta_detail(&conn, &path, from_ms, to_ms) {
                    Ok(entries) => DaemonResponse::DeltaDetail { entries },
                    Err(e) => DaemonResponse::Error {
                        message: e.to_string(),
                    },
                }
            }
            DaemonRequest::RequestConsolidation => {
                let mut conn = db.lock().await;
                let threshold = 500;
                match consolidate_events(&mut conn, threshold) {
                    Ok(count) => DaemonResponse::ConsolidationDone {
                        consolidated_count: count,
                    },
                    Err(e) => DaemonResponse::Error {
                        message: e.to_string(),
                    },
                }
            }
            DaemonRequest::ClearDb => {
                let conn = db.lock().await;
                match clear_all_events(&conn) {
                    Ok(deleted) => DaemonResponse::DbCleared {
                        deleted_count: deleted,
                    },
                    Err(e) => DaemonResponse::Error {
                        message: e.to_string(),
                    },
                }
            }
        };

        let response_bytes = bincode::serialize(&response)?;
        let len = (response_bytes.len() as u32).to_be_bytes();
        stream.write_all(&len).await?;
        stream.write_all(&response_bytes).await?;
        stream.flush().await?;
    }

    Ok(())
}

async fn handle_get_delta(
    db: &Arc<Mutex<Connection>>,
    path: PathBuf,
    from_ms: u64,
    to_ms: u64,
    include_entries: bool,
) -> DaemonResponse {
    let conn = db.lock().await;
    match query_delta_total(&conn, &path, from_ms, to_ms) {
        Ok(total_delta) => {
            let entries = if include_entries {
                query_delta_detail(&conn, &path, from_ms, to_ms).unwrap_or_default()
            } else {
                Vec::new()
            };
            DaemonResponse::Delta {
                total_delta,
                entries,
            }
        }
        Err(e) => DaemonResponse::Error {
            message: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argus_core::{insert_events, open_db, DeltaEntry};
    use tempfile::tempdir;

    #[test]
    fn test_uds_path_location() {
        let path = argus_core::DEFAULT_UDS_PATH;
        assert!(path.contains("argusd.sock"));
    }

    #[tokio::test]
    async fn test_get_delta_totals_only_returns_empty_entries() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("test.db");
        let mut conn = open_db(&db_path).unwrap();
        insert_events(
            &mut conn,
            &[DeltaEntry {
                path: PathBuf::from("/tmp/a.txt"),
                delta_size: 42,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            }],
        )
        .unwrap();
        drop(conn);

        let db = Arc::new(Mutex::new(open_db(&db_path).unwrap()));
        let resp = handle_get_delta(&db, PathBuf::from("/tmp"), 0, 9999, false).await;
        match resp {
            DaemonResponse::Delta {
                total_delta,
                entries,
            } => {
                assert_eq!(total_delta, 42);
                assert!(entries.is_empty());
            }
            other => panic!("unexpected response: {other:?}"),
        }

        let resp_full = handle_get_delta(&db, PathBuf::from("/tmp"), 0, 9999, true).await;
        match resp_full {
            DaemonResponse::Delta {
                total_delta,
                entries,
            } => {
                assert_eq!(total_delta, 42);
                assert_eq!(entries.len(), 1);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }
}
