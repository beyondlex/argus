mod browsing;
mod command;
mod filter;
mod finder;
mod prompt;
mod search;

use crate::app::{App, AppMode};
use crossterm::event::KeyEvent;

pub use browsing::start_scan;

/// Handle keyboard events
pub fn handle_key(key: KeyEvent, app: &mut App) {
    match app.mode {
        AppMode::Browsing => browsing::handle_browsing_key(key, app),
        AppMode::DeletePrompt => prompt::handle_delete_prompt_key(key, app),
        AppMode::DeletePermanentPrompt => prompt::handle_delete_permanent_prompt_key(key, app),
        AppMode::Help => prompt::handle_help_key(key, app),
        AppMode::TimeHelp => prompt::handle_time_help_key(key, app),
        AppMode::Command => command::handle_command_key(key, app),
        AppMode::Finder => finder::handle_finder_key(key, app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{TreeLine, TreeNode};
    use crate::handler::browsing::{
        handle_browsing_key, handle_delete_action, handle_gg_double_tap, move_cursor,
        set_root_to_selected,
    };
    use crate::handler::command::{execute_command, handle_command_key};
    use crate::handler::filter::{adjust_filter_focus, handle_filter_pane_key};
    use crate::handler::prompt::{
        handle_delete_common, handle_delete_permanent_prompt_key, handle_delete_prompt_key,
        handle_help_key, handle_time_help_key,
    };
    use crate::handler::search::handle_search_keys;
    use crate::types::{FilterFocus, Focus, SearchMode};
    use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn file_node(name: &str, size: u64) -> FileNode {
        FileNode {
            name: name.to_string(),
            parent: None,
            is_dir: false,
            file_type: FileType::File,
            size,
            children: Vec::new(),
        }
    }

    fn dir_node(name: &str, children: Vec<(&str, NodeIndex)>) -> FileNode {
        FileNode {
            name: name.to_string(),
            parent: None,
            is_dir: true,
            file_type: FileType::Directory,
            size: 0,
            children: children
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    fn make_app(snap: Snapshot, scan_snap: Snapshot) -> App {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/tmp/test");
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.scan_cache.insert(PathBuf::from("/tmp/test"), scan_snap);
        app.update_tree_lines();
        app
    }

    // ── filter pane key handlers ─────────────────────────────────────────────

    #[test]
    fn test_filter_tab_cycles_forward() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;

        app.filter_focus = FilterFocus::TimePreset;
        handle_filter_pane_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut app);
        assert_eq!(app.filter_focus, FilterFocus::DeltaValue);

        handle_filter_pane_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut app);
        assert_eq!(app.filter_focus, FilterFocus::DeltaUnit);

        handle_filter_pane_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut app);
        assert_eq!(app.filter_focus, FilterFocus::TimePreset);
    }

    #[test]
    fn test_filter_backtab_cycles_reverse() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;

        app.filter_focus = FilterFocus::TimePreset;
        handle_filter_pane_key(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.filter_focus, FilterFocus::DeltaUnit);

        handle_filter_pane_key(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.filter_focus, FilterFocus::DeltaValue);

        handle_filter_pane_key(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.filter_focus, FilterFocus::TimePreset);
    }

    #[test]
    fn test_filter_tab_skips_time_when_custom() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;
        app.time_custom = true;

        app.filter_focus = FilterFocus::TimePreset;
        handle_filter_pane_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut app);
        assert_eq!(app.filter_focus, FilterFocus::DeltaValue);

        handle_filter_pane_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut app);
        assert_eq!(app.filter_focus, FilterFocus::DeltaUnit);

        handle_filter_pane_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut app);
        assert_eq!(app.filter_focus, FilterFocus::DeltaValue);
    }

    #[test]
    fn test_filter_esc_returns_to_tree() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;

        handle_filter_pane_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);
        assert_eq!(app.focus, Focus::Tree);
    }

    #[test]
    fn test_filter_enter_confirms_to_tree() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;

        handle_filter_pane_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.focus, Focus::Tree);
    }

    #[test]
    fn test_filter_digit_input_sets_value() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;
        app.filter_focus = FilterFocus::DeltaValue;

        handle_filter_pane_key(
            KeyEvent::new(KeyCode::Char('4'), KeyModifiers::empty()),
            &mut app,
        );
        assert!(app.delta_filter_active);
        assert_eq!(app.delta_filter_value, 4);

        handle_filter_pane_key(
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.delta_filter_value, 42);
    }

    #[test]
    fn test_filter_backspace_removes_digit() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;
        app.filter_focus = FilterFocus::DeltaValue;
        app.delta_filter_active = true;
        app.delta_filter_value = 42;

        handle_filter_pane_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.delta_filter_value, 4);

        handle_filter_pane_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
            &mut app,
        );
        assert!(!app.delta_filter_active);
    }

    #[test]
    fn test_filter_clear_resets_state() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;
        app.delta_filter_active = true;
        app.delta_filter_value = 500;

        handle_filter_pane_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::empty()),
            &mut app,
        );
        assert!(!app.delta_filter_active);
        assert_eq!(app.focus, Focus::Tree);
    }

    // ── delete prompt handlers ────────────────────────────────────────────────

    #[test]
    fn test_delete_prompt_no_dismisses() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::DeletePrompt;
        app.delete_target_path = Some(PathBuf::from("/tmp/test_file"));

        handle_delete_prompt_key(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.delete_target_path.is_none());
    }

    #[test]
    fn test_delete_prompt_esc_dismisses() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::DeletePrompt;
        app.delete_target_path = Some(PathBuf::from("/tmp/test_file"));

        handle_delete_prompt_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);

        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.delete_target_path.is_none());
    }

    #[test]
    fn test_delete_permanent_prompt_no_dismisses() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::DeletePermanentPrompt;
        app.delete_target_path = Some(PathBuf::from("/tmp/test_file"));

        handle_delete_permanent_prompt_key(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.delete_target_path.is_none());
    }

    #[test]
    fn test_delete_permanent_prompt_esc_dismisses() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::DeletePermanentPrompt;
        app.delete_target_path = Some(PathBuf::from("/tmp/test_file"));

        handle_delete_permanent_prompt_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.delete_target_path.is_none());
    }

    #[test]
    fn test_delete_permanent_prompt_yes_removes_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("test_file.txt");
        fs::write(&file_path, "content").unwrap();
        assert!(file_path.exists());

        let root_path = PathBuf::from("/tmp/test");
        let arena = vec![dir_node("test", vec![])];
        let snap = Snapshot::new(root_path.clone(), arena, 0);
        let scan_snap = Snapshot::new(root_path.clone(), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.mode = AppMode::DeletePermanentPrompt;
        app.delete_target_path = Some(file_path.clone());

        handle_delete_permanent_prompt_key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(!file_path.exists());
        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.delete_target_path.is_none());
    }

    // ── move_cursor ──────────────────────────────────────────────────────

    #[test]
    fn test_move_cursor_basic_down() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.filtered_tree_lines = vec![0, 1, 2];
        app.tree_lines = vec![
            TreeLine {
                depth: 0,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(
                        PathBuf::from("/"),
                        vec![file_node("root", 0)],
                        0,
                    )),
                    ROOT_NODE,
                ),
                expanded: true,
                has_scan_data: false,
                path: vec!["root".to_string()],
            },
            TreeLine {
                depth: 1,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(
                        PathBuf::from("/"),
                        vec![file_node("a", 0)],
                        0,
                    )),
                    ROOT_NODE,
                ),
                expanded: false,
                has_scan_data: false,
                path: vec!["root".to_string(), "a".to_string()],
            },
            TreeLine {
                depth: 1,
                node: TreeNode::Snapshot(
                    Arc::new(Snapshot::new(
                        PathBuf::from("/"),
                        vec![file_node("b", 0)],
                        0,
                    )),
                    ROOT_NODE,
                ),
                expanded: false,
                has_scan_data: false,
                path: vec!["root".to_string(), "b".to_string()],
            },
        ];
        app.cursor = 0;

        move_cursor(&mut app, 1);
        assert_eq!(app.cursor, 1);

        move_cursor(&mut app, 1);
        assert_eq!(app.cursor, 2);
    }

    #[test]
    fn test_move_cursor_basic_up() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.filtered_tree_lines = vec![0, 1, 2];
        app.cursor = 2;

        move_cursor(&mut app, -1);
        assert_eq!(app.cursor, 1);

        move_cursor(&mut app, -1);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_move_cursor_bounds_top() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.filtered_tree_lines = vec![0, 1, 2];
        app.cursor = 0;

        move_cursor(&mut app, -1);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_move_cursor_bounds_bottom() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.filtered_tree_lines = vec![0, 1, 2];
        app.cursor = 2;

        move_cursor(&mut app, 1);
        assert_eq!(app.cursor, 2);
    }

    #[test]
    fn test_move_cursor_empty() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.filtered_tree_lines = vec![];
        app.cursor = 0;

        move_cursor(&mut app, 1);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_set_root_to_selected_dir() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("file.txt"), "data").unwrap();
        // Put a snapshot for the new root in the cache
        let sub_snap = Snapshot::new(sub.clone(), vec![file_node("sub", 0)], 0);
        let arena = vec![
            dir_node(
                tmp.path().file_name().unwrap().to_str().unwrap(),
                vec![("sub", 1)],
            ),
            dir_node("sub", vec![("file.txt", 2)]),
            file_node("file.txt", 100),
        ];
        let snap = Snapshot::new(tmp.path().to_path_buf(), arena, 100);
        let scan_snap = Snapshot::new(tmp.path().to_path_buf(), vec![file_node("tmp", 0)], 0);
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = tmp.path().to_path_buf();
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
        app.scan_cache.insert(tmp.path().to_path_buf(), scan_snap);
        app.scan_cache.insert(sub, sub_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.update_tree_lines();
        app.cursor = 1; // on "sub"

        set_root_to_selected(&mut app);

        assert_eq!(app.view_root_path, tmp.path().join("sub"));
    }

    #[test]
    fn test_set_root_to_selected_non_dir() {
        let arena = vec![
            dir_node("test", vec![("file.txt", 1)]),
            file_node("file.txt", 100),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 100);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded.insert(vec!["test".to_string()]);
        app.update_tree_lines();
        app.cursor = 1; // on "file.txt"

        let prev_root = app.view_root_path.clone();
        set_root_to_selected(&mut app);

        // Should not change root since file.txt is not a dir
        assert_eq!(app.view_root_path, prev_root);
    }

    #[test]
    fn test_set_root_to_selected_already_at_root() {
        let arena = vec![dir_node("test", vec![])];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.update_tree_lines();
        app.cursor = 0; // on root

        let prev_root = app.view_root_path.clone();
        set_root_to_selected(&mut app);

        // Should not change root since already at root
        assert_eq!(app.view_root_path, prev_root);
    }

    // ── handle_gg_double_tap ─────────────────────────────────────────────

    #[test]
    fn test_gg_double_tap_first_sets_pending() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.pending_gg = false;
        app.cursor = 5;

        handle_gg_double_tap(&mut app);

        assert!(app.pending_gg);
        assert_eq!(app.cursor, 5);
    }

    #[test]
    fn test_gg_double_tap_second_jumps_to_top() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.pending_gg = true;
        app.cursor = 5;

        handle_gg_double_tap(&mut app);

        assert!(!app.pending_gg);
        assert_eq!(app.cursor, 0);
    }

    // ── handle_help_key ──────────────────────────────────────────────────

    #[test]
    fn test_help_key_esc_returns_to_browsing() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Help;

        handle_help_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);
        assert_eq!(app.mode, AppMode::Browsing);

        app.mode = AppMode::Help;
        handle_help_key(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.mode, AppMode::Browsing);
    }

    #[test]
    fn test_help_key_other_keys_ignored() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Help;

        handle_help_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.mode, AppMode::Help);
    }

    // ── handle_time_help_key ─────────────────────────────────────────────

    #[test]
    fn test_time_help_key_esc_returns_to_browsing() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::TimeHelp;
        app.time_help_scroll = 5;

        handle_time_help_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);
        assert_eq!(app.mode, AppMode::Browsing);
        assert_eq!(app.time_help_scroll, 0);
    }

    #[test]
    fn test_time_help_key_scroll_down() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::TimeHelp;
        app.time_help_scroll = 0;

        handle_time_help_key(
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.time_help_scroll, 1);
    }

    #[test]
    fn test_time_help_key_scroll_up() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::TimeHelp;
        app.time_help_scroll = 5;

        handle_time_help_key(
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.time_help_scroll, 4);
    }

    #[test]
    fn test_time_help_key_scroll_up_stays_non_negative() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::TimeHelp;
        app.time_help_scroll = 0;

        handle_time_help_key(
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::empty()),
            &mut app,
        );
        // saturating_sub, so stays at 0
        assert_eq!(app.time_help_scroll, 0);
    }

    // ── handle_search_keys ───────────────────────────────────────────────

    #[test]
    fn test_search_keys_input_char() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.search_mode = SearchMode::Input;
        // Set up tree_root so recompute_matches doesn't panic
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0);
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));

        let consumed = handle_search_keys(
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(consumed);
        assert_eq!(app.search_word, "f");
    }

    #[test]
    fn test_search_keys_input_backspace() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.search_mode = SearchMode::Input;
        app.search_word = "fo".to_string();
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0);
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));

        let consumed = handle_search_keys(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
            &mut app,
        );

        assert!(consumed);
        assert_eq!(app.search_word, "f");
    }

    #[test]
    fn test_search_keys_input_enter_empty_goes_inactive() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.search_mode = SearchMode::Input;
        app.search_word.clear();
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0);
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));

        let consumed = handle_search_keys(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            &mut app,
        );

        assert!(consumed);
        assert_eq!(app.search_mode, SearchMode::Inactive);
    }

    #[test]
    fn test_search_keys_input_enter_with_word_goes_active() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.search_mode = SearchMode::Input;
        app.search_word = "foo".to_string();
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0);
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));

        let consumed = handle_search_keys(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            &mut app,
        );

        assert!(consumed);
        assert_eq!(app.search_mode, SearchMode::Active);
    }

    #[test]
    fn test_search_keys_input_esc_clears_and_goes_inactive() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.search_mode = SearchMode::Input;
        app.search_word = "foo".to_string();
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0);
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));

        let consumed =
            handle_search_keys(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);

        assert!(consumed);
        assert!(app.search_word.is_empty());
        assert_eq!(app.search_mode, SearchMode::Inactive);
    }

    #[test]
    fn test_search_keys_active_n_jumps_next() {
        let root_arena = vec![
            dir_node("test", vec![("a", 1), ("b", 2)]),
            dir_node("a", vec![("target", 3)]),
            dir_node("b", vec![("target", 4)]),
            file_node("target", 1),
            file_node("target", 1),
        ];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), root_arena, 2);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["test".to_string(), "a".to_string()]);
        app.expanded
            .insert(vec!["test".to_string(), "b".to_string()]);
        app.update_tree_lines();
        app.search_word = "target".to_string();
        app.recompute_matches();
        app.search_mode = SearchMode::Active;
        app.cursor = 0;

        let consumed = handle_search_keys(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(!consumed);
        let selected = app.selected_line().unwrap();
        assert_eq!(selected.node.name(), "target");
    }

    #[test]
    fn test_search_keys_active_esc_returns_inactive() {
        let root_arena = vec![dir_node("test", vec![])];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), root_arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.search_word = "target".to_string();
        app.recompute_matches();
        app.search_mode = SearchMode::Active;

        let consumed =
            handle_search_keys(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);

        assert!(consumed);
        assert!(app.search_word.is_empty());
        assert_eq!(app.search_mode, SearchMode::Inactive);
    }

    #[test]
    fn test_search_keys_inactive_returns_false() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.search_mode = SearchMode::Inactive;

        let consumed = handle_search_keys(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()),
            &mut app,
        );
        assert!(!consumed);
    }

    // ── handle_delete_action ─────────────────────────────────────────────

    #[test]
    fn test_delete_action_root_dir_guard() {
        let arena = vec![dir_node("test", vec![])];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.update_tree_lines();
        app.cursor = 0; // on root "test"

        handle_delete_action(&mut app, false);

        // Should not enter delete prompt — root dir guard
        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.delete_target_path.is_none());
    }

    #[test]
    fn test_delete_action_non_root_dir() {
        let arena = vec![dir_node("test", vec![("sub", 1)]), dir_node("sub", vec![])];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["test".to_string(), "sub".to_string()]);
        app.update_tree_lines();
        app.cursor = 1; // on "sub" (depth 1, not root)

        handle_delete_action(&mut app, false);

        // Should enter delete prompt (non-root dir)
        assert_eq!(app.mode, AppMode::DeletePrompt);
        assert!(app.delete_target_path.is_some());
    }

    #[test]
    fn test_delete_action_permanent_flag() {
        let arena = vec![dir_node("test", vec![("sub", 1)]), dir_node("sub", vec![])];
        let snap = Snapshot::new(PathBuf::from("/tmp/test"), arena, 0);
        let scan_snap = Snapshot::new(PathBuf::from("/tmp/test"), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.sort_mode = crate::app::SortMode::Name;
        app.expanded
            .insert(vec!["test".to_string(), "sub".to_string()]);
        app.update_tree_lines();
        app.cursor = 1;

        handle_delete_action(&mut app, true);

        assert_eq!(app.mode, AppMode::DeletePermanentPrompt);
    }

    // ── adjust_filter_focus ──────────────────────────────────────────────

    #[test]
    fn test_adjust_filter_focus_time_preset_forward() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        // Use non-existent path so request_delta_refresh returns early
        app.view_root_path = PathBuf::from("/nonexistent_path_xyz");
        app.focus = Focus::FilterPane;
        app.filter_focus = FilterFocus::TimePreset;
        app.time_preset = 0;

        adjust_filter_focus(&mut app, true);

        assert_eq!(app.time_preset, 1);
    }

    #[test]
    fn test_adjust_filter_focus_time_preset_backward() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = PathBuf::from("/nonexistent_path_xyz");
        app.focus = Focus::FilterPane;
        app.filter_focus = FilterFocus::TimePreset;
        app.time_preset = 1;

        adjust_filter_focus(&mut app, false);

        assert_eq!(app.time_preset, 0);
    }

    #[test]
    fn test_adjust_filter_focus_delta_value_forward() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;
        app.filter_focus = FilterFocus::DeltaValue;
        app.delta_filter_active = false;
        app.delta_filter_value = 0;

        adjust_filter_focus(&mut app, true);

        assert!(app.delta_filter_active);
    }

    #[test]
    fn test_adjust_filter_focus_delta_unit_forward() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.focus = Focus::FilterPane;
        app.filter_focus = FilterFocus::DeltaUnit;
        app.delta_filter_active = false;
        app.delta_filter_unit = 0;

        adjust_filter_focus(&mut app, true);

        assert!(app.delta_filter_active);
    }

    // ── prev_match_index ─────────────────────────────────────────────────

    // ── execute_command ──────────────────────────────────────────────────
    #[test]
    fn test_execute_command_empty() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.command_input = "  ".to_string();

        execute_command(&mut app, "  ");

        assert!(app.command_input.is_empty());
    }

    #[test]
    fn test_execute_command_unknown_command() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        // Set up a tree_root so execute_command doesn't error on missing root
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0);
        app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));

        execute_command(&mut app, "xyzzy");

        // Unknown command should produce an error
        assert!(app.last_error.is_some());
    }

    // ── handle_command_key ───────────────────────────────────────────────

    #[test]
    fn test_command_key_char_input() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;

        handle_command_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.command_input, "s");
    }

    #[test]
    fn test_command_key_backspace() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;
        app.command_input = "sc".to_string();

        handle_command_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.command_input, "s");
    }

    #[test]
    fn test_command_key_esc_clears_and_exits() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;
        app.command_input = "scan".to_string();
        app.command_matches = vec!["Scan"];
        app.command_selected = 0;

        handle_command_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);

        assert!(app.command_input.is_empty());
        assert!(app.command_matches.is_empty());
        assert_eq!(app.mode, AppMode::Browsing);
    }

    #[test]
    fn test_command_key_char_limited_to_200() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;
        app.command_input = "x".repeat(200);

        // Try to add another char — should be ignored
        handle_command_key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.command_input.len(), 200);
    }

    // ── handle_browsing_key dispatch ─────────────────────────────────────

    #[test]
    fn test_browsing_key_quit() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;

        handle_browsing_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(app.should_quit);
    }

    #[test]
    fn test_browsing_key_ctrl_c() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;

        handle_browsing_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &mut app,
        );

        assert!(app.should_quit);
    }

    #[test]
    fn test_browsing_key_enter_command_mode() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;

        handle_browsing_key(
            KeyEvent::new(KeyCode::Char(':'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Command);
    }

    #[test]
    fn test_browsing_key_help() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;

        handle_browsing_key(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Help);
    }

    #[test]
    fn test_browsing_key_cancel_scan() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;
        app.scanning = true;

        handle_browsing_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);

        assert!(app.cancel_scan.load(Ordering::Relaxed));
    }

    #[test]
    fn test_browsing_key_gg_double_tap_pending_cleared_on_other_key() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;
        app.pending_gg = true;

        handle_browsing_key(
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(!app.pending_gg);
    }

    #[test]
    fn test_delete_common_yes_runs_success_flow() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("delete_test.txt");
        fs::write(&file_path, "content").unwrap();
        assert!(file_path.exists());

        let root_path = PathBuf::from("/tmp/test");
        let arena = vec![dir_node("test", vec![])];
        let snap = Snapshot::new(root_path.clone(), arena, 0);
        let scan_snap = Snapshot::new(root_path.clone(), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.mode = AppMode::DeletePrompt;
        app.delete_target_path = Some(file_path.clone());

        handle_delete_common(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()),
            &mut app,
            |path| {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
                Ok(format!("deleted: {}", path.display()))
            },
        );

        assert!(!file_path.exists());
        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.delete_target_path.is_none());
    }
}
