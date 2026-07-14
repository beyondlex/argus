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
            app.view_root_path = std::path::PathBuf::from(&path);
            app.finder_state = None;
            app.mode = AppMode::Browsing;
            app.rebuild_tree();
            app.set_info(format!("changed root to {path}"), 3);
        }
        FinderAction::Cancel => {
            app.finder_state = None;
            app.mode = AppMode::Browsing;
        }
        FinderAction::Redraw | FinderAction::None => {}
    }
}
