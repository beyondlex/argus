mod config;
mod daemonize;
mod debounce;
mod ipc_server;
mod retention;
mod watcher;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tokio::sync::{mpsc, Mutex};
use tracing_subscriber::EnvFilter;

use argus_core::{init_db, open_db, DeltaEvent};
use daemonize::{DaemonGuard, ServiceTemplate};

pub(crate) static SHOULD_QUIT: AtomicBool = AtomicBool::new(false);

#[derive(Parser)]
#[command(
    name = "argusd",
    version,
    about = "Argus daemon — background disk change monitor"
)]
struct Args {
    #[arg(long, help = "Path to config file")]
    config: Option<String>,

    #[arg(long, help = "Log level [trace, debug, info, warn, error]")]
    log_level: Option<String>,

    #[arg(long, help = "Override UDS socket path")]
    uds_path: Option<String>,

    #[arg(short, long, help = "Daemonize (fork to background)")]
    daemon: bool,

    #[arg(long, value_enum, help = "Generate service manager config and exit")]
    generate_service: Option<ServiceTemplate>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Stop the running daemon")]
    Stop,
}

fn main() {
    let args = Args::parse();

    if let Some(cmd) = &args.command {
        match cmd {
            Command::Stop => {
                DaemonGuard::stop();
                return;
            }
        }
    }

    if let Some(template) = args.generate_service {
        DaemonGuard::print_service(template);
        return;
    }

    let _guard = if args.daemon {
        Some(DaemonGuard::daemonize().expect("failed to daemonize"))
    } else {
        None
    };

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(run(args));
}

async fn run(args: Args) {
    let mut config = if let Some(ref path) = args.config {
        config::load_config_from(path)
    } else {
        config::load_config()
    };

    let log_filter = if let Some(level) = args.log_level.clone() {
        EnvFilter::new(level)
    } else if config.log_enabled {
        config
            .log_level
            .clone()
            .or_else(|| std::env::var("RUST_LOG").ok())
            .map(EnvFilter::new)
            .unwrap_or_else(|| EnvFilter::new("info"))
    } else {
        EnvFilter::new("off")
    };

    tracing_subscriber::fmt()
        .with_env_filter(log_filter)
        .with_target(false)
        .init();

    ctrlc::set_handler(|| {
        SHOULD_QUIT.store(true, Ordering::Relaxed);
    })
    .expect("failed to set ctrl-c handler");

    if let Some(ref uds_path) = args.uds_path {
        config.uds_path = uds_path.clone();
    }

    tracing::info!("argusd starting, watching {:?}", config.watch_dirs);

    let db_path = argus_core::default_db_path();
    let conn = open_db(&db_path).expect("failed to open database");
    init_db(&conn).expect("failed to initialize database");
    let db = Arc::new(Mutex::new(conn));

    let (event_tx, event_rx) = mpsc::channel::<DeltaEvent>(1024);

    let watcher_handle = watcher::start_watcher(config.watch_dirs.clone(), event_tx);

    let debounce_db = db.clone();
    let debounce_handle = debounce::start_debounce(
        event_rx,
        debounce_db,
        Duration::from_secs(config.debounce_seconds),
    );

    let retention_db = db.clone();
    let retention_handle = retention::start_retention_worker(retention_db, config.clone());

    let ipc_db = db.clone();
    let ipc_handle = ipc_server::start_ipc_server(&config.uds_path, ipc_db);

    wait_for_shutdown().await;

    tracing::info!("shutting down...");
    watcher_handle.store(false, Ordering::Relaxed);
    drop(watcher_handle);
    debounce_handle.await.expect("debounce engine failed");
    retention_handle.abort();
    ipc_handle.abort();
    tracing::info!("argusd stopped");
}

async fn wait_for_shutdown() {
    while !SHOULD_QUIT.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
