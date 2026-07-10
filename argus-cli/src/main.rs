use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};

use argus_core::{
    build_diff_tree, default_db_path, filter_by_threshold, has_significant_changes, open_db,
    parse_human_size, query_delta, query_root_summaries, query_scan_timestamps, scan_path,
    write_scan, DiffNode, FileNode, RootScanSummary,
};

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Scan { path } => cmd_scan(path),
        Commands::Diff {
            path,
            from,
            to,
            threshold,
            format,
        } => cmd_diff(path, from, to, threshold, format),
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

#[derive(ValueEnum, Clone, Default)]
enum OutputFormat {
    #[default]
    Text,
    Json,
    Markdown,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan a path and write it into the SQLite scan history.
    Scan {
        #[arg(long, help = "Path to scan")]
        path: PathBuf,
    },
    /// Compare two timestamps for one root path and print a diff report.
    Diff {
        #[arg(long, help = "Root path to compare")]
        path: PathBuf,
        #[arg(long, help = "Start timestamp in RFC3339 UTC")]
        from: String,
        #[arg(long, help = "End timestamp in RFC3339 UTC")]
        to: String,
        #[arg(
            long = "threshold",
            default_value = "0",
            help = "Only show changes at or above this size"
        )]
        threshold: String,
        #[arg(
            long = "format",
            default_value = "text",
            help = "Output format: text, json, or markdown"
        )]
        format: OutputFormat,
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

fn cmd_diff(
    path: &Path,
    from: &str,
    to: &str,
    threshold_str: &str,
    format: &OutputFormat,
) -> Result<i32> {
    let threshold = if threshold_str == "0" {
        0u64
    } else {
        parse_human_size(threshold_str)
            .map_err(|e| anyhow::anyhow!("failed to parse threshold '{}': {}", threshold_str, e))?
    };

    let from_time = parse_rfc3339_utc(from)?;
    let to_time = parse_rfc3339_utc(to)?;
    let db_path = default_db_path();
    let conn = open_db(&db_path).context("failed to open SQLite database")?;

    let records =
        query_delta(&conn, path, &from_time, &to_time).context("failed to query SQLite delta")?;
    let root_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let diff = build_diff_tree(&records, &root_name);

    let filtered = if threshold > 0 {
        filter_by_threshold(&diff, threshold).unwrap_or_else(|| empty_diff_root(&root_name))
    } else {
        diff
    };

    match format {
        OutputFormat::Text => print_text_diff(&filtered, threshold),
        OutputFormat::Json => print_json_diff(&filtered)?,
        OutputFormat::Markdown => print_markdown_diff(&filtered),
    }

    let has_changes = has_significant_changes(&filtered, threshold);
    Ok(if has_changes { 1 } else { 0 })
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

fn parse_rfc3339_utc(input: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .map(|dt| dt.with_timezone(&Utc))
        .with_context(|| format!("failed to parse RFC3339 timestamp: {input}"))
}

fn empty_diff_root(name: &str) -> DiffNode {
    DiffNode {
        name: name.to_string(),
        is_dir: true,
        current_size: 0,
        size_delta: 0,
        children: HashMap::new(),
    }
}

fn print_text_diff(node: &DiffNode, _threshold: u64) {
    print_text_node(node, 0);
}

fn print_text_node(node: &DiffNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let delta_str = if node.size_delta >= 0 {
        format!("+{}", format_size(node.size_delta as u64))
    } else {
        format!("-{}", format_size(node.size_delta.unsigned_abs()))
    };
    let size_str = format_size(node.current_size);

    let suffix = if node.is_dir { "/" } else { "" };
    println!(
        "{}{}{}  [size: {}, delta: {}]",
        indent, node.name, suffix, size_str, delta_str
    );

    let mut sorted_children: Vec<&DiffNode> = node.children.values().collect();
    sorted_children.sort_by(|a, b| {
        let a_delta_abs = a.size_delta.abs();
        let b_delta_abs = b.size_delta.abs();
        b_delta_abs.cmp(&a_delta_abs)
    });

    for child in sorted_children {
        print_text_node(child, depth + 1);
    }
}

fn print_json_diff(node: &DiffNode) -> Result<()> {
    let json = serde_json::to_string_pretty(node).context("failed to serialize diff result")?;
    println!("{}", json);
    Ok(())
}

fn print_markdown_diff(node: &DiffNode) {
    println!("# Diff Report\n");
    println!("| Path | Size | Delta |");
    println!("|------|------|-------|");
    print_markdown_node(node, "");
}

fn print_markdown_node(node: &DiffNode, prefix: &str) {
    let delta_str = if node.size_delta >= 0 {
        format!("+{}", format_size(node.size_delta as u64))
    } else {
        format!("-{}", format_size(node.size_delta.unsigned_abs()))
    };
    let size_str = format_size(node.current_size);
    println!(
        "| {}{}/ | {} | {} |",
        prefix, node.name, size_str, delta_str
    );

    let mut sorted_children: Vec<&DiffNode> = node.children.values().collect();
    sorted_children.sort_by(|a, b| {
        let a_delta_abs = a.size_delta.abs();
        let b_delta_abs = b.size_delta.abs();
        b_delta_abs.cmp(&a_delta_abs)
    });

    for child in sorted_children {
        let new_prefix = if prefix.is_empty() {
            node.name.clone()
        } else {
            format!("{}/{}", prefix, node.name)
        };
        print_markdown_node(child, &new_prefix);
    }
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
