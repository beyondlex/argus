use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use argus_core::{scan_path, NodeIndex, ROOT_NODE};

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Scan { path } => cmd_scan(path),
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
