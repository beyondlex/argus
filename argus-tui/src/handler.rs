mod browsing;
mod command;
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
        AppMode::Deleting => {} // ignore all keys during deletion
        AppMode::Help => prompt::handle_help_key(key, app),
        AppMode::TimeHelp => prompt::handle_time_help_key(key, app),
        AppMode::Command => command::handle_command_key(key, app),
        AppMode::Finder => finder::handle_finder_key(key, app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TreeNode;
    use crate::handler::browsing::{
        handle_browsing_key, handle_delete_action, handle_gg_double_tap, move_cursor,
    };
    use crate::handler::command::{execute_command, handle_command_key};
    use crate::handler::prompt::{
        handle_delete_common, handle_delete_permanent_prompt_key, handle_delete_prompt_key,
        handle_help_key, handle_time_help_key,
    };
    use crate::handler::search::handle_search_keys;
    use crate::types::SearchMode;
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
            disk_usage: size,
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
            disk_usage: 0,
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
        app.scan_cache
            .insert(PathBuf::from("/tmp/test"), Arc::new(scan_snap));
        app.current_dir_path = vec!["test".to_string()];
        app.load_current_children();
        app
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
        let snap = Snapshot::new(root_path.clone(), arena, 0, 0);
        let scan_snap = Snapshot::new(root_path.clone(), vec![file_node("test", 0)], 0, 0);
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
        app.current_filtered = vec![0, 1, 2];
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
        app.current_filtered = vec![0, 1, 2];
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
        app.current_filtered = vec![0, 1, 2];
        app.cursor = 0;

        move_cursor(&mut app, -1);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_move_cursor_bounds_bottom() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.current_filtered = vec![0, 1, 2];
        app.cursor = 2;

        move_cursor(&mut app, 1);
        assert_eq!(app.cursor, 2);
    }

    #[test]
    fn test_move_cursor_empty() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.current_filtered = vec![];
        app.cursor = 0;

        move_cursor(&mut app, 1);
        assert_eq!(app.cursor, 0);
    }

    #[test]
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
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0, 0);
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
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0, 0);
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
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0, 0);
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
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0, 0);
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

        let consumed =
            handle_search_keys(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);

        assert!(consumed);
        assert!(app.search_word.is_empty());
        assert_eq!(app.search_mode, SearchMode::Inactive);
    }

    #[test]
    fn test_search_keys_active_esc_returns_inactive() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.search_mode = SearchMode::Active;
        app.search_word = "target".to_string();

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
        let snap = Snapshot::new(PathBuf::from("/tmp"), vec![file_node("tmp", 0)], 0, 0);
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
        let snap = Snapshot::new(root_path.clone(), arena, 0, 0);
        let scan_snap = Snapshot::new(root_path.clone(), vec![file_node("test", 0)], 0, 0);
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
