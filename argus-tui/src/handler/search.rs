use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, SearchMode};

/// Handle search-mode input keys. Returns true if the key was consumed.
pub(crate) fn handle_search_keys(key: KeyEvent, app: &mut App) -> bool {
    match app.search_mode {
        SearchMode::Input => {
            match key.code {
                KeyCode::Char(c) => {
                    app.search_word.push(c);
                    app.recompute_matches();
                }
                KeyCode::Backspace => {
                    app.search_word.pop();
                    app.recompute_matches();
                }
                KeyCode::Enter => {
                    if app.search_word.is_empty() {
                        app.recompute_matches();
                        app.search_mode = SearchMode::Inactive;
                    } else {
                        app.search_mode = SearchMode::Active;
                    }
                }
                KeyCode::Esc => {
                    app.search_word.clear();
                    app.recompute_matches();
                    app.search_mode = SearchMode::Inactive;
                }
                _ => {}
            }
            true
        }
        SearchMode::Active => {
            match key.code {
                KeyCode::Char('n') => crate::search::jump_to_next_match(app, 1),
                KeyCode::Char('N') => crate::search::jump_to_next_match(app, -1),
                KeyCode::Char('/') => {
                    app.search_word.clear();
                    app.recompute_matches();
                    app.search_mode = SearchMode::Input;
                }
                KeyCode::Esc => {
                    app.search_word.clear();
                    app.recompute_matches();
                    app.search_mode = SearchMode::Inactive;
                    return true;
                }
                _ => {}
            }
            false
        }
        SearchMode::Inactive => false,
    }
}
