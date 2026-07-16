use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, SearchMode};

/// Handle search-mode input keys. Returns true if the key was consumed.
pub(crate) fn handle_search_keys(key: KeyEvent, app: &mut App) -> bool {
    match app.search_mode {
        SearchMode::Input => {
            match key.code {
                KeyCode::Char(c) => {
                    app.search_word.push(c);
                    if app.search_word.is_empty() {
                        app.refresh_current_filtered();
                    } else {
                        app.apply_search();
                    }
                }
                KeyCode::Backspace => {
                    app.search_word.pop();
                    if app.search_word.is_empty() {
                        app.refresh_current_filtered();
                    } else {
                        app.apply_search();
                    }
                }
                KeyCode::Enter => {
                    if app.search_word.is_empty() {
                        app.refresh_current_filtered();
                        app.search_mode = SearchMode::Inactive;
                    } else {
                        app.apply_search();
                        app.search_mode = SearchMode::Active;
                    }
                }
                KeyCode::Esc => {
                    app.search_word.clear();
                    app.refresh_current_filtered();
                    app.search_mode = SearchMode::Inactive;
                }
                _ => {}
            }
            true
        }
        SearchMode::Active => {
            match key.code {
                KeyCode::Char('n') => app.cycle_match(true),
                KeyCode::Char('N') => app.cycle_match(false),
                KeyCode::Char('/') => {
                    app.search_word.clear();
                    app.refresh_current_filtered();
                    app.search_mode = SearchMode::Input;
                }
                KeyCode::Esc => {
                    app.search_word.clear();
                    app.refresh_current_filtered();
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
