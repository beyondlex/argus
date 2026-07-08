use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use anyhow::{Context, Result};

use argus_core::{
    compare_trees, extract_feature, filter_by_threshold, generate_prompt, has_significant_changes,
    parse_human_size, scan_path, DiffNode, FileNode, Snapshot,
};

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Scan { path, output } => cmd_scan(path, output),
        Commands::Diff {
            old,
            new,
            threshold,
            format,
        } => cmd_diff(old, new, threshold, format),
        Commands::Explain {
            old,
            new,
            target_path,
        } => cmd_explain(old, new, target_path),
    };

    match result {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(3);
        }
    }
}

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "argus")]
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
    Scan {
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Diff {
        #[arg(long)]
        old: PathBuf,
        #[arg(long)]
        new: PathBuf,
        #[arg(long = "threshold", default_value = "0")]
        threshold: String,
        #[arg(long = "format", default_value = "text")]
        format: OutputFormat,
    },
    Explain {
        #[arg(long)]
        old: PathBuf,
        #[arg(long)]
        new: PathBuf,
        #[arg(long = "target-path")]
        target_path: PathBuf,
    },
}

fn cmd_scan(path: &Path, output: &Path) -> Result<i32> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    ctrlc::set_handler(move || {
        cancel_clone.store(true, Ordering::Relaxed);
        eprintln!("\ncancelling scan...");
    })
    .context("failed to set Ctrl+C handler")?;

    let (tx, rx) = mpsc::channel();

    let snapshot =
        scan_path(path, &cancel, Some(tx)).map_err(|e| anyhow::anyhow!("scan failed: {}", e))?;

    while rx.try_recv().is_ok() {}

    let json = serde_json::to_string_pretty(&snapshot).context("failed to serialize snapshot")?;

    std::fs::write(output, &json)
        .with_context(|| format!("failed to write snapshot file: {}", output.display()))?;

    println!("snapshot saved to: {}", output.display());
    println!("scan path: {}", path.display());
    println!("total files: {}", count_files(&snapshot.root_node));
    println!("total size: {}", format_size(snapshot.total_size));

    Ok(0)
}

fn cmd_diff(
    old_path: &Path,
    new_path: &Path,
    threshold_str: &str,
    format: &OutputFormat,
) -> Result<i32> {
    let threshold = if threshold_str == "0" {
        0u64
    } else {
        parse_human_size(threshold_str)
            .map_err(|e| anyhow::anyhow!("failed to parse threshold '{}': {}", threshold_str, e))?
    };

    let old_json = std::fs::read_to_string(old_path)
        .with_context(|| format!("failed to read old snapshot: {}", old_path.display()))?;
    let new_json = std::fs::read_to_string(new_path)
        .with_context(|| format!("failed to read new snapshot: {}", new_path.display()))?;

    let old_snap: Snapshot =
        serde_json::from_str(&old_json).context("failed to parse old snapshot")?;
    let new_snap: Snapshot =
        serde_json::from_str(&new_json).context("failed to parse new snapshot")?;

    let diff =
        compare_trees(&old_snap, &new_snap).map_err(|e| anyhow::anyhow!("diff failed: {}", e))?;

    let filtered = if threshold > 0 {
        filter_by_threshold(&diff, threshold).unwrap_or(DiffNode {
            name: diff.name.clone(),
            is_dir: diff.is_dir,
            current_size: diff.current_size,
            size_delta: diff.size_delta,
            children: std::collections::HashMap::new(),
        })
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

fn cmd_explain(old_path: &Path, new_path: &Path, target_path: &Path) -> Result<i32> {
    let old_json = std::fs::read_to_string(old_path)
        .with_context(|| format!("failed to read old snapshot: {}", old_path.display()))?;
    let new_json = std::fs::read_to_string(new_path)
        .with_context(|| format!("failed to read new snapshot: {}", new_path.display()))?;

    let old_snap: Snapshot =
        serde_json::from_str(&old_json).context("failed to parse old snapshot")?;
    let new_snap: Snapshot =
        serde_json::from_str(&new_json).context("failed to parse new snapshot")?;

    let diff =
        compare_trees(&old_snap, &new_snap).map_err(|e| anyhow::anyhow!("diff failed: {}", e))?;

    let scan_root = &new_snap.root_path;
    let relative = if target_path == scan_root {
        scan_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    } else if let Ok(rest) = target_path.strip_prefix(scan_root) {
        rest.to_string_lossy().trim_start_matches('/').to_string()
    } else {
        target_path.to_string_lossy().to_string()
    };

    let context = extract_feature(&diff, &relative)
        .ok_or_else(|| anyhow::anyhow!("path not found in diff tree: {}", target_path.display()))?;

    let prompt = generate_prompt(&context);
    println!("{}", prompt);

    Ok(0)
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
