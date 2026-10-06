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
            // Everything this arm acts on must report consumed: a `false`
            // lets the key fall through to browsing, where `N` is the
            // permanent-delete shortcut — searching "N" then pressing `N`
            // (previous match) also opened the delete prompt.
            match key.code {
                KeyCode::Char('n') => {
                    app.cycle_match(true);
                    true
                }
                KeyCode::Char('N') => {
                    app.cycle_match(false);
                    true
                }
                KeyCode::Char('/') => {
                    app.search_word.clear();
                    app.refresh_current_filtered();
                    app.search_mode = SearchMode::Input;
                    true
                }
                // The list header advertises "Enter edit"; the key used to
                // fall through and enter the directory under the cursor
                // instead (discarding the search).
                KeyCode::Enter => {
                    app.search_mode = SearchMode::Input;
                    true
                }
                KeyCode::Esc => {
                    app.search_word.clear();
                    app.refresh_current_filtered();
                    app.search_mode = SearchMode::Inactive;
                    true
                }
                // j/k and every other key stay unconsumed: movement and the
                // rest of browsing keep working while a search is active.
                _ => false,
            }
        }
        SearchMode::Inactive => false,
    }
}
