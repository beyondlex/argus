mod config;
mod debounce;
mod ipc_server;
mod retention;
mod watcher;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Mutex};
use tracing_subscriber::EnvFilter;

use argus_core::{init_db, open_db, DeltaEvent};

pub(crate) static SHOULD_QUIT: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    ctrlc::set_handler(|| {
        SHOULD_QUIT.store(true, Ordering::Relaxed);
    })
    .expect("failed to set ctrl-c handler");

    let config = config::load_config();
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
