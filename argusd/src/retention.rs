use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;
use tokio::sync::Mutex;
use tokio::time;

use argus_core::{consolidate_events, purge_events_before};

use crate::config::DaemonConfig;

pub fn start_retention_worker(
    db: Arc<Mutex<Connection>>,
    config: DaemonConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let retention_days = config.delta_retention_days;
        let threshold = config.consolidation.sibling_threshold;
        let interval_mins = config.consolidation.interval_minutes.max(1);

        time::sleep(Duration::from_secs(60)).await;

        let mut interval = time::interval(Duration::from_secs(interval_mins * 60));
        interval.tick().await;

        loop {
            interval.tick().await;

            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            let prune_before = now_ms.saturating_sub(retention_days * 24 * 60 * 60 * 1000);

            let mut conn = db.lock().await;
            match purge_events_before(&conn, prune_before) {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(
                            "purged {count} delta events older than {retention_days} days"
                        );
                    }
                }
                Err(e) => tracing::error!("retention prune failed: {e}"),
            }

            if threshold > 0 {
                match consolidate_events(&mut conn, threshold) {
                    Ok(count) => {
                        if count > 0 {
                            tracing::info!(
                                "consolidated {count} events (threshold={threshold} siblings)"
                            );
                        }
                    }
                    Err(e) => tracing::error!("event consolidation failed: {e}"),
                }
            }

            drop(conn);
        }
    })
}
