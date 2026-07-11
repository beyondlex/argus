use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use argus_core::{
    default_db_path, open_db, query_root_summaries, query_scan_timestamps, scan_path, write_scan,
    FileNode, RootScanSummary,
};

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Scan { path } => cmd_scan(path),
        Commands::ListScans { path } => cmd_list_scans(path),
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
#[command(
    name = "argus",
    version,
    about = "SQLite-first CLI for scanning disk usage and comparing scan history"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan a path and write it into the SQLite scan history.
    Scan {
        #[arg(long, help = "Path to scan")]
        path: PathBuf,
    },
    /// List available scan timestamps or scan roots.
    ListScans {
        #[arg(long, help = "Root path to list scans for")]
        path: Option<PathBuf>,
    },
}

fn cmd_scan(path: &Path) -> Result<i32> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    ctrlc::set_handler(move || {
        cancel_clone.store(true, Ordering::Relaxed);
        eprintln!("\ncancelling scan...");
    })
    .context("failed to set Ctrl+C handler")?;

    let snapshot =
        scan_path(path, &cancel, None, &[]).map_err(|e| anyhow::anyhow!("scan failed: {}", e))?;
    let db_path = default_db_path();
    let mut conn = open_db(&db_path).context("failed to open SQLite database")?;
    let scan_id = write_scan(&mut conn, &snapshot).context("failed to write scan to database")?;

    println!("scan saved to SQLite: {}", db_path.display());
    println!("scan id: {}", scan_id);
    println!("scan path: {}", path.display());
    println!("total files: {}", count_files(&snapshot.root_node));
    println!("total size: {}", format_size(snapshot.total_size));

    Ok(0)
}

fn cmd_list_scans(path: &Option<PathBuf>) -> Result<i32> {
    let db_path = default_db_path();
    let conn = open_db(&db_path).context("failed to open SQLite database")?;

    match path {
        Some(path) => {
            let scans = query_scan_timestamps(&conn, path).context("failed to list scans")?;
            if scans.is_empty() {
                println!("no scans found for {}", path.display());
            } else {
                for (id, timestamp, total_size, total_files) in scans {
                    println!(
                        "{}  {}  files: {}  size: {}",
                        id,
                        timestamp.to_rfc3339(),
                        total_files,
                        format_size(total_size),
                    );
                }
            }
        }
        None => {
            let roots = query_root_summaries(&conn).context("failed to list scan roots")?;
            if roots.is_empty() {
                println!("no scan roots found");
            } else {
                for RootScanSummary {
                    root_path,
                    root_path_hash,
                    scan_count,
                    latest_timestamp,
                } in roots
                {
                    println!(
                        "{}  [{}]  scans: {}  latest: {}",
                        root_path.display(),
                        root_path_hash,
                        scan_count,
                        latest_timestamp.to_rfc3339(),
                    );
                }
            }
        }
    }

    Ok(0)
}

fn count_files(node: &FileNode) -> u64 {
    let mut count = 0u64;
    if !node.is_dir {
        count += 1;
    }
    for child in node.children.values() {
        count += count_files(child);
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
