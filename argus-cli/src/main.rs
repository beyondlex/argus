use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[cfg(feature = "cleanup")]
use argus_core::{
    default_clean_targets, dry_clean, exec_clean, find_artifacts, find_installed_apps,
    find_leftovers, find_orphaned_data, remove_artifacts, uninstall_app, CleanItem, CleanReport,
    CleanTarget, TargetCategory,
};

#[cfg(feature = "shell-cmds")]
use argus_core::{
    default_shell_cmd_targets, try_exec_shell_cmd,
};
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
        #[cfg(feature = "cleanup")]
        Commands::Clean { dry_run, yes } => cmd_clean(*dry_run, *yes),
        #[cfg(feature = "cleanup")]
        Commands::Uninstall { dry_run } => cmd_uninstall(*dry_run),
        #[cfg(feature = "cleanup")]
        Commands::Purge { paths, dry_run } => cmd_purge(paths.as_deref(), *dry_run),
    };

    match result {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
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
    /// Scan and clean caches, logs, temp files, and trash.
    #[cfg(feature = "cleanup")]
    Clean {
        #[arg(long, help = "Preview only, don't delete anything")]
        dry_run: bool,
        #[arg(long, short = 'y', help = "Skip confirmation prompt")]
        yes: bool,
    },
    /// List installed apps and uninstall with leftover cleanup.
    #[cfg(feature = "cleanup")]
    Uninstall {
        #[arg(long, help = "Preview only, don't delete anything")]
        dry_run: bool,
    },
    /// Find and remove project build artifacts (node_modules, target, etc.).
    #[cfg(feature = "cleanup")]
    Purge {
        #[arg(long, help = "Directories to scan for artifacts")]
        paths: Option<Vec<PathBuf>>,
        #[arg(long, help = "Preview only, don't delete anything")]
        dry_run: bool,
    },
}

fn cmd_scan(path: &PathBuf) -> Result<i32> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    ctrlc::set_handler(move || {
        cancel_clone.store(true, Ordering::Relaxed);
        eprintln!("\n{}", "cancelling scan...".yellow());
    })
    .context("failed to set Ctrl+C handler")?;

    let snapshot =
        scan_path(path, &cancel, None).map_err(|e| anyhow::anyhow!("scan failed: {}", e))?;

    println!(
        "{} {}",
        "scan path:".bold(),
        path.display().to_string().cyan()
    );
    println!(
        "{} {}",
        "total files:".bold(),
        count_files(&snapshot, ROOT_NODE).to_string().green()
    );
    println!(
        "{} {}",
        "total size:".bold(),
        format_size(snapshot.total_size).green()
    );

    Ok(0)
}

fn cmd_help() -> Result<i32> {
    println!("{}", "Argus — Disk Usage Scanner".bold().cyan());
    println!();
    println!("{}", "Commands:".bold().underline());
    println!(
        "  {:34}  {}",
        "scan --path <PATH>".green(),
        "Scan a path and print summary"
    );
    println!(
        "  {:34}  {}",
        "delta-summary --path <PATH>".green(),
        "Print delta summary for a path"
    );
    println!("  {:34}  {}", "help".green(), "Print this help text");
    println!(
        "  {:34}  {}",
        "consolidate".green(),
        "Request daemon to consolidate delta events"
    );
    println!("  {:34}  {}", "status".green(), "Query daemon status");
    println!(
        "  {:34}  {}",
        "clear".green(),
        "Clear all delta events in daemon database"
    );
    #[cfg(feature = "cleanup")]
    {
        println!(
            "  {:34}  {}",
            "clean [--dry-run] [-y]".green(),
            "Scan and clean caches, logs, temp files"
        );
        println!(
            "  {:34}  {}",
            "uninstall [--dry-run]".green(),
            "List and uninstall apps with leftovers"
        );
        println!(
            "  {:34}  {}",
            "purge [--paths <DIR>] [--dry-run]".green(),
            "Find and remove build artifacts"
        );
    }
    println!();
    println!(
        "{}",
        "TUI commands (type : inside the TUI):".bold().underline()
    );
    println!("  {:34}  {}", ":Scan".cyan(), "Scan current directory");
    println!(
        "  {:34}  {}",
        ":Delta <N>[k|m|g]".cyan(),
        "Set delta threshold"
    );
    println!(
        "  {:34}  {}",
        ":Time <N>[m|h|d|w]".cyan(),
        "Set time range (relative)"
    );
    println!(
        "  {:34}  {}",
        ":Time <from> to <to>".cyan(),
        "Set time range (absolute or mixed)"
    );
    println!(
        "  {:34}  {}",
        ":Consolidate".cyan(),
        "Request event consolidation"
    );
    println!("  {:34}  {}", ":Help".cyan(), "Show help overlay");
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
                println!(
                    "{} {} events",
                    "consolidated".green().bold(),
                    consolidated_count.to_string().cyan()
                );
                Ok(0i32)
            }
            DaemonResponse::Error { message } => {
                eprintln!("{} {message}", "daemon error:".red().bold());
                Ok(1)
            }
            _ => {
                eprintln!("{}", "unexpected response".red());
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
                println!("{} v{version}", "argusd".bold().cyan());
                println!("  {}  {}", "uptime:".bold(), format_duration(uptime_secs));
                let start_secs = start_time_secs as i64;
                let naive = chrono::DateTime::from_timestamp(start_secs, 0)
                    .map(|dt| {
                        dt.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_else(|| "unknown".into());
                println!("  {}  {}", "started:".bold(), naive);
                println!(
                    "  {}  {}",
                    "log level:".bold(),
                    log_level.as_deref().unwrap_or("(none)")
                );
                println!("  {}  {}s", "debounce:".bold(), debounce_seconds);
                println!("  {}  {}d", "retention:".bold(), delta_retention_days);
                println!(
                    "  {}  {} events",
                    "db events:".bold(),
                    db_event_count.to_string().cyan()
                );
                println!(
                    "  {}  {}",
                    "db size:".bold(),
                    format_size(db_size_bytes).cyan()
                );
                println!("  {}", "watch dirs:".bold());
                for dir in &watch_dirs {
                    let mut line = format!("    {}", dir.path.display().to_string().blue());
                    if let Some(ref include) = dir.include {
                        line.push_str(&format!(" (include: {include})"));
                    }
                    if let Some(ref exclude) = dir.exclude {
                        line.push_str(&format!(" (exclude: {exclude})"));
                    }
                    println!("{line}");
                }
                Ok(0i32)
            }
            DaemonResponse::Error { message } => {
                eprintln!("{} {message}", "daemon error:".red().bold());
                Ok(1)
            }
            _ => {
                eprintln!("{}", "unexpected response".red());
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
                println!(
                    "{} {} events",
                    "cleared".green().bold(),
                    deleted_count.to_string().cyan()
                );
                Ok(0i32)
            }
            DaemonResponse::Error { message } => {
                eprintln!("{} {message}", "daemon error:".red().bold());
                Ok(1)
            }
            _ => {
                eprintln!("{}", "unexpected response".red());
                Ok(1)
            }
        }
    })
}

// ── Highlight utils ─────────────────────────────────────────────────────────

#[cfg(feature = "cleanup")]
fn risk_color(risk: &str) -> colored::ColoredString {
    match risk {
        "safe" => "safe".green(),
        "low" => "low".cyan(),
        "medium" => "medium".yellow(),
        "high" => "high".red(),
        _ => risk.into(),
    }
}

// ── Clean ────────────────────────────────────────────────────────────────────

#[cfg(feature = "cleanup")]
fn cmd_clean(dry_run: bool, yes: bool) -> Result<i32> {
    let targets = default_clean_targets();
    if targets.is_empty() {
        println!(
            "{}",
            "no cleanup targets available for this platform".yellow()
        );
        return Ok(0);
    }

    let plan = dry_clean(&targets).map_err(|e| anyhow::anyhow!("plan clean: {e}"))?;
    if plan.is_empty() {
        println!("{}", "nothing to clean — all targets are empty".green());
        return Ok(0);
    }

    let target_map: std::collections::HashMap<&str, &CleanTarget> = targets
        .iter()
        .map(|t| (t.id.as_str(), t))
        .collect();

    let mut grouped: Vec<(TargetCategory, Vec<&CleanItem>)> = Vec::new();
    for item in &plan.items {
        let cat = target_map
            .get(item.target_id.as_str())
            .map(|t| t.category)
            .unwrap_or(TargetCategory::TempFiles);
        if let Some(pos) = grouped.iter().position(|(g, _)| *g == cat) {
            grouped[pos].1.push(item);
        } else {
            grouped.push((cat, vec![item]));
        }
    }
    grouped.sort_by_key(|(g, _)| *g);

    println!("{}", "Clean Your Mac".bold().cyan());
    println!();
    if dry_run {
        println!("{}", "☻ First time? Run mo clean --dry-run first to preview changes".yellow());
    }
    println!(
        "{} {}",
        "Free space:".bold(),
        format_size(free_space_macos()).cyan()
    );
    println!();

    for (cat, items) in &grouped {
        let label = match cat {
            TargetCategory::AppCache => "App Cache",
            TargetCategory::BrowserCache => "Browser Cache",
            TargetCategory::DevTools => "Developer Tools",
            TargetCategory::DevApps => "Development Applications",
            TargetCategory::SystemLogs => "System Logs",
            TargetCategory::SystemCache => "macOS System Caches",
            TargetCategory::TempFiles => "Temp Files",
            TargetCategory::Trash => "Trash",
            TargetCategory::UserData => "User Essentials",
            TargetCategory::CloudStorage => "Cloud Storage",
            TargetCategory::Office => "Office Applications",
            TargetCategory::VMTools => "Virtual Machine Tools",
            TargetCategory::AppSupport => "Application Support",
            TargetCategory::UninstalledData => "Uninstalled App Data",
            TargetCategory::IosBackup => "iOS Device Backups",
            TargetCategory::TimeMachine => "Time Machine",
        };
        println!("➤ {}", label.bold());
        for item in items {
            let risk_l = item.risk.label();
            let colored_risk = risk_color(risk_l);
            let size_s = format_size(item.size).green().bold();
            let label = target_map
                .get(item.target_id.as_str())
                .map(|t| t.label.as_str())
                .unwrap_or(&item.target_id);
            println!(
                "  ✓ {} {} ({})",
                label.white(),
                if item.size > 0 {
                    format!("({})", size_s)
                } else {
                    String::new()
                },
                colored_risk
            );
        }
        println!();
    }

    // ── Uninstalled app data ──────────────────────────────────────────────────
    println!("➤ {}", "Uninstalled App Data".bold());
    match find_orphaned_data() {
        Ok(orphaned) => {
            let apps = find_installed_apps(None).unwrap_or_default();
            println!("  ✓ Found {} active/installed apps", apps.len().to_string().cyan());
            if orphaned.item_count > 0 {
                println!(
                    "  ✓ {} {} items ({})",
                    orphaned.item_count.to_string().white(),
                    "orphaned paths".white(),
                    format_size(orphaned.total_bytes).green().bold(),
                );
            } else {
                println!("  ✓ {}", "Nothing to clean".green());
            }
        }
        Err(e) => {
            println!("  ! {}", format!("scan failed: {e}").red());
        }
    }
    println!();

    // ── Shell commands (brew, docker) ────────────────────────────────────────
    #[cfg(feature = "shell-cmds")]
    {
        let shell_cmds = default_shell_cmd_targets();
        println!("➤ {}", "Shell Commands".bold());
        for cmd in &shell_cmds {
            if dry_run {
                println!("  ☻ {} (dry-run, skipped)", cmd.label.white());
            } else {
                let result = try_exec_shell_cmd(cmd);
                if result.success {
                    let output = if result.output.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", result.output.dimmed())
                    };
                    println!("  ✓ {}{}", cmd.label.white(), output);
                } else {
                    let err = result.error.unwrap_or_else(|| "unknown error".into());
                    println!("  ☻ {} ({})", cmd.label.white(), err.yellow());
                }
            }
        }
        println!();
    }

    if dry_run {
        println!("{}", "[dry-run] no files were deleted".yellow().bold());
        return Ok(0);
    }

    if !yes {
        let ans = inquire::Confirm::new("Proceed with cleanup?")
            .with_default(false)
            .prompt()?;
        if !ans {
            println!("{}", "cancelled".yellow());
            return Ok(0);
        }
    }

    let report = exec_clean(&plan.items, false).map_err(|e| anyhow::anyhow!("exec clean: {e}"))?;
    print_clean_report(&report);
    Ok(0)
}

#[cfg(target_os = "macos")]
fn free_space_macos() -> u64 {
    let path = std::path::Path::new("/");
    if !path.exists() {
        return 0;
    }
    match std::process::Command::new("df")
        .arg("-k")
        .arg("/")
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    if let Ok(blocks) = parts[3].parse::<u64>() {
                        return blocks * 1024;
                    }
                }
            }
            0
        }
        Err(_) => 0,
    }
}

// ── Uninstall ────────────────────────────────────────────────────────────────

#[cfg(feature = "cleanup")]
fn cmd_uninstall(dry_run: bool) -> Result<i32> {
    let apps = find_installed_apps(None).map_err(|e| anyhow::anyhow!("find apps: {e}"))?;
    if apps.is_empty() {
        println!("{}", "no apps found".yellow());
        return Ok(0);
    }

    let selections: Vec<String> = apps
        .iter()
        .map(|a| {
            let size = format_size(a.size);
            format!("{:<30} {:>9}  {}", a.name, size, a.id)
        })
        .collect();

    let sel = inquire::Select::new(
        "Select app to uninstall (↑↓/j/k to move, type to filter, Esc to cancel):",
        selections,
    )
    .with_page_size(15)
    .with_vim_mode(true)
    .with_help_message("↑↓ navigate • type to filter • Enter confirm • Esc cancel")
    .prompt();

    let idx = match sel {
        Ok(chosen) => apps.iter().position(|a| {
            let size = format_size(a.size);
            format!("{:<30} {:>9}  {}", a.name, size, a.id) == chosen
        }),
        Err(_) => None,
    };

    let app = match idx {
        Some(i) => &apps[i],
        None => {
            println!("{}", "cancelled".yellow());
            return Ok(0);
        }
    };

    let leftovers = find_leftovers(app).map_err(|e| anyhow::anyhow!("find leftovers: {e}"))?;

    println!("\n{} {}", "Selected:".bold(), app.name.cyan().bold());
    println!("  {}  {}", "bundle:".bold(), app.id);
    println!("  {}  {}", "size:".bold(), format_size(app.size).green());

    if !leftovers.leftover_paths.is_empty() {
        println!(
            "  {}  {} across {} paths",
            "leftovers:".bold(),
            format_size(leftovers.total_leftover_bytes).yellow(),
            leftovers.leftover_paths.len().to_string().cyan()
        );
        for p in &leftovers.leftover_paths {
            println!("    └─ {}", p.display().to_string().dimmed());
        }
    } else {
        println!("  {}  none found", "leftovers:".bold());
    }

    if dry_run {
        println!("\n{}", "[dry-run] no files were deleted".yellow().bold());
        return Ok(0);
    }

    let remove_leftovers = if leftovers.total_leftover_bytes > 0 {
        inquire::Confirm::new("Remove leftovers too?")
            .with_default(true)
            .prompt()?
    } else {
        true
    };

    let proceed = inquire::Confirm::new(&format!("Uninstall {}?", app.name))
        .with_default(false)
        .prompt()?;
    if !proceed {
        println!("{}", "cancelled".yellow());
        return Ok(0);
    }

    let report =
        uninstall_app(app, remove_leftovers).map_err(|e| anyhow::anyhow!("uninstall: {e}"))?;
    print_clean_report(&report);
    Ok(0)
}

// ── Purge ────────────────────────────────────────────────────────────────────

#[cfg(feature = "cleanup")]
fn cmd_purge(paths: Option<&[PathBuf]>, dry_run: bool) -> Result<i32> {
    let roots = paths.unwrap_or_default();
    let artifacts = find_artifacts(roots).map_err(|e| anyhow::anyhow!("find artifacts: {e}"))?;
    if artifacts.is_empty() {
        println!("{}", "no build artifacts found".green());
        return Ok(0);
    }

    let total_size: u64 = artifacts.iter().map(|a| a.size).sum();
    println!("{}", "Build Artifacts".bold().cyan().underline());
    println!(
        "{} {} ({} items)\n",
        "total:".bold(),
        format_size(total_size).yellow().bold(),
        artifacts.len().to_string().cyan()
    );

    for art in &artifacts {
        let kind = art.kind.label();
        let age = if art.age_days == 0 {
            "today".to_string()
        } else {
            format!("{}d old", art.age_days)
        };
        println!(
            "  {:>20}  {}  {}  {}",
            kind.cyan().bold(),
            format_size(art.size).green(),
            art.path.display().to_string().white(),
            age.dimmed()
        );
    }

    if dry_run {
        println!("\n{}", "[dry-run] no files were deleted".yellow().bold());
        return Ok(0);
    }

    let proceed = inquire::Confirm::new(&format!("Remove all {} artifacts?", artifacts.len()))
        .with_default(false)
        .prompt()?;
    if !proceed {
        println!("{}", "cancelled".yellow());
        return Ok(0);
    }

    let report =
        remove_artifacts(&artifacts).map_err(|e| anyhow::anyhow!("remove artifacts: {e}"))?;
    print_clean_report(&report);
    Ok(0)
}

// ── Report ───────────────────────────────────────────────────────────────────

#[cfg(feature = "cleanup")]
fn print_clean_report(report: &CleanReport) {
    println!("\n{}", "Result".bold().green().underline());
    let status = if report.total_failed == 0 {
        "✓ success".green().bold()
    } else {
        format!("⚠ {} failures", report.total_failed).red().bold()
    };
    println!(
        "  {}  {}",
        status,
        format_size(report.freed_bytes).yellow().bold()
    );
    println!(
        "  {} {}/{} attempted",
        "items:".bold(),
        report.total_succeeded.to_string().green(),
        report.total_attempted.to_string().cyan()
    );

    for (path, err) in &report.errors {
        eprintln!(
            "  {}  {}  — {}",
            "✗".red(),
            path.display().to_string().dimmed(),
            err.red()
        );
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn count_files(snap: &argus_core::Snapshot, idx: NodeIndex) -> u64 {
    let mut count = 0u64;
    if !snap.node(idx).is_dir() {
        count += 1;
    }
    for &child_idx in snap.children(idx) {
        count += count_files(snap, child_idx);
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
    println!(
        "{}  {}",
        "delta summary path:".bold(),
        path.display().to_string().cyan()
    );
    println!("{}  {} .. {}", "window:".bold(), from_ms, to_ms);
    println!(
        "{}  {}",
        "events:".bold(),
        summary.event_count.to_string().green()
    );
    println!(
        "  create/modify/delete/agg: {}/{}/{}/{}",
        summary.create_count.to_string().green(),
        summary.modify_count.to_string().cyan(),
        summary.delete_count.to_string().red(),
        summary.agg_count
    );
    println!(
        "  +/-/0: {}/{}/{}",
        summary.positive_events.to_string().green(),
        summary.negative_events.to_string().red(),
        summary.zero_events
    );
    println!(
        "{}  {}",
        "total delta:".bold(),
        format_signed_size(summary.total_delta)
    );
    println!(
        "  {}  {}",
        "positive delta:".bold(),
        format_signed_size(summary.positive_delta).green()
    );
    println!(
        "  {}  {}",
        "negative delta:".bold(),
        format_signed_size(summary.negative_delta).red()
    );
}
