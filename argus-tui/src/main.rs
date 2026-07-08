use anyhow::Result;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    // Load config
    let config_path = argus_tui::util::default_config_path();
    let tui_config = argus_tui::config::load_config(&config_path);

    // Create message channel (capacity 256 for backpressure)
    let (tx, rx) = mpsc::channel(256);

    // Build app with both sender and receiver
    let mut app = argus_tui::app::App::new(tui_config, tx, rx);

    // Initialize from existing snapshots
    app.initialize_from_snapshots();

    // Run the event loop
    argus_tui::event::run(&mut app).await?;

    Ok(())
}
