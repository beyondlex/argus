use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use argus_core::{scan_path, DaemonRequest, DaemonResponse, NodeIndex, ROOT_NODE};

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Scan { path } => cmd_scan(path),
        Commands::Help => cmd_help(),
        Commands::Consolidate => cmd_consolidate(),
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
    /// Print usage information.
    Help,
    /// Request delta event consolidation on the daemon.
    Consolidate,
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
    println!("  help                  Print this help text");
    println!("  consolidate           Request daemon to consolidate delta events");
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
