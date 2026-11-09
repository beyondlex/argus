mod ai_review;
pub(crate) mod brew;
mod browsing;
pub(crate) mod cleanup;
mod command;
mod delta_detail;
mod finder;
mod info;
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
        AppMode::AiReview => ai_review::handle_ai_review_key(key, app),
        AppMode::Info => info::handle_info_key(key, app),
        AppMode::DeltaDetail => delta_detail::handle_delta_detail_key(key, app),
        AppMode::Cleanup => cleanup::handle_cleanup_key(key, app),
        AppMode::Uninstall => cleanup::handle_uninstall_key(key, app),
        AppMode::Brew => brew::handle_brew_key(key, app),
        AppMode::QuitConfirm => prompt::handle_quit_confirm_key(key, app),
        AppMode::MultiSelectExitConfirm => prompt::handle_multi_select_exit_confirm_key(key, app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TreeNode;
    use crate::app::{AppMessage, AppMode};
    use crate::handler::browsing::{handle_browsing_key, handle_gg_double_tap, move_cursor};
    use crate::handler::command::{execute_command, handle_command_key};
    use crate::handler::prompt::{
        handle_delete_common, handle_delete_permanent_prompt_key, handle_delete_prompt_key,
        handle_help_key, handle_multi_select_exit_confirm_key, handle_quit_confirm_key,
        handle_time_help_key,
    };
    use crate::handler::search::handle_search_keys;
    use crate::types::SearchMode;
    use argus_core::{Snapshot, SnapshotBuilder, ROOT_NODE};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn empty_root_snap(name: &str, path: PathBuf) -> Snapshot {
        SnapshotBuilder::new(name).finish(path, 0, 0)
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
        let snap = empty_root_snap("test", root_path.clone());
        let scan_snap = empty_root_snap("test", root_path.clone());
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

    /// A permanent single-directory delete must report per-item progress.
    /// It used to take the whole-directory fast path, which emitted only an
    /// initial (0,1) and left the gauge frozen at 0% until the entire
    /// recursive removal returned.
    #[test]
    fn test_delete_permanent_dir_reports_item_progress() {
        let (tx, mut rx) = mpsc::channel(64);
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("big");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), "x").unwrap();
        fs::write(dir.join("sub").join("b.txt"), "y").unwrap();

        let errors = super::prompt::delete_dir_worker(&dir, true, &tx, 7);

        // The worker must emit one progress tick per item (root, a.txt,
        // sub/, sub/b.txt) so the gauge advances instead of sitting at 0%.
        // Ticks must carry the task id so the pane can route them.
        let mut progress_ticks = 0;
        while let Ok(msg) = rx.try_recv() {
            if let AppMessage::DeleteProgress { task_id, .. } = msg {
                assert_eq!(task_id, 7);
                progress_ticks += 1;
            }
        }

        assert!(errors.is_empty(), "expected clean delete, got: {errors:?}");
        assert!(!dir.exists(), "directory must be fully removed");
        assert!(
            progress_ticks >= 3,
            "permanent dir delete must report per-item progress, got {progress_ticks} ticks"
        );
    }

    /// Only a whole-directory trash task animates the shared spinner: it moves
    /// the subtree in one atomic call with no intermediate progress. Tasks
    /// with real per-item progress must not trigger a full-UI repaint 10x/sec.
    #[test]
    fn test_delete_spinner_active_only_for_whole_dir_trash() {
        use crate::types::DeleteOrigin;
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);

        app.add_delete_task("dir".into(), false, true, DeleteOrigin::Browse, vec![]);
        assert!(
            app.delete_spinner_active(),
            "whole-dir trash must animate spinner"
        );

        // A task with real per-item progress does not need the spinner.
        app.delete_tasks[0].spinner = false;
        assert!(
            !app.delete_spinner_active(),
            "per-item progress must not repaint on every spinner tick"
        );

        // A finished spinner task no longer animates.
        app.delete_tasks[0].spinner = true;
        app.delete_tasks[0].state = crate::types::DeleteTaskState::Finished { failed: 0 };
        assert!(!app.delete_spinner_active(), "finished task must be inert");
    }

    /// Two separate directory deletes must become two independent registry
    /// entries, and progress messages must be routed to the right one by id.
    #[test]
    fn test_parallel_delete_tasks_route_progress_by_id() {
        let (tx, rx) = mpsc::channel(8);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);

        let a = app.add_delete_task(
            "a".into(),
            true,
            false,
            crate::types::DeleteOrigin::Browse,
            vec![],
        );
        let b = app.add_delete_task(
            "b".into(),
            true,
            false,
            crate::types::DeleteOrigin::Browse,
            vec![],
        );
        assert_ne!(a, b);
        assert_eq!(app.delete_tasks.len(), 2);
        assert_eq!(
            app.mode,
            AppMode::Browsing,
            "delete must not block browsing"
        );

        app.handle_message(AppMessage::DeleteProgress {
            task_id: b,
            current: 5,
            total: 10,
        });
        match &app.delete_tasks[1].state {
            crate::types::DeleteTaskState::Running { current, total } => {
                assert_eq!((*current, *total), (5, 10));
            }
            _ => panic!("task b must be running"),
        }
        // Task a is untouched by b's progress.
        match &app.delete_tasks[0].state {
            crate::types::DeleteTaskState::Running { current, total } => {
                assert_eq!((*current, *total), (0, 1));
            }
            _ => panic!("task a must still be at its initial state"),
        }
    }

    /// `T` clears only finished rows; running tasks stay in the pane.
    #[test]
    fn test_clear_finished_delete_tasks_keeps_running() {
        let (tx, rx) = mpsc::channel(8);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);

        let a = app.add_delete_task(
            "a".into(),
            true,
            false,
            crate::types::DeleteOrigin::Browse,
            vec![],
        );
        app.add_delete_task(
            "b".into(),
            true,
            false,
            crate::types::DeleteOrigin::Browse,
            vec![],
        );
        app.handle_message(AppMessage::DeleteComplete {
            task_id: a,
            errors: Vec::new(),
            paths: Vec::new(),
        });
        app.clear_finished_delete_tasks();

        assert_eq!(app.delete_tasks.len(), 1, "only the running task remains");
        assert!(app.delete_tasks[0].is_running());
    }

    /// End-to-end: two different directories deleted concurrently, each with
    /// its own task id. Both workers run to completion and their rows finish
    /// without either blocking the other's registration.
    #[test]
    fn test_two_dir_deletes_run_concurrently() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("dir-a");
        let b = tmp.path().join("dir-b");
        for dir in [&a, &b] {
            fs::create_dir_all(dir.join("nested")).unwrap();
            fs::write(dir.join("f.bin"), "x").unwrap();
            fs::write(dir.join("nested").join("g.bin"), "y").unwrap();
        }

        let (tx, rx) = mpsc::channel(64);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        let id_a = app.add_delete_task(
            "dir-a".into(),
            true,
            false,
            crate::types::DeleteOrigin::Browse,
            vec![],
        );
        let id_b = app.add_delete_task(
            "dir-b".into(),
            true,
            false,
            crate::types::DeleteOrigin::Browse,
            vec![],
        );

        // Both workers are spawned from the same handler pattern as production.
        let (tx_a, tx_b) = (app.tx.clone(), app.tx.clone());
        rt.block_on(async {
            let (pa, pb) = (a.clone(), b.clone());
            let h1 = tokio::task::spawn_blocking(move || {
                super::prompt::delete_dir_worker(&pa, true, &tx_a, id_a)
            });
            let h2 = tokio::task::spawn_blocking(move || {
                super::prompt::delete_dir_worker(&pb, true, &tx_b, id_b)
            });
            let (ra, rb) = tokio::join!(h1, h2);
            assert!(ra.unwrap().is_empty());
            assert!(rb.unwrap().is_empty());
        });

        // Drain progress + synthesize completion for both, then verify rows.
        while app.rx.try_recv().is_ok() {}
        app.handle_message(AppMessage::DeleteComplete {
            task_id: id_a,
            errors: Vec::new(),
            paths: vec![a.clone()],
        });
        app.handle_message(AppMessage::DeleteComplete {
            task_id: id_b,
            errors: Vec::new(),
            paths: vec![b.clone()],
        });

        assert!(!a.exists() && !b.exists(), "both trees must be gone");
        assert_eq!(app.delete_tasks.len(), 2);
        assert!(app.delete_tasks.iter().all(|t| !t.is_running()));
    }

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

    // ── handle_quit_confirm_key ──────────────────────────────────────────

    #[test]
    fn test_quit_confirm_y_sets_should_quit() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::QuitConfirm;

        handle_quit_confirm_key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(app.should_quit);
    }

    #[test]
    fn test_quit_confirm_n_returns_to_browsing() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::QuitConfirm;

        handle_quit_confirm_key(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Browsing);
        assert!(!app.should_quit);
    }

    #[test]
    fn test_quit_confirm_esc_returns_to_browsing() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::QuitConfirm;

        handle_quit_confirm_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);

        assert_eq!(app.mode, AppMode::Browsing);
        assert!(!app.should_quit);
    }

    // ── handle_multi_select_exit_confirm_key ───────────────────────────────

    #[test]
    fn test_multi_select_exit_confirm_y_exits_multi_select() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::MultiSelectExitConfirm;
        app.multi_select = true;
        app.selected_paths
            .insert(vec!["test".to_string(), "file.txt".to_string()]);

        handle_multi_select_exit_confirm_key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Browsing);
        assert!(!app.multi_select);
        assert!(app.selected_paths.is_empty());
    }

    #[test]
    fn test_multi_select_exit_confirm_n_returns_to_browsing() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::MultiSelectExitConfirm;
        app.multi_select = true;
        app.selected_paths
            .insert(vec!["test".to_string(), "file.txt".to_string()]);

        handle_multi_select_exit_confirm_key(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.multi_select);
        assert!(!app.selected_paths.is_empty());
    }

    #[test]
    fn test_multi_select_exit_confirm_esc_returns_to_browsing() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::MultiSelectExitConfirm;
        app.multi_select = true;

        handle_multi_select_exit_confirm_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.multi_select);
    }

    // ── handle_search_keys ───────────────────────────────────────────────

    #[test]
    fn test_search_keys_input_char() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.search_mode = SearchMode::Input;
        // Set up tree_root so recompute_matches doesn't panic
        let snap = empty_root_snap("tmp", PathBuf::from("/tmp"));
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
        let snap = empty_root_snap("tmp", PathBuf::from("/tmp"));
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
        let snap = empty_root_snap("tmp", PathBuf::from("/tmp"));
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
        let snap = empty_root_snap("tmp", PathBuf::from("/tmp"));
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

    /// While a search is active, `N` (previous match) must be consumed by the
    /// search handler: falling through to browsing opened the *permanent
    /// delete* prompt, because `N` is that panel's shortcut there.
    #[test]
    fn test_search_keys_active_n_does_not_open_delete_prompt() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;
        app.search_mode = SearchMode::Active;
        app.search_word = "log".to_string();

        let consumed = handle_search_keys(
            KeyEvent::new(KeyCode::Char('N'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(consumed);
        assert_eq!(
            app.mode,
            AppMode::Browsing,
            "N must stay a match-cycle key, not a delete prompt"
        );
    }

    /// The list header advertises "Enter edit"; Enter must return to input
    /// mode with the word preserved instead of entering the directory.
    #[test]
    fn test_search_keys_active_enter_returns_to_input() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.search_mode = SearchMode::Active;
        app.search_word = "log".to_string();

        let consumed = handle_search_keys(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            &mut app,
        );

        assert!(consumed);
        assert_eq!(app.search_mode, SearchMode::Input);
        assert_eq!(app.search_word, "log");
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
        let snap = empty_root_snap("tmp", PathBuf::from("/tmp"));
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

    // ── resolve_enter_command ────────────────────────────────────────────

    /// `sn` is a fuzzy subsequence of `Scan`, but Enter must run the typed
    /// alias (sort-by-name), not the highlighted completion.
    #[test]
    fn test_resolve_enter_alias_not_replaced_by_completion() {
        let resolved = super::command::resolve_enter_command("sn", &["Scan"], 0);
        assert_eq!(resolved.as_deref(), Some("sn"));
    }

    #[test]
    fn test_resolve_enter_prefix_expands_to_completion() {
        let resolved = super::command::resolve_enter_command("cle", &["Clean"], 0);
        assert_eq!(resolved.as_deref(), Some("Clean"));
    }

    #[test]
    fn test_resolve_enter_case_insensitive_prefix() {
        let resolved = super::command::resolve_enter_command("con", &["Connect", "Consolidate"], 1);
        assert_eq!(resolved.as_deref(), Some("Consolidate"));
    }

    #[test]
    fn test_resolve_enter_empty_input_closes() {
        assert_eq!(
            super::command::resolve_enter_command("", &["Brew", "Clean"], 0),
            None
        );
    }

    #[test]
    fn test_resolve_enter_typed_args_survive() {
        // Matches would be empty for "Time 3d" in practice, but even with a
        // stale match list the typed text must win.
        let resolved = super::command::resolve_enter_command("Time 3d", &["Time"], 0);
        assert_eq!(resolved.as_deref(), Some("Time 3d"));
    }

    #[test]
    fn test_command_enter_runs_typed_alias() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;
        app.command_input = "sn".to_string();
        app.command_matches = vec!["Scan"];
        app.command_selected = 0;

        handle_command_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Browsing);
        assert_eq!(app.sort_mode, crate::types::SortMode::Name);
    }

    #[test]
    fn test_command_enter_empty_input_closes_bar() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;
        app.command_input.clear();
        app.command_matches = App::COMMANDS.to_vec();
        app.command_selected = 0;

        handle_command_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Browsing);
        // No command must have fired.
        assert!(app.last_error.is_none());
        assert!(app.brew_state.is_none());
        assert!(!app.should_quit);
    }

    // ── command bar history vs completion navigation ─────────────────────

    fn command_app_with_history() -> App {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;
        app.push_command_history("Time 3d");
        app.push_command_history("Scan");
        app
    }

    /// Up recalls history newest-first regardless of the completion list
    /// (which used to capture Up/Down for almost any input, review #29).
    #[test]
    fn test_command_up_recalls_history_newest_first() {
        let mut app = command_app_with_history();

        handle_command_key(KeyEvent::new(KeyCode::Up, KeyModifiers::empty()), &mut app);
        assert_eq!(app.command_input, "Scan");
        assert_eq!(app.command_history_idx, Some(1));

        handle_command_key(KeyEvent::new(KeyCode::Up, KeyModifiers::empty()), &mut app);
        assert_eq!(app.command_input, "Time 3d");
        assert_eq!(app.command_history_idx, Some(0));

        // Oldest entry: Up again stays put.
        handle_command_key(KeyEvent::new(KeyCode::Up, KeyModifiers::empty()), &mut app);
        assert_eq!(app.command_input, "Time 3d");
        assert_eq!(app.command_history_idx, Some(0));
    }

    #[test]
    fn test_command_down_walks_back_and_clears() {
        let mut app = command_app_with_history();
        handle_command_key(KeyEvent::new(KeyCode::Up, KeyModifiers::empty()), &mut app);
        handle_command_key(KeyEvent::new(KeyCode::Up, KeyModifiers::empty()), &mut app);
        assert_eq!(app.command_input, "Time 3d");

        handle_command_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.command_input, "Scan");

        // Past the newest entry: back to the empty draft.
        handle_command_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::empty()),
            &mut app,
        );
        assert!(app.command_input.is_empty());
        assert_eq!(app.command_history_idx, None);
    }

    /// With no history (Up/Down are no-ops) and matches shown, Up/Down must
    /// not move the completion selection — that is Tab/BackTab's job now.
    #[test]
    fn test_command_up_down_do_not_move_completion_selection() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;
        app.command_input = "s".to_string();
        app.update_command_matches();
        assert!(!app.command_matches.is_empty());
        app.command_selected = 1;

        handle_command_key(KeyEvent::new(KeyCode::Up, KeyModifiers::empty()), &mut app);
        handle_command_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.command_selected, 1);
        assert!(app.command_history.is_empty());
    }

    /// Completion cycling moved to Tab/BackTab.
    #[test]
    fn test_command_tab_and_backtab_cycle_matches() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;
        app.command_input = "s".to_string();
        app.update_command_matches();
        assert!(!app.command_matches.is_empty());
        let first = app.command_selected;

        handle_command_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut app);
        let second = app.command_selected;
        assert_ne!(first, second);
        assert_eq!(app.command_input, app.command_matches[second]);

        handle_command_key(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.command_selected, first);
    }

    /// j/k are literal characters in the command bar now (review #27) —
    /// args like `:Time 3d` must remain typeable.
    #[test]
    fn test_command_jk_type_literally() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Command;
        app.command_input = "Time 3".to_string();
        app.command_selected = 2;

        handle_command_key(
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::empty()),
            &mut app,
        );
        handle_command_key(
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.command_input, "Time 3jk");
    }

    // ── handle_browsing_key dispatch ─────────────────────────────────────

    #[test]
    fn test_browsing_key_quit_goes_to_confirm() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;

        handle_browsing_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::QuitConfirm);
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

    /// Confirming a batch delete must consume the selection immediately: the
    /// task now runs in the background while browsing stays live, so keeping
    /// multi-select armed would leave stale highlights (and completion used to
    /// clear a *new* selection made during the delete).
    #[test]
    fn test_confirm_batch_delete_clears_multi_select_and_registers_task() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("batch.bin");
        fs::write(&file, "x").unwrap();

        let (tx, rx) = mpsc::channel(8);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::DeletePrompt;
        app.multi_select = true;
        app.selected_paths
            .insert(vec!["test".into(), "batch.bin".into()]);
        app.delete_target_paths = vec![file];

        // spawn_blocking uses blocking_send, which requires a Tokio runtime.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            handle_delete_common(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()),
                &mut app,
                |p| {
                    std::fs::remove_file(p).map_err(|e| e.to_string())?;
                    Ok("deleted".into())
                },
            );
        });

        assert_eq!(
            app.mode,
            AppMode::Browsing,
            "delete must not block browsing"
        );
        assert!(!app.multi_select, "selection consumed into the task");
        assert!(app.selected_paths.is_empty());
        assert_eq!(app.delete_tasks.len(), 1, "one background task registered");
        assert!(app.delete_tasks[0].is_running());
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

    /// Time presets are daemon-only; in standalone mode the key must say so
    /// instead of doing nothing.
    #[test]
    fn test_browsing_key_time_in_standalone_mode_hints() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;
        app.server_mode = false;

        handle_browsing_key(
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(!app.status_is_error);
        let msg = app.last_error.clone().unwrap_or_default();
        assert!(msg.contains("daemon mode"), "got: {msg}");
    }

    /// Reconnecting while already connected must not spawn a second
    /// connection; the keypress gets an informational hint instead.
    #[test]
    fn test_browsing_key_reconnect_when_connected_hints() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Browsing;
        app.server_mode = true;

        handle_browsing_key(
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(!app.status_is_error);
        let msg = app.last_error.clone().unwrap_or_default();
        assert!(msg.contains("already connected"), "got: {msg}");
    }

    #[test]
    fn test_delete_common_yes_runs_success_flow() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("delete_test.txt");
        fs::write(&file_path, "content").unwrap();
        assert!(file_path.exists());

        let root_path = PathBuf::from("/tmp/test");
        let snap = empty_root_snap("test", root_path.clone());
        let scan_snap = empty_root_snap("test", root_path.clone());
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

    /// A child directory that merely shares the view root's name must stay
    /// deletable: the old guard compared names, so browsing `/tmp/test` made
    /// its own `/tmp/test/test` subdirectory permanently undeletable. The
    /// root itself is unreachable by selection (children only), so the guard
    /// compares full paths instead.
    #[test]
    fn test_delete_child_named_like_root_is_allowed() {
        use argus_core::FileType;
        let mut b = SnapshotBuilder::new("test");
        let inner = b.push_dir(ROOT_NODE, "test");
        b.push_file(inner, "note.txt", FileType::File, 10, 10);
        let root_path = PathBuf::from("/tmp/test");
        let snap = b.finish(root_path.clone(), 10, 10);
        // build_current_tree prefers the cached scan, so both views must
        // contain the child.
        let scan_snap = snap.clone();

        let mut app = make_app(snap, scan_snap);
        app.cursor = app
            .current_children
            .iter()
            .position(|e| e.path == vec!["test".to_string(), "test".to_string()])
            .expect("same-named child must be listed");

        super::browsing::handle_browsing_key(
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::DeletePrompt, "child must be deletable");
        assert_eq!(app.delete_target_path, Some(root_path.join("test")));
    }

    /// A child and its ancestor can both be multi-selected (select the
    /// child, navigate up, select the ancestor). Trashing in HashSet order
    /// used to strand the nested path with a guaranteed ENOENT that surfaced
    /// as a failed delete; the outermost selection must win instead.
    #[test]
    fn test_multi_delete_dedupes_nested_selections() {
        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, _rx);
        app.mode = AppMode::Browsing;
        app.view_root_path = PathBuf::from("/tmp/test");
        app.multi_select = true;
        app.selected_paths
            .insert(vec!["test".into(), "dir".into(), "f.txt".into()]);
        app.selected_paths.insert(vec!["test".into(), "dir".into()]);
        app.selected_paths
            .insert(vec!["test".into(), "other.txt".into()]);

        super::browsing::handle_browsing_key(
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::DeletePrompt);
        let mut targets = app.delete_target_paths.clone();
        targets.sort();
        assert_eq!(
            targets,
            vec![
                PathBuf::from("/tmp/test/dir"),
                PathBuf::from("/tmp/test/other.txt"),
            ],
            "nested selection must be dropped in favor of its ancestor"
        );
    }

    /// Pressing `o` (sort toggle) while a search filter is active must keep
    /// the filter: the old reset to the full app list left the header showing
    /// the word while the list silently showed everything again.
    #[test]
    fn test_uninstall_sort_keeps_active_search_filter() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Uninstall;
        let mk = |name: &str| argus_core::AppInfo {
            id: format!("com.{name}"),
            name: name.into(),
            path: PathBuf::from(format!("/Applications/{name}.app")),
            size: 0,
            last_used: None,
            is_from_app_store: false,
        };
        app.uninstall_state = Some(crate::types::UninstallState {
            apps: vec![mk("Alpha"), mk("Beta"), mk("Alpine")],
            filtered: (0..3).collect(),
            ..Default::default()
        });

        // Type "al" in filter mode, then leave filter mode.
        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::empty()),
            &mut app,
        );
        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()),
            &mut app,
        );
        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::empty()),
            &mut app,
        );
        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(
            app.uninstall_state.as_ref().unwrap().filtered.len(),
            2,
            "search 'al' matches Alpha and Alpine"
        );

        // Sort toggle: filter must survive.
        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::empty()),
            &mut app,
        );
        let state = app.uninstall_state.as_ref().unwrap();
        assert_eq!(state.search_word, "al");
        let names: Vec<&str> = state
            .filtered
            .iter()
            .map(|&i| state.apps[i].name.as_str())
            .collect();
        assert_eq!(names, vec!["Alpha", "Alpine"]);

        // Backspace past the last char restores the full list.
        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
            &mut app,
        );
        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.uninstall_state.as_ref().unwrap().filtered.len(), 3);
    }

    // ── panel scanning failure recovery ─────────────────────────────────

    /// A failed panel scan (Error message) must clear the panel's scanning
    /// flag: the completion message that normally clears it never arrives, so
    /// the panel used to sit on its scanning screen forever (only Esc, which
    /// quit the whole panel, still worked).
    #[test]
    fn test_error_clears_panel_scanning_flags() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Uninstall;
        app.uninstall_state = Some(crate::types::UninstallState {
            scanning: true,
            ..Default::default()
        });
        app.cleanup_state = Some(crate::types::CleanupState {
            scanning: true,
            ..Default::default()
        });

        app.handle_message(AppMessage::Error("leftover scan failed".into()));

        assert!(!app.uninstall_state.as_ref().unwrap().scanning);
        assert!(!app.cleanup_state.as_ref().unwrap().scanning);
    }

    /// A failed delta-detail fetch must not strand the user in DeltaDetail
    /// mode with no popup data: the overlay renders nothing there and only
    /// Esc worked. The Error falls back to the browsing list instead.
    #[test]
    fn test_error_returns_from_empty_delta_detail() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::DeltaDetail;
        app.delta_detail = None;

        app.handle_message(AppMessage::Error("failed to fetch delta detail".into()));

        assert_eq!(app.mode, AppMode::Browsing);
    }

    /// A loaded popup must survive unrelated error messages: only the empty
    /// (fetch failed) state bounces back to browsing.
    #[test]
    fn test_error_keeps_loaded_delta_detail() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::DeltaDetail;
        app.delta_detail = Some(crate::types::DeltaDetailState {
            path: PathBuf::from("/tmp/test"),
            entries: Vec::new(),
            scroll: 0,
        });

        app.handle_message(AppMessage::Error("unrelated failure".into()));

        assert_eq!(app.mode, AppMode::DeltaDetail);
        assert!(app.delta_detail.is_some());
    }

    /// Esc during the Confirm-phase leftover scan must return to the app list,
    /// not quit the whole uninstall panel.
    #[test]
    fn test_uninstall_confirm_scan_esc_returns_to_select_app() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Uninstall;
        app.uninstall_state = Some(crate::types::UninstallState {
            scanning: true,
            phase: crate::types::UninstallPhase::Confirm,
            selected_app: Some(0),
            ..Default::default()
        });

        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
            &mut app,
        );

        assert_eq!(app.mode, AppMode::Uninstall, "panel must stay open");
        let state = app.uninstall_state.as_ref().unwrap();
        assert_eq!(state.phase, crate::types::UninstallPhase::SelectApp);
        assert!(!state.scanning);
        assert!(state.selected_app.is_none());

        // SelectApp-phase scans keep the old behavior: Esc leaves the panel.
        // The in-flight scan's state stays addressable (re-entry shows the
        // running scan instead of stacking a second one); a finished panel
        // drops its state.
        app.uninstall_state.as_mut().unwrap().scanning = true;
        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.mode, AppMode::Browsing);
        assert!(
            app.uninstall_state.as_ref().is_some_and(|s| s.scanning),
            "in-flight scan state must survive the panel exit"
        );

        // A finished panel drops its state entirely.
        app.mode = AppMode::Uninstall;
        app.uninstall_state.as_mut().unwrap().scanning = false;
        super::cleanup::handle_uninstall_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty()),
            &mut app,
        );
        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.uninstall_state.is_none());
    }

    /// A leftover scan that completes after the user backed out of Confirm —
    /// or after a *different* app was selected — must not populate the panel.
    #[test]
    fn test_stale_leftovers_for_other_app_ignored() {
        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::Uninstall;
        let app_b = argus_core::AppInfo {
            id: "com.b".into(),
            name: "B".into(),
            path: PathBuf::from("/Applications/B.app"),
            size: 0,
            last_used: None,
            is_from_app_store: false,
        };
        app.uninstall_state = Some(crate::types::UninstallState {
            scanning: true,
            phase: crate::types::UninstallPhase::Confirm,
            selected_app: Some(0),
            apps: vec![app_b.clone()],
            ..Default::default()
        });

        // Leftovers of a different app arrive (the scan the user backed out
        // of, racing a new selection).
        let stale = argus_core::AppLeftovers {
            app: argus_core::AppInfo {
                id: "com.a".into(),
                name: "A".into(),
                path: PathBuf::from("/Applications/A.app"),
                size: 0,
                last_used: None,
                is_from_app_store: false,
            },
            leftover_paths: vec![PathBuf::from("/Users/x/Library/A")],
            total_leftover_bytes: 10,
        };
        app.handle_message(AppMessage::UninstallLeftoversReady(stale));

        let state = app.uninstall_state.as_ref().unwrap();
        assert!(state.scanning, "stale result must not end the wait");
        assert!(state.leftovers.is_none(), "stale leftovers must be dropped");
    }

    // ── ai_review delete confirm ─────────────────────────────────────────

    /// Confirming an AI-review delete must respect the protected-path gate:
    /// browsing-mode deletes refuse these before the prompt, and the review
    /// confirm used to be a side door around it (permanent remove_dir_all
    /// included).
    #[cfg(target_os = "macos")]
    #[test]
    fn test_ai_review_delete_confirm_refuses_protected_paths() {
        use crate::app::{AiPathVerdict, AiReviewState, AiStatus, RiskLevel};
        use std::collections::HashSet;

        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.mode = AppMode::AiReview;
        app.ai_state = Some(AiReviewState {
            results: vec![AiPathVerdict {
                path: PathBuf::from("/etc"),
                size: 4096,
                label: String::new(),
                label_detail: String::new(),
                purpose: String::new(),
                risk_level: RiskLevel::High,
                suggestion: String::new(),
                background: String::new(),
                deletable: false,
                source: crate::app::AI_SOURCE_MODEL.into(),
            }],
            pending_paths: Vec::new(),
            pending_total_size: 0,
            cursor: 0,
            scroll_offset: 0,
            mark_for_delete: HashSet::new(),
            status: AiStatus::Ready,
            delete_confirm: Some((vec![PathBuf::from("/etc")], true)),
            info_item: None,
        });

        super::handle_key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()),
            &mut app,
        );

        // The deletion runs on a background thread: the confirm closes and
        // the panel flips to Deleting until the completion message lands.
        assert!(
            std::path::Path::new("/etc").exists(),
            "protected path must survive"
        );
        let state = app.ai_state.as_ref().unwrap();
        assert!(state.delete_confirm.is_none());
        assert_eq!(state.status, AiStatus::Deleting);

        // The spawned thread's exact completion is covered by the
        // delete_marked unit test; here the message handler half is driven
        // directly (App owns the receiver, so the real message can't be
        // intercepted from the test). The confirm registered one task.
        let task_id = app.delete_tasks[0].id;
        app.handle_message(AppMessage::DeleteComplete {
            task_id,
            errors: vec!["/etc: protected path, skipped".into()],
            paths: Vec::new(),
        });

        assert!(
            std::path::Path::new("/etc").exists(),
            "protected path must survive"
        );
        assert!(
            app.last_error
                .as_deref()
                .unwrap_or_default()
                .contains("protected"),
            "expected a protected-path error, got: {:?}",
            app.last_error
        );
    }
}
