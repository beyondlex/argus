use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use rusqlite::Connection;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use argus_core::{query_delta_detail, query_delta_total, DaemonRequest, DaemonResponse};

pub fn start_ipc_server(uds_path: &str, db: Arc<Mutex<Connection>>) -> tokio::task::JoinHandle<()> {
    let path = uds_path.to_string();
    let start_time = Instant::now();

    tokio::spawn(async move {
        if let Err(e) = run_ipc_server(&path, db, start_time).await {
            error!("IPC server error: {e}");
        }
    })
}

async fn run_ipc_server(
    uds_path: &str,
    db: Arc<Mutex<Connection>>,
    start_time: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(uds_path);

    if path.exists() {
        std::fs::remove_file(&path)?;
        info!("removed stale socket at {uds_path}");
    }

    let listener = UnixListener::bind(&path)?;
    info!("IPC server listening on {uds_path}");

    loop {
        match listener.accept().await {
            Ok((mut stream, _addr)) => {
                let db = db.clone();
                let start = start_time;
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(&mut stream, db, start).await {
                        warn!("connection error: {e}");
                    }
                });
            }
            Err(e) => {
                error!("accept error: {e}");
            }
        }
    }
}

async fn handle_connection(
    stream: &mut tokio::net::UnixStream,
    db: Arc<Mutex<Connection>>,
    start_time: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let payload_len = u32::from_be_bytes(len_buf) as usize;

    let mut payload = vec![0u8; payload_len];
    stream.read_exact(&mut payload).await?;

    let request: DaemonRequest = bincode::deserialize(&payload)?;

    let response = match request {
        DaemonRequest::Ping => DaemonResponse::Pong,
        DaemonRequest::GetStatus => {
            let conn = db.lock().await;
            let uptime = start_time.elapsed().as_secs();
            drop(conn);
            DaemonResponse::Status {
                version: env!("CARGO_PKG_VERSION").to_string(),
                watch_dirs: Vec::new(),
                uptime_secs: uptime,
            }
        }
        DaemonRequest::GetDelta {
            path,
            from_ms,
            to_ms,
        } => {
            let conn = db.lock().await;
            match query_delta_total(&conn, &path, from_ms, to_ms) {
                Ok(total_delta) => {
                    let entries =
                        query_delta_detail(&conn, &path, from_ms, to_ms).unwrap_or_default();
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
    };

    let response_bytes = bincode::serialize(&response)?;
    let len = (response_bytes.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&response_bytes).await?;
    stream.flush().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_uds_path_location() {
        let path = argus_core::DEFAULT_UDS_PATH;
        assert!(path.contains("argusd.sock"));
    }
}
