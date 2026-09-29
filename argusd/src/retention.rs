use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;
use tokio::sync::Mutex;
use tokio::time;

use argus_core::{consolidate_events, purge_events_before};

use crate::config::DaemonConfig;

/// Milliseconds in `days`, saturating instead of wrapping: an absurd
/// `delta_retention_days` config value multiplied out in release mode used
/// to wrap to a small number, making `prune_before` land in the near past
/// or future — one retention tick could purge the whole delta log.
fn retention_ms(days: u64) -> u64 {
    days.saturating_mul(24 * 60 * 60 * 1000)
}

/// Consolidation tick period in seconds; zero (or overflow) clamps to the
/// smallest sane interval instead of panicking in debug builds.
fn interval_secs(minutes: u64) -> u64 {
    minutes.max(1).saturating_mul(60)
}

/// Retention window in days. `0` would make `prune_before` land at "now",
/// wiping the whole delta log on the first tick — a typo like
/// `delta_retention_days = 0` must not behave as "delete everything"
/// (that is what `argus clear` is for), so it clamps to one day.
fn retention_days(days: u64) -> u64 {
    days.max(1)
}

pub fn start_retention_worker(
    db: Arc<Mutex<Connection>>,
    config: DaemonConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let retention_days = retention_days(config.delta_retention_days);
        let threshold = config.consolidation.sibling_threshold;
        let interval = Duration::from_secs(interval_secs(config.consolidation.interval_minutes));

        time::sleep(Duration::from_secs(60)).await;

        let mut interval = time::interval(interval);
        interval.tick().await;

        loop {
            interval.tick().await;

            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            let prune_before = now_ms.saturating_sub(retention_ms(retention_days));

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A huge retention window must saturate (prune nothing), not wrap to a
    /// small `prune_before` that purges the entire delta log on the next tick.
    #[test]
    fn test_retention_ms_saturates() {
        assert_eq!(retention_ms(0), 0);
        assert_eq!(retention_ms(30), 30 * 24 * 60 * 60 * 1000);
        assert_eq!(retention_ms(u64::MAX), u64::MAX);
    }

    #[test]
    fn test_interval_secs_clamps_zero_and_overflow() {
        assert_eq!(interval_secs(0), 60);
        assert_eq!(interval_secs(60), 3_600);
        assert_eq!(interval_secs(u64::MAX), u64::MAX);
    }

    /// `delta_retention_days = 0` must clamp to 1: with the raw value,
    /// `prune_before = now` and the first tick wipes the entire delta log.
    #[test]
    fn test_retention_days_clamps_zero() {
        assert_eq!(retention_days(0), 1);
        assert_eq!(retention_days(30), 30);
    }
}
