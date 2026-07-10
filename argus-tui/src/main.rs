use anyhow::Result;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let config_path = argus_tui::util::default_config_path();
    let tui_config = argus_tui::config::load_config(&config_path);

    let db_path = argus_core::default_db_path();
    let (tx, rx) = mpsc::channel(256);

    let mut app = argus_tui::app::App::new(tui_config, db_path, tx, rx);

    // Load scan history from SQLite into cache
    app.load_from_db();

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
