use crossterm::event::KeyEvent;
use ratatui_finder::FinderAction;
use std::path::PathBuf;

use crate::app::{App, AppMode};

/// Expand `~` to `$HOME` in a path string, since ratatui_finder does not
/// expand it in the confirmed path (only internally for FS operations).
fn expand_tilde(path: &str) -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        if path == "~" {
            return PathBuf::from(home);
        }
        if let Some(rest) = path.strip_prefix("~/") {
            let mut p = PathBuf::from(home);
            p.push(rest);
            return p;
        }
    }
    PathBuf::from(path)
}

pub(crate) fn handle_finder_key(key: KeyEvent, app: &mut App) {
    let Some(finder) = &mut app.finder_state else {
        return;
    };

    let action = finder.handle_key(key);
    match action {
        FinderAction::Confirm(path) => {
            let p = expand_tilde(&path);
            // Normalize trailing separator
            let parent = p.parent();
            let normalized = if p.to_string_lossy().ends_with('/') && p.file_name().is_none() {
                parent.map(|pp| pp.to_path_buf()).unwrap_or(p)
            } else {
                p
            };
            app.view_root_path = normalized;
            app.finder_state = None;
            app.mode = AppMode::Browsing;
            app.rebuild_tree();
            app.set_info(
                format!("changed root to {}", app.view_root_path.display()),
                3,
            );
        }
        FinderAction::Cancel => {
            app.finder_state = None;
            app.mode = AppMode::Browsing;
        }
        FinderAction::Redraw | FinderAction::None => {}
    }
}
