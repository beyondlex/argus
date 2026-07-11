use anyhow::Result;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let config_path = argus_tui::util::default_config_path();
    let tui_config = argus_tui::config::load_config(&config_path);

    let (tx, rx) = mpsc::channel(256);

    let mut app = argus_tui::app::App::new(tui_config.clone(), tx.clone(), rx);

    // Build tree for current working directory
    app.rebuild_tree();

    // Try to connect to daemon
    let uds_path = tui_config.daemon.uds_path.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        if let Ok(mut client) = argus_tui::ipc_client::IpcClient::connect(&uds_path).await {
            if client.ping().await.is_ok() {
                let _ = tx_clone
                    .send(argus_tui::app::AppMessage::DaemonConnected(client))
                    .await;
            }
        }
    });

    // Optionally start auto-scan
    let auto_scan = app.config.browsing.auto_scan_on_start;
    if auto_scan {
        argus_tui::handler::start_scan(&mut app);
    }

    argus_tui::event::run(&mut app).await?;

    Ok(())
}
