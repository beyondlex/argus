use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[cfg(feature = "cleanup")]
use argus_core::{
    brew_cache_size, brew_dependents_of, default_clean_targets, dry_clean, exec_clean,
    find_artifacts, find_installed_apps, find_leftovers, find_orphaned_data, is_brew_available,
    list_brew_packages, remove_artifacts, uninstall_app, uninstall_brew_package, BrewFilterType,
    BrewPackageType, CleanItem, CleanReport, CleanTarget, TargetCategory,
};

use argus_core::{
    default_db_path, open_db, query_delta_summary, scan_path, DaemonRequest, DaemonResponse,
    DeltaSummary,
};
#[cfg(feature = "shell-cmds")]
use argus_core::{default_shell_cmd_targets, try_exec_shell_cmd};

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
        Commands::Consolidate => cmd_consolidate(cli.uds_path.as_deref()),
        Commands::Status => cmd_status(cli.uds_path.as_deref()),
        Commands::Clear => cmd_clear(cli.uds_path.as_deref()),
        #[cfg(feature = "cleanup")]
        Commands::Clean { dry_run, yes } => cmd_clean(*dry_run, *yes),
        #[cfg(feature = "cleanup")]
        Commands::Uninstall { dry_run } => cmd_uninstall(*dry_run),
        #[cfg(feature = "cleanup")]
        Commands::Purge { paths, dry_run } => cmd_purge(paths.as_deref(), *dry_run),
        #[cfg(feature = "cleanup")]
        Commands::Brew {
            formula,
            cask,
            dry_run,
            yes,
        } => cmd_brew(*formula, *cask, *dry_run, *yes),
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
    /// Unix domain socket of the argusd daemon. Must match the daemon's
    /// configured uds_path (default: /tmp/argusd.sock).
    #[arg(long, global = true)]
    uds_path: Option<String>,

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
    /// List and uninstall Homebrew packages, sorted by last used time.
    #[cfg(feature = "cleanup")]
    Brew {
        #[arg(long, help = "Show only formula")]
        formula: bool,
        #[arg(long, help = "Show only casks")]
        cask: bool,
        #[arg(long, help = "Preview only, don't uninstall")]
        dry_run: bool,
        #[arg(long, short = 'y', help = "Skip confirmation prompt")]
        yes: bool,
    },
}

fn cmd_scan(path: &Path) -> Result<i32> {
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
        snapshot.total_files.to_string().green()
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
        "  {:34}  Scan a path and print summary",
        "scan --path <PATH>".green()
    );
    println!(
        "  {:34}  Print delta summary for a path",
        "delta-summary --path <PATH>".green()
    );
    println!("  {:34}  Print this help text", "help".green());
    println!(
        "  {:34}  Request daemon to consolidate delta events",
        "consolidate".green()
    );
    println!("  {:34}  Query daemon status", "status".green());
    println!(
        "  {:34}  Clear all delta events in daemon database",
        "clear".green()
    );
    #[cfg(feature = "cleanup")]
    {
        println!(
            "  {:34}  Scan and clean caches, logs, temp files",
            "clean [--dry-run] [-y]".green()
        );
        println!(
            "  {:34}  List and uninstall apps with leftovers",
            "uninstall [--dry-run]".green()
        );
        println!(
            "  {:34}  Find and remove build artifacts",
            "purge [--paths <DIR>] [--dry-run]".green()
        );
        println!(
            "  {:34}  List/uninstall brew packages by last used",
            "brew [--formula] [--cask] [--dry-run] [-y]".green()
        );
    }
    println!();
    println!(
        "{}",
        "TUI commands (type : inside the TUI):".bold().underline()
    );
    println!("  {:34}  Scan current directory", ":Scan".cyan());
    println!("  {:34}  Set delta threshold", ":Delta <N>[k|m|g]".cyan());
    println!(
        "  {:34}  Set time range (relative)",
        ":Time <N>[m|h|d|w]".cyan()
    );
    println!(
        "  {:34}  Set time range (absolute or mixed)",
        ":Time <from> to <to>".cyan()
    );
    println!(
        "  {:34}  Request event consolidation",
        ":Consolidate".cyan()
    );
    println!("  {:34}  Show help overlay", ":Help".cyan());
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

// ── Daemon IPC ───────────────────────────────────────────────────────────────

async fn daemon_request(
    req: DaemonRequest,
    uds_path: Option<&str>,
) -> anyhow::Result<DaemonResponse> {
    let uds = uds_path.unwrap_or(argus_core::DEFAULT_UDS_PATH);
    let mut stream = UnixStream::connect(uds)
        .await
        .map_err(|e| anyhow::anyhow!("connect to daemon at {uds} failed: {e}"))?;
    send_daemon_request(&mut stream, req).await
}

/// One framed request/response exchange on an open daemon connection.
/// Previously copy-pasted across consolidate/status/clear.
///
/// The response length is capped like the daemon's request cap: a corrupt
/// or hostile stream must not turn 4 bytes into a multi-gigabyte alloc.
async fn send_daemon_request(
    stream: &mut UnixStream,
    req: DaemonRequest,
) -> anyhow::Result<DaemonResponse> {
    const MAX_RESPONSE_LEN: usize = 64 * 1024 * 1024;

    let payload = bincode::serialize(&req).map_err(|e| anyhow::anyhow!("serialize: {e}"))?;
    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    stream.write_all(&payload).await?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;
    if resp_len > MAX_RESPONSE_LEN {
        anyhow::bail!("oversized response ({resp_len} bytes)");
    }
    let mut resp_buf = vec![0u8; resp_len];
    stream.read_exact(&mut resp_buf).await?;

    bincode::deserialize(&resp_buf).map_err(|e| anyhow::anyhow!("deserialize: {e}"))
}

fn cmd_consolidate(uds_path: Option<&str>) -> Result<i32> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let resp = daemon_request(DaemonRequest::RequestConsolidation, uds_path).await?;
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

fn cmd_status(uds_path: Option<&str>) -> Result<i32> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let resp = daemon_request(DaemonRequest::GetStatus, uds_path).await?;
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

fn cmd_clear(uds_path: Option<&str>) -> Result<i32> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let resp = daemon_request(DaemonRequest::ClearDb, uds_path).await?;
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

    let target_map: std::collections::HashMap<&str, &CleanTarget> =
        targets.iter().map(|t| (t.id.as_str(), t)).collect();

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
    if !dry_run {
        println!(
            "{}",
            "☻ First time? Run argus clean --dry-run first to preview changes".yellow()
        );
    }
    println!(
        "{} {}",
        "Free space:".bold(),
        format_size(free_space_bytes()).cyan()
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
            // The orphan scan already knows how many installed apps it
            // considered; a second find_installed_apps pass used to double
            // the cost (one mdls spawn + bundle walk per app).
            println!(
                "  ✓ Found {} active/installed apps",
                orphaned.installed_app_count.to_string().cyan()
            );
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
    // Listing only at this point. These commands are irreversible (docker
    // builder prune drops build cache), so they must run after the user has
    // confirmed the cleanup below — they used to execute before the prompt.
    #[cfg(feature = "shell-cmds")]
    let shell_cmds = default_shell_cmd_targets();
    #[cfg(feature = "shell-cmds")]
    {
        println!("➤ {}", "Shell Commands".bold());
        for cmd in &shell_cmds {
            if dry_run {
                println!("  ☻ {} (dry-run, skipped)", cmd.label.white());
            } else {
                println!(
                    "  ☻ {} (pending, run after confirmation)",
                    cmd.label.white()
                );
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

    #[cfg(feature = "shell-cmds")]
    {
        for cmd in &shell_cmds {
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
        println!();
    }

    let report = exec_clean(&plan.items).map_err(|e| anyhow::anyhow!("exec clean: {e}"))?;
    print_clean_report(&report);
    Ok(0)
}

/// Free space on the filesystem holding `/`, queried via statfs/statvfs
/// syscalls. Earlier this shelled out to `df -k /`, which the repo hard
/// constraints forbid and which breaks on PATH-less environments.
#[cfg(target_os = "macos")]
fn free_space_bytes() -> u64 {
    let c_path = match std::ffi::CString::new("/") {
        Ok(p) => p,
        Err(_) => return 0,
    };
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return 0;
    }
    (stat.f_bavail as u64).saturating_mul(stat.f_bsize as u64)
}

#[cfg(not(target_os = "macos"))]
fn free_space_bytes() -> u64 {
    let c_path = match std::ffi::CString::new("/") {
        Ok(p) => p,
        Err(_) => return 0,
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return 0;
    }
    (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64)
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

// ── Brew ────────────────────────────────────────────────────────────────────

#[cfg(feature = "cleanup")]
fn cmd_brew(formula: bool, cask: bool, dry_run: bool, yes: bool) -> Result<i32> {
    if !is_brew_available() {
        println!("{}", "brew is not installed or not in PATH".yellow());
        return Ok(1);
    }

    println!("{}", "Brew Packages".bold().cyan().underline());
    println!();
    println!(
        "{} {}",
        "Cache size:".bold(),
        format_size(brew_cache_size()).cyan()
    );
    println!();

    let filter = if formula && !cask {
        Some(BrewFilterType::Formula)
    } else if cask && !formula {
        Some(BrewFilterType::Cask)
    } else {
        None
    };

    let all_packages = list_brew_packages(None);
    if all_packages.is_empty() {
        println!("{}", "no brew packages found".yellow());
        return Ok(0);
    }

    let packages: Vec<_> = match filter {
        Some(BrewFilterType::All) | None => all_packages,
        Some(BrewFilterType::Formula) => all_packages
            .into_iter()
            .filter(|p| p.package_type == BrewPackageType::Formula)
            .collect(),
        Some(BrewFilterType::Cask) => all_packages
            .into_iter()
            .filter(|p| p.package_type == BrewPackageType::Cask)
            .collect(),
    };

    // 按 last_used 升序 (None 排最前 = 最久没用)
    let mut sorted = packages;
    sorted.sort_by(|a, b| match (&a.last_used, &b.last_used) {
        (None, None) => b.size.cmp(&a.size),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a_dt), Some(b_dt)) => a_dt.cmp(b_dt),
    });

    println!(
        "{} {} packages (sorted by last used, oldest first)\n",
        "Found".bold(),
        sorted.len().to_string().cyan()
    );

    // 分组: formula 和 cask 分别打印
    let formulae: Vec<_> = sorted
        .iter()
        .filter(|p| p.package_type == BrewPackageType::Formula)
        .collect();
    let casks: Vec<_> = sorted
        .iter()
        .filter(|p| p.package_type == BrewPackageType::Cask)
        .collect();

    if !formulae.is_empty() && filter.is_none() {
        println!("  {} ({})", "Formula".bold(), formulae.len());
        print_brew_list(&formulae);
        println!();
    } else if !formulae.is_empty() {
        print_brew_list(&formulae);
        println!();
    }

    if !casks.is_empty() && filter.is_none() {
        println!("  {} ({})", "Cask".bold(), casks.len());
        print_brew_list(&casks);
        println!();
    } else if !casks.is_empty() {
        print_brew_list(&casks);
        println!();
    }

    if dry_run {
        println!(
            "{}",
            "[dry-run] no packages were uninstalled".yellow().bold()
        );
        return Ok(0);
    }

    // 交互式选择要卸载的包
    let selections: Vec<String> = sorted
        .iter()
        .map(|p| {
            let time_str = format_brew_last_used(p.last_used);
            let size = format_size(p.size);
            let deps = if p.dependents > 0 {
                format!(" deps:{}", p.dependents)
            } else {
                String::new()
            };
            format!(
                "{:<30} {:>9}  {:>5}  {}{}",
                p.name,
                size,
                p.package_type.label(),
                time_str,
                deps
            )
        })
        .collect();

    let sel = inquire::Select::new(
        "Select package to uninstall (↑↓/j/k to move, type to filter, Esc to cancel):",
        selections,
    )
    .with_page_size(15)
    .with_vim_mode(true)
    .with_help_message("↑↓ navigate • type to filter • Enter confirm • Esc cancel")
    .prompt();

    let idx = match sel {
        Ok(chosen) => sorted.iter().position(|p| {
            let time_str = format_brew_last_used(p.last_used);
            let size = format_size(p.size);
            let deps = if p.dependents > 0 {
                format!(" deps:{}", p.dependents)
            } else {
                String::new()
            };
            format!(
                "{:<30} {:>9}  {:>5}  {}{}",
                p.name,
                size,
                p.package_type.label(),
                time_str,
                deps
            ) == chosen
        }),
        Err(_) => None,
    };

    let pkg = match idx {
        Some(i) => &sorted[i],
        None => {
            println!("{}", "cancelled".yellow());
            return Ok(0);
        }
    };

    println!("\n{} {}", "Selected:".bold(), pkg.name.cyan().bold());
    println!("  {}  {}", "type:".bold(), pkg.package_type.label());
    println!("  {}  {}", "version:".bold(), pkg.version);
    println!("  {}  {}", "size:".bold(), format_size(pkg.size).green());
    println!(
        "  {}  {}",
        "last used:".bold(),
        format_brew_last_used(pkg.last_used)
    );
    if !pkg.description.is_empty() {
        println!("  {}  {}", "desc:".bold(), pkg.description);
    }
    // The list view cannot afford a per-package `brew uses` pass, so the
    // dependency lookup happens once for the selected package. Uninstall
    // runs with --force: brew itself will not refuse, this warning must.
    let dependents = brew_dependents_of(&pkg.name);
    if !dependents.is_empty() {
        println!(
            "  {}  {} other package(s) depend on this: {}",
            "dependents:".bold(),
            dependents.len().to_string().yellow(),
            dependents.join(", ").yellow()
        );
    }

    if !yes {
        let proceed = inquire::Confirm::new(&format!("Uninstall {}?", pkg.name))
            .with_default(false)
            .prompt()?;
        if !proceed {
            println!("{}", "cancelled".yellow());
            return Ok(0);
        }
    }

    let report = uninstall_brew_package(pkg).map_err(|e| anyhow::anyhow!("uninstall: {e}"))?;
    print_clean_report(&report);
    Ok(0)
}

#[cfg(feature = "cleanup")]
fn print_brew_list(packages: &[&argus_core::BrewPackage]) {
    for pkg in packages {
        let time_str = format_brew_last_used(pkg.last_used);
        let size = format_size(pkg.size);
        let deps = if pkg.dependents > 0 {
            format!(" deps:{}", pkg.dependents)
        } else {
            String::new()
        };
        println!(
            "    {:>12}  {:>9}  {:>5}  {}{}  {}",
            time_str,
            size,
            pkg.package_type.label(),
            pkg.name,
            deps,
            pkg.description.truncate_ellipsis(40),
        );
    }
}

#[cfg(feature = "cleanup")]
fn format_brew_last_used(dt: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match dt {
        None => "never".to_string(),
        Some(dt) => {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(dt);
            let days = duration.num_days();
            if days == 0 {
                "today".to_string()
            } else if days == 1 {
                "yesterday".to_string()
            } else if days < 30 {
                format!("{}d ago", days)
            } else if days < 365 {
                format!("{}mo ago", days / 30)
            } else {
                format!("{}y ago", days / 365)
            }
        }
    }
}

trait TruncateEllipsis {
    fn truncate_ellipsis(&self, max_len: usize) -> String;
}

impl TruncateEllipsis for str {
    fn truncate_ellipsis(&self, max_len: usize) -> String {
        if self.len() <= max_len {
            return self.to_string();
        }
        // Byte slicing panics on non-char boundaries (e.g. CJK brew
        // descriptions); cut on the last full character instead.
        let mut end = max_len.saturating_sub(3);
        while end > 0 && !self.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &self[..end])
    }
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

fn print_delta_summary(path: &Path, from_ms: u64, to_ms: u64, summary: &DeltaSummary) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_ascii() {
        assert_eq!("abcdefgh".truncate_ellipsis(40), "abcdefgh");
        assert_eq!("abcdefgh".truncate_ellipsis(6), "abc...");
    }

    /// Multi-byte descriptions (common in brew casks) must not panic on a
    /// non-char-boundary slice.
    #[test]
    fn test_truncate_cjk_no_panic() {
        let s = "图形界面工具用于管理磁盘空间和系统清理";
        let out = s.truncate_ellipsis(40);
        assert!(out.ends_with("..."));
        assert!(out.len() <= 40);
    }

    #[test]
    fn test_truncate_mixed_boundary() {
        // 3-byte chars: cutting at byte 10 would split a char.
        let s = "ääääääää";
        let out = s.truncate_ellipsis(10);
        assert!(out.ends_with("..."));
    }
}
