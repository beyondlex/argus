use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use argus_core::{
    default_db_path, open_db, query_delta_summary, scan_path, DaemonRequest, DaemonResponse,
    DeltaSummary, NodeIndex, ROOT_NODE,
};

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Scan { path } => cmd_scan(path),
        Commands::DeltaSummary {
            path,
            from_ms,
            to_ms,
        } => cmd_delta_summary(path, *from_ms, *to_ms),
        Commands::Help => cmd_help(),
        Commands::Consolidate => cmd_consolidate(),
        Commands::Status => cmd_status(),
        Commands::Clear => cmd_clear(),
    };

    match result {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(3);
        }
    }
}

#[derive(Parser)]
#[command(disable_help_subcommand = true)]
#[command(name = "argus", version, about = "Disk usage scanner")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan a path and print disk usage summary.
    Scan {
        #[arg(long, help = "Path to scan")]
        path: PathBuf,
    },
    /// Print a delta summary for a path without listing items.
    DeltaSummary {
        #[arg(long, help = "Path to summarize")]
        path: PathBuf,
        #[arg(long, help = "Inclusive lower bound timestamp in milliseconds")]
        from_ms: Option<u64>,
        #[arg(long, help = "Inclusive upper bound timestamp in milliseconds")]
        to_ms: Option<u64>,
    },
    /// Print usage information.
    Help,
    /// Request delta event consolidation on the daemon.
    Consolidate,
    /// Query daemon status.
    Status,
    /// Clear all delta events in the daemon database.
    Clear,
}

fn cmd_scan(path: &PathBuf) -> Result<i32> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    ctrlc::set_handler(move || {
        cancel_clone.store(true, Ordering::Relaxed);
        eprintln!("\ncancelling scan...");
    })
    .context("failed to set Ctrl+C handler")?;

    let snapshot =
        scan_path(path, &cancel, None, &[]).map_err(|e| anyhow::anyhow!("scan failed: {}", e))?;

    println!("scan path: {}", path.display());
    println!("total files: {}", count_files(&snapshot, ROOT_NODE));
    println!("total size: {}", format_size(snapshot.total_size));

    Ok(0)
}

fn cmd_help() -> Result<i32> {
    println!("Argus - Disk usage scanner");
    println!();
    println!("Commands:");
    println!("  scan --path <PATH>    Scan a path and print summary");
    println!("  delta-summary --path <PATH>  Print delta summary for a path");
    println!("  help                  Print this help text");
    println!("  consolidate           Request daemon to consolidate delta events");
    println!("  status                Query daemon status");
    println!("  clear                 Clear all delta events in daemon database");
    println!();
    println!("TUI commands (type : inside the TUI):");
    println!("  :Scan                 Scan current directory");
    println!("  :FilterClear          Clear delta filter");
    println!("  :FilterFocus          Focus filter pane");
    println!("  :Delta <N>[k|m|g]    Set delta threshold");
    println!("  :Time <N>[h|d|w]     Set time range (relative)");
    println!("  :Time <from> to <to> Set time range (absolute or mixed)");
    println!("  :Consolidate          Request event consolidation");
    println!("  :Help                 Show help overlay");
    Ok(0)
}

fn cmd_delta_summary(path: &PathBuf, from_ms: Option<u64>, to_ms: Option<u64>) -> Result<i32> {
    let path = std::fs::canonicalize(path)
        .with_context(|| format!("failed to resolve path: {}", path.display()))?;
    let db_path = default_db_path();
    let conn =
        open_db(&db_path).with_context(|| format!("failed to open {}", db_path.display()))?;
    let from_ms = from_ms.unwrap_or(0);
    let to_ms = to_ms.unwrap_or(i64::MAX as u64);
    let summary = query_delta_summary(&conn, &path, from_ms, to_ms)
        .with_context(|| format!("failed to query summary for {}", path.display()))?;

    print_delta_summary(&path, from_ms, to_ms, &summary);
    Ok(0)
}

fn cmd_consolidate() -> Result<i32> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let uds_path = argus_core::DEFAULT_UDS_PATH;
        let mut stream = UnixStream::connect(uds_path)
            .await
            .map_err(|e| anyhow::anyhow!("connect to daemon failed: {e}"))?;

        let req = DaemonRequest::RequestConsolidation;
        let payload = bincode::serialize(&req).map_err(|e| anyhow::anyhow!("serialize: {e}"))?;
        stream
            .write_all(&(payload.len() as u32).to_be_bytes())
            .await?;
        stream.write_all(&payload).await?;

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        stream.read_exact(&mut resp_buf).await?;

        let resp: DaemonResponse =
            bincode::deserialize(&resp_buf).map_err(|e| anyhow::anyhow!("deserialize: {e}"))?;
        match resp {
            DaemonResponse::ConsolidationDone { consolidated_count } => {
                println!("consolidated {consolidated_count} events");
                Ok(0i32)
            }
            DaemonResponse::Error { message } => {
                eprintln!("daemon error: {message}");
                Ok(1)
            }
            _ => {
                eprintln!("unexpected response");
                Ok(1)
            }
        }
    })
}

fn cmd_status() -> Result<i32> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let uds_path = argus_core::DEFAULT_UDS_PATH;
        let mut stream = UnixStream::connect(uds_path)
            .await
            .map_err(|e| anyhow::anyhow!("connect to daemon failed: {e}"))?;

        let req = DaemonRequest::GetStatus;
        let payload = bincode::serialize(&req).map_err(|e| anyhow::anyhow!("serialize: {e}"))?;
        stream
            .write_all(&(payload.len() as u32).to_be_bytes())
            .await?;
        stream.write_all(&payload).await?;

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        stream.read_exact(&mut resp_buf).await?;

        let resp: DaemonResponse =
            bincode::deserialize(&resp_buf).map_err(|e| anyhow::anyhow!("deserialize: {e}"))?;
        match resp {
            DaemonResponse::Status {
                version,
                watch_dirs,
                uptime_secs,
                start_time_secs,
                log_level,
                debounce_seconds,
                delta_retention_days,
                db_event_count,
                db_size_bytes,
            } => {
                println!("version: {version}");
                println!("uptime: {}", format_duration(uptime_secs));
                let start_secs = start_time_secs as i64;
                let naive = chrono::DateTime::from_timestamp(start_secs, 0)
                    .map(|dt| {
                        dt.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_else(|| "unknown".into());
                println!("started: {naive}");
                println!("log_level: {}", log_level.as_deref().unwrap_or("(none)"));
                println!("debounce_seconds: {debounce_seconds}");
                println!("delta_retention_days: {delta_retention_days}");
                println!("db_event_count: {db_event_count}");
                println!("db_size: {}", format_size(db_size_bytes));
                println!("watch_dirs:");
                for dir in &watch_dirs {
                    println!("  {}", dir.display());
                }
                Ok(0i32)
            }
            DaemonResponse::Error { message } => {
                eprintln!("daemon error: {message}");
                Ok(1)
            }
            _ => {
                eprintln!("unexpected response");
                Ok(1)
            }
        }
    })
}

fn cmd_clear() -> Result<i32> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let uds_path = argus_core::DEFAULT_UDS_PATH;
        let mut stream = UnixStream::connect(uds_path)
            .await
            .map_err(|e| anyhow::anyhow!("connect to daemon failed: {e}"))?;

        let req = DaemonRequest::ClearDb;
        let payload = bincode::serialize(&req).map_err(|e| anyhow::anyhow!("serialize: {e}"))?;
        stream
            .write_all(&(payload.len() as u32).to_be_bytes())
            .await?;
        stream.write_all(&payload).await?;

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        stream.read_exact(&mut resp_buf).await?;

        let resp: DaemonResponse =
            bincode::deserialize(&resp_buf).map_err(|e| anyhow::anyhow!("deserialize: {e}"))?;
        match resp {
            DaemonResponse::DbCleared { deleted_count } => {
                println!("cleared {deleted_count} events");
                Ok(0i32)
            }
            DaemonResponse::Error { message } => {
                eprintln!("daemon error: {message}");
                Ok(1)
            }
            _ => {
                eprintln!("unexpected response");
                Ok(1)
            }
        }
    })
}

fn count_files(snap: &argus_core::Snapshot, idx: NodeIndex) -> u64 {
    let node = snap.node(idx);
    let mut count = 0u64;
    if !node.is_dir {
        count += 1;
    }
    for (_, child_idx) in node.children.iter() {
        count += count_files(snap, *child_idx);
    }
    count
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}

fn format_duration(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let secs = secs % 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if secs > 0 || parts.is_empty() {
        parts.push(format!("{secs}s"));
    }
    parts.join(" ")
}

fn format_signed_size(bytes: i64) -> String {
    let sign = if bytes < 0 { "-" } else { "+" };
    let abs = bytes.unsigned_abs();
    format!("{sign}{}", format_size(abs))
}

fn print_delta_summary(path: &PathBuf, from_ms: u64, to_ms: u64, summary: &DeltaSummary) {
    println!("delta summary path: {}", path.display());
    println!("window: {}..{}", from_ms, to_ms);
    println!("events: {}", summary.event_count);
    println!(
        "create/modify/delete/agg: {}/{}/{}/{}",
        summary.create_count, summary.modify_count, summary.delete_count, summary.agg_count
    );
    println!(
        "positive/negative/zero events: {}/{}/{}",
        summary.positive_events, summary.negative_events, summary.zero_events
    );
    println!("total delta: {}", format_signed_size(summary.total_delta));
    println!(
        "positive delta: {}",
        format_signed_size(summary.positive_delta)
    );
    println!(
        "negative delta: {}",
        format_signed_size(summary.negative_delta)
    );
}
