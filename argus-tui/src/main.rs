use anyhow::Result;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let config_path = argus_tui::util::default_config_path();
    let tui_config = argus_tui::config::load_config(&config_path);

    let (tx, rx) = mpsc::channel(256);

    let mut app = argus_tui::app::App::new(tui_config, tx, rx);

    // Load all snapshots into cache
    app.load_all_snapshots();

    // Build tree for current working directory
    app.rebuild_tree();

    // Optionally start auto-scan
    let auto_scan = app.config.browsing.auto_scan_on_start;
    if auto_scan {
        argus_tui::handler::start_scan(&mut app);
    }

    argus_tui::event::run(&mut app).await?;

    Ok(())
}
