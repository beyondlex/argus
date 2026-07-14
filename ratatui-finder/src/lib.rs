pub mod fs;
pub mod matcher;
pub mod widgets;

pub use widgets::render_finder_popup;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fs::RawItem;

/// Filter mode: what types of entries to show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FinderMode {
    Dir,
    File,
    Both,
}

impl Default for FinderMode {
    fn default() -> Self {
        Self::Both
    }
}

/// Configuration for the Finder component.
#[derive(Debug, Clone)]
pub struct FinderConfig {
    pub mode: FinderMode,
    pub initial_path: String,
    pub extensions: Option<Vec<String>>,
}

impl Default for FinderConfig {
    fn default() -> Self {
        Self {
            mode: FinderMode::Both,
            initial_path: "~".to_string(),
            extensions: None,
        }
    }
}

/// An item in the finder results list, ready for rendering.
#[derive(Debug, Clone)]
pub struct FinderItem {
    pub name: String,
    pub display: String,
    pub is_dir: bool,
    pub is_self: bool,
    pub display_offset: usize,
    pub match_positions: Vec<usize>,
}

/// Actions the Finder can signal to the host application.
#[derive(Debug, Clone, PartialEq)]
pub enum FinderAction {
    None,
    Confirm(String),
    Cancel,
    Redraw,
}

/// The main state machine for the Finder component.
#[derive(Debug, Clone)]
pub struct FinderState {
    /// Current input text
    pub input: String,
    /// Cursor position (byte offset)
    pub cursor: usize,
    /// Results list for rendering
    pub items: Vec<FinderItem>,
    /// Selected index (0-based)
    pub selected: usize,
    /// Configuration (fixed after initialization)
    pub config: FinderConfig,

    // Internal
    parent_display: String,
    raw_items: Vec<RawItem>,
}

impl FinderState {
    /// Create a new FinderState with the given configuration.
    pub fn new(config: FinderConfig) -> Self {
        let mut state = Self {
            input: config.initial_path.clone(),
            cursor: config.initial_path.len(),
            items: Vec::new(),
            selected: 0,
            parent_display: String::new(),
            raw_items: Vec::new(),
            config,
        };
        state.refresh();
        state
    }

    /// Refresh the results list based on current input.
    pub fn refresh(&mut self) {
        self.update_items();
        if self.selected >= self.items.len() && !self.items.is_empty() {
            self.selected = self.items.len() - 1;
        } else if self.items.is_empty() {
            self.selected = 0;
        }
    }

    /// Handle a key event and return an action for the host.
    pub fn handle_key(&mut self, key: KeyEvent) -> FinderAction {
        match key.code {
            KeyCode::Enter => {
                if self.items.is_empty() {
                    return FinderAction::Confirm(self.input.clone());
                }
                let selected = self.selected.min(self.items.len().saturating_sub(1));
                let item = &self.items[selected];
                if item.is_self {
                    // Self-item: confirm the directory itself
                    FinderAction::Confirm(self.input.clone())
                } else {
                    let path = if self.input.ends_with('/') {
                        format!("{}{}", self.input, item.name)
                    } else {
                        // Compute parent dir from input
                        let parent = self.parent_display.clone();
                        format!("{}{}", parent, item.name)
                    };
                    FinderAction::Confirm(path)
                }
            }
            KeyCode::Esc => {
                FinderAction::Cancel
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                FinderAction::Cancel
            }
            KeyCode::Tab => {
                self.tab_complete();
                FinderAction::Redraw
            }
            KeyCode::Up => {
                self.move_selection(-1);
                FinderAction::Redraw
            }
            KeyCode::Down => {
                self.move_selection(1);
                FinderAction::Redraw
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                self.move_selection(-1);
                FinderAction::Redraw
            }
            KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                self.move_selection(1);
                FinderAction::Redraw
            }
            KeyCode::Home => {
                self.cursor = 0;
                FinderAction::Redraw
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                FinderAction::Redraw
            }
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                self.cursor = 0;
                FinderAction::Redraw
            }
            KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => {
                self.cursor = self.input.len();
                FinderAction::Redraw
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor = self.cursor.saturating_sub(1);
                }
                FinderAction::Redraw
            }
            KeyCode::Right => {
                if self.cursor < self.input.len() {
                    self.cursor = self.cursor.saturating_add(1).min(self.input.len());
                }
                FinderAction::Redraw
            }
            KeyCode::Backspace => {
                if self.cursor > 0 && !self.input.is_empty() {
                    let before = &self.input[..self.cursor];
                    let new_cursor = before
                        .char_indices()
                        .rev()
                        .next()
                        .map(|(i, _c)| i)
                        .unwrap_or(0);
                    self.input = format!(
                        "{}{}",
                        &self.input[..new_cursor],
                        &self.input[self.cursor..]
                    );
                    self.cursor = new_cursor;
                    self.refresh();
                }
                FinderAction::Redraw
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    let next = self.input[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _c)| self.cursor + i)
                        .unwrap_or(self.input.len());
                    self.input = format!("{}{}", &self.input[..self.cursor], &self.input[next..]);
                    self.refresh();
                }
                FinderAction::Redraw
            }
            KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                self.go_up_dir();
                FinderAction::Redraw
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                self.input.clear();
                self.cursor = 0;
                self.refresh();
                FinderAction::Redraw
            }
            KeyCode::Char(c) => {
                if self.cursor <= self.input.len() {
                    self.input.insert(self.cursor, c);
                    self.cursor += 1;
                    self.refresh();
                }
                FinderAction::Redraw
            }
            _ => FinderAction::None,
        }
    }

    /// Move selection by delta.
    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let new = self.selected as isize + delta;
        if new < 0 {
            self.selected = 0;
        } else if new >= self.items.len() as isize {
            self.selected = self.items.len() - 1;
        } else {
            self.selected = new as usize;
        }
    }

    /// Tab-complete: replace the last path segment with the selected item's name.
    fn tab_complete(&mut self) {
        if self.items.is_empty() || self.selected >= self.items.len() {
            return;
        }
        let item = &self.items[self.selected];
        if item.is_self {
            return;
        }
        let name = &item.name;
        if self.input.ends_with('/') {
            self.input.push_str(name);
        } else {
            // Replace the partial segment after the last '/'
            if let Some(slash_pos) = self.input.rfind('/') {
                self.input.truncate(slash_pos + 1);
                self.input.push_str(name);
            } else {
                self.input = name.clone();
            }
        }
        self.cursor = self.input.len();
        self.refresh();
    }

    /// Navigate to parent directory (Ctrl-w).
    fn go_up_dir(&mut self) {
        if self.input.is_empty() || self.input == "/" {
            return;
        }
        // Remove trailing slash if present
        let trimmed = if self.input.ends_with('/') && self.input.len() > 1 {
            &self.input[..self.input.len() - 1]
        } else {
            &self.input
        };
        // Find last slash
        if let Some(slash_pos) = trimmed.rfind('/') {
            self.input = if slash_pos == 0 {
                "/".to_string()
            } else {
                format!("{}/", &trimmed[..slash_pos])
            };
        } else {
            // No slash, go to parent (the home/root context)
            let parent = fs::parent(&self.input);
            if parent != self.input {
                self.input = parent;
            }
        }
        self.cursor = self.input.len();
        self.refresh();
    }

    /// Update items based on current input path.
    fn update_items(&mut self) {
        let input = self.input.clone();

        if input.is_empty() {
            self.items.clear();
            self.raw_items.clear();
            return;
        }

        let expanded = fs::expand(&input);

        if input.ends_with('/') {
            // Listing Mode: list directory contents
            self.raw_items = fs::list(&expanded, self.config.mode);
            self.parent_display = input.clone();
            self.items = self.build_listing_items(&expanded, &input);
        } else if !input.contains('/') {
            // No slash — could be a directory or a partial name
            if fs::is_dir(&expanded) {
                // Auto-listing: it's a directory, list its contents
                let dir_path = if input == "~" {
                    "~/".to_string()
                } else {
                    format!("{}/", input)
                };
                self.raw_items = fs::list(&expanded, self.config.mode);
                self.parent_display = dir_path.clone();
                self.items = self.build_listing_items(&expanded, &dir_path);
            } else {
                // Matching Mode: match partial name against current directory
                let cwd = ".";
                self.raw_items = fs::list(cwd, self.config.mode);
                self.parent_display = String::new();
                let matched = matcher::match_items(&self.raw_items, &input);
                self.items = matched
                    .into_iter()
                    .map(|m| FinderItem {
                        display: m.name.clone(),
                        name: m.name,
                        is_dir: m.is_dir,
                        is_self: false,
                        display_offset: 0,
                        match_positions: m.match_positions,
                    })
                    .collect();
            }
        } else {
            // Contains '/' but doesn't end with '/' — Matching Mode or Auto-listing
            // Check if the full path is a directory (auto-listing)
            if fs::is_dir(&expanded) {
                let dir_path = format!("{}/", input);
                self.raw_items = fs::list(&expanded, self.config.mode);
                self.parent_display = dir_path.clone();
                self.items = self.build_listing_items(&expanded, &dir_path);
            } else {
                let slash_pos = input.rfind('/').unwrap_or(0);
                let parent_dir = &input[..=slash_pos];
                let partial = &input[slash_pos + 1..];

                let expanded_parent = fs::expand(parent_dir);
                self.raw_items = fs::list(&expanded_parent, self.config.mode);
                self.parent_display = parent_dir.to_string();

                let matched = matcher::match_items(&self.raw_items, partial);
                self.items = matched
                    .into_iter()
                    .map(|m| {
                        FinderItem {
                            display: format!("{}{}", parent_dir, m.name),
                            name: m.name,
                            is_dir: m.is_dir,
                            is_self: false,
                            display_offset: parent_dir.len(),
                            match_positions: m.match_positions,
                        }
                    })
                    .collect();
            }
        }
    }

    /// Build listing items for a directory (with self-item as first entry).
    fn build_listing_items(&mut self, expanded_dir: &str, display_dir: &str) -> Vec<FinderItem> {
        let mut items = Vec::new();

        // Self-item: the current directory itself
        let dir_name = fs::basename(expanded_dir);
        let dir_display = if dir_name.is_empty() {
            display_dir.to_string()
        } else {
            let parent = fs::parent(expanded_dir);
            let contracted_parent = fs::contract(&parent);
            if contracted_parent.ends_with('/') {
                format!("{}{}", contracted_parent, dir_name)
            } else {
                format!("{}/{}", contracted_parent, dir_name)
            }
        };

        items.push(FinderItem {
            name: dir_name.clone(),
            display: dir_display,
            is_dir: true,
            is_self: true,
            display_offset: 0,
            match_positions: Vec::new(),
        });

        // Directory contents
        for raw in &self.raw_items {
            items.push(FinderItem {
                display: format!("{}{}", display_dir, raw.name),
                name: raw.name.clone(),
                is_dir: raw.is_dir,
                is_self: false,
                display_offset: display_dir.len(),
                match_positions: Vec::new(),
            });
        }

        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn test_new_finder_state() {
        let state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "~".to_string(),
            extensions: None,
        });
        assert_eq!(state.input, "~");
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn test_handle_key_char_input() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "".to_string(),
            extensions: None,
        });
        let action = state.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
        assert_eq!(action, FinderAction::Redraw);
        assert_eq!(state.input, "a");
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn test_handle_key_esc() {
        let mut state = FinderState::new(FinderConfig::default());
        let action = state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
        assert_eq!(action, FinderAction::Cancel);
    }

    #[test]
    fn test_handle_key_ctrl_c() {
        let mut state = FinderState::new(FinderConfig::default());
        let action = state.handle_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(action, FinderAction::Cancel);
    }

    #[test]
    fn test_handle_key_backspace() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "ab".to_string(),
            extensions: None,
        });
        state.cursor = 2;
        let action = state.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()));
        assert_eq!(action, FinderAction::Redraw);
        assert_eq!(state.input, "a");
    }

    #[test]
    fn test_handle_key_delete() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "ab".to_string(),
            extensions: None,
        });
        state.cursor = 0;
        let action = state.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::empty()));
        assert_eq!(action, FinderAction::Redraw);
        assert_eq!(state.input, "b");
    }

    #[test]
    fn test_move_selection() {
        let mut state = FinderState::new(FinderConfig::default());
        // We need items to test selection
        state.items = vec![
            FinderItem {
                name: "a".into(),
                display: "a".into(),
                is_dir: false,
                is_self: false,
                display_offset: 0,
                match_positions: vec![],
            },
            FinderItem {
                name: "b".into(),
                display: "b".into(),
                is_dir: false,
                is_self: false,
                display_offset: 0,
                match_positions: vec![],
            },
        ];
        state.selected = 0;
        state.move_selection(1);
        assert_eq!(state.selected, 1);
        state.move_selection(1);
        assert_eq!(state.selected, 1); // clamped
        state.move_selection(-1);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_go_up_dir() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "~/a/b/".to_string(),
            extensions: None,
        });
        state.go_up_dir();
        assert_eq!(state.input, "~/a/");
    }

    #[test]
    fn test_tab_complete() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "~/".to_string(),
            extensions: None,
        });
        // Ensure we have items
        let home = std::env::var("HOME").unwrap();
        state.refresh();
        // Tab-complete should be safe even if no items
        let _ = state.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
    }

    #[test]
    fn test_ctrl_w() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "~/a/b/c".to_string(),
            extensions: None,
        });
        let action = state.handle_key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(action, FinderAction::Redraw);
        assert_eq!(state.input, "~/a/b/");
    }

    #[test]
    fn test_ctrl_u() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "some text".to_string(),
            extensions: None,
        });
        let action = state.handle_key(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(action, FinderAction::Redraw);
        assert!(state.input.is_empty());
    }

    #[test]
    fn test_enter_on_empty_items() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "/tmp".to_string(),
            extensions: None,
        });
        state.items.clear();
        let action = state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        // Should confirm with current input
        assert_eq!(action, FinderAction::Confirm("/tmp".to_string()));
    }

    #[test]
    fn test_cursor_movement() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "hello".to_string(),
            extensions: None,
        });
        state.cursor = 5;

        // Left
        let _ = state.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::empty()));
        assert_eq!(state.cursor, 4);

        // Right
        let _ = state.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::empty()));
        assert_eq!(state.cursor, 5);

        // Home
        let _ = state.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::empty()));
        assert_eq!(state.cursor, 0);

        // End
        let _ = state.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::empty()));
        assert_eq!(state.cursor, 5);
    }

    #[test]
    fn test_ctrl_a_and_ctrl_e() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "hello".to_string(),
            extensions: None,
        });
        state.cursor = 3;

        // Ctrl-a
        let _ = state.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(state.cursor, 0);

        // Ctrl-e
        let _ = state.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(state.cursor, 5);
    }

    #[test]
    fn test_listing_mode_on_slash_ending() {
        let mut state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: "/tmp/".to_string(),
            extensions: None,
        });
        assert!(!state.items.is_empty());
        // First item should be self-item
        assert!(state.items[0].is_self);
    }

    #[test]
    fn test_auto_listing_on_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir_path = tmp.path().to_string_lossy().to_string();
        // Create a file in the directory so list returns something
        std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();
        let state = FinderState::new(FinderConfig {
            mode: FinderMode::Both,
            initial_path: dir_path.clone(),
            extensions: None,
        });
        // Should auto-list since it's an existing directory
        assert!(!state.items.is_empty(), "items should not be empty for directory with content");
        assert!(state.items[0].is_self, "first item should be self-item");
    }
}