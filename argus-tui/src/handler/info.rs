use crate::app::App;
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_info_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.info_data = None;
            app.info_ai = None;
            app.mode = app.prev_mode;
        }
        _ => {}
    }
}
