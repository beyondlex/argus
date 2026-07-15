use crossterm::event::KeyEvent;
use ratatui_finder::FinderAction;

use crate::app::{App, AppMode};

pub(crate) fn handle_finder_key(key: KeyEvent, app: &mut App) {
    let Some(finder) = &mut app.finder_state else {
        return;
    };

    let action = finder.handle_key(key);
    match action {
        FinderAction::Confirm(path) => {
            // Normalize trailing separator: Path::join handles trailing slashes,
            // but comparisons (==) and display are cleaner without them.
            let p: std::path::PathBuf = path.into();
            let parent = p.parent();
            let normalized = if p.to_string_lossy().ends_with('/') && p.file_name().is_none() {
                // e.g. "/Users/code/" → parent is "/Users/code" (unless root "/")
                parent.map(|pp| pp.to_path_buf()).unwrap_or(p)
            } else {
                p
            };
            app.view_root_path = normalized;
            app.finder_state = None;
            app.mode = AppMode::Browsing;
            app.rebuild_tree();
            app.set_info(format!("changed root to {}", app.view_root_path.display()), 3);
        }
        FinderAction::Cancel => {
            app.finder_state = None;
            app.mode = AppMode::Browsing;
        }
        FinderAction::Redraw | FinderAction::None => {}
    }
}
