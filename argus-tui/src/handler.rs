use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::app::{App, AppMessage, AppMode, FilterFocus, Focus, SearchMode};
use crate::ipc_client::IpcClient;

/// Handle keyboard events
pub fn handle_key(key: KeyEvent, app: &mut App) {
    match app.mode {
        AppMode::Browsing => handle_browsing_key(key, app),
        AppMode::DeletePrompt => handle_delete_prompt_key(key, app),
        AppMode::DeletePermanentPrompt => handle_delete_permanent_prompt_key(key, app),
        AppMode::Help => handle_help_key(key, app),
        AppMode::TimeHelp => handle_time_help_key(key, app),
        AppMode::Command => handle_command_key(key, app),
    }
}

fn handle_browsing_key(key: KeyEvent, app: &mut App) {
    if app.scanning {
        if key.code == KeyCode::Esc {
            app.cancel_scan.store(true, Ordering::Relaxed);
        }
        return;
    }

    if app.focus == Focus::FilterPane {
        handle_filter_pane_key(key, app);
        return;
    }

    if handle_search_keys(key, app) {
        return;
    }

    if app.search_mode == SearchMode::Inactive {
        if let KeyCode::Char('/') = key.code {
            app.search_mode = SearchMode::Input;
            return;
        }
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => move_cursor(app, 1),
        KeyCode::Char('k') | KeyCode::Up => move_cursor(app, -1),
        KeyCode::Char('g') => handle_gg_double_tap(app),
        KeyCode::Char('G') => {
            if !app.filtered_tree_lines.is_empty() {
                app.cursor = app.filtered_tree_lines.len() - 1;
            }
            app.pending_gg = false;
        }
        KeyCode::Char('l') | KeyCode::Right => crate::tree_ops::expand_node(app),
        KeyCode::Char('h') | KeyCode::Left => crate::tree_ops::collapse_or_navigate_up(app),
        KeyCode::Char('H') => crate::tree_ops::collapse_all_children(app),
        KeyCode::Char('u') => crate::tree_ops::navigate_up_root(app),
        KeyCode::Enter if !app.search_word.is_empty() => app.search_mode = SearchMode::Input,
        KeyCode::Char('s') => start_scan(app),
        KeyCode::Char('.') => set_root_to_selected(app),
        KeyCode::Char('o') => {
            app.sort_mode = app.sort_mode.toggle();
            app.update_tree_lines();
        }
        KeyCode::Char('d') => handle_delete_action(app, false),
        KeyCode::Char('D') => handle_delete_action(app, true),
        KeyCode::Char('?') => app.mode = AppMode::Help,
        KeyCode::Char(':') => {
            app.mode = AppMode::Command;
            app.clear_command_state();
            app.command_history_idx = None;
            app.update_command_matches();
        }
        KeyCode::Char('t') if app.server_mode => handle_time_toggle(app),
        KeyCode::Char('R') if !app.server_mode => handle_daemon_reconnect(app),
        KeyCode::Char('i') => handle_info_popup(app),
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('y') => handle_copy_path(app),
        KeyCode::Char('f') if app.server_mode => {
            app.focus = Focus::FilterPane;
            app.filter_focus = FilterFocus::TimePreset;
        }
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => app.should_quit = true,
        KeyCode::Char('c') => app.clear_filter_pane(),
        KeyCode::Esc => app.info_data = None,
        _ => {}
    }
    if key.code != KeyCode::Char('g') {
        app.pending_gg = false;
    }
}

/// Handle search-mode input keys. Returns true if the key was consumed.
fn handle_search_keys(key: KeyEvent, app: &mut App) -> bool {
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
            false // let other keys pass through
        }
        SearchMode::Inactive => false,
    }
}

fn handle_delete_action(app: &mut App, permanent: bool) {
    let Some(line) = app.selected_line() else {
        return;
    };
    let root_name = app
        .view_root_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if line.node.is_dir() && line.node.name() == root_name {
        app.set_error("cannot delete root directory".into(), 3);
        return;
    }
    let Some(full_path) = app.selected_node_full_path() else {
        return;
    };
    if crate::util::is_protected_path(&full_path) {
        app.set_error("protected path, cannot delete".into(), 3);
        return;
    }
    app.delete_target_path = Some(full_path);
    app.mode = if permanent {
        AppMode::DeletePermanentPrompt
    } else {
        AppMode::DeletePrompt
    };
}

fn handle_gg_double_tap(app: &mut App) {
    if app.pending_gg {
        app.cursor = 0;
        app.pending_gg = false;
    } else {
        app.pending_gg = true;
    }
}

fn handle_time_toggle(app: &mut App) {
    if app.time_custom {
        app.set_time_preset(0);
        app.set_error(format!("time range: {}", App::time_preset_label(0)), 2);
    } else {
        let next = (app.time_preset + 1) % crate::app::TIME_PRESET_COUNT;
        app.set_time_preset(next);
        app.set_error(format!("time range: {}", App::time_preset_label(next)), 2);
    }
    app.request_delta_refresh();
}

fn handle_daemon_reconnect(app: &mut App) {
    let path = crate::config::TuiConfig::default().daemon.uds_path.clone();
    let path_clone = path.clone();
    let tx = app.tx.clone();
    tokio::spawn(async move {
        if let Ok(mut client) = IpcClient::connect(&path_clone).await {
            if client.ping().await.is_ok() {
                let _ = tx.send(AppMessage::DaemonConnected(client)).await;
                return;
            }
        }
        let _ = tx
            .send(AppMessage::Error("daemon reconnect failed".into()))
            .await;
    });
}

fn handle_info_popup(app: &mut App) {
    let Some(path) = app.selected_node_full_path() else {
        return;
    };
    match std::fs::metadata(&path) {
        Ok(meta) => app.info_data = Some((path, meta)),
        Err(e) => app.set_error(format!("stat failed: {}", e), 3),
    }
}

fn handle_copy_path(app: &mut App) {
    let Some(path) = app.selected_node_full_path() else {
        return;
    };
    match arboard::Clipboard::new() {
        Ok(mut cb) => {
            let path_str = path.display().to_string();
            if cb.set_text(path_str.clone()).is_ok() {
                app.set_error(format!("copied: {}", path_str), 2);
            } else {
                app.set_error("clipboard write failed".into(), 3);
            }
        }
        Err(_) => {
            app.set_error("clipboard unavailable".into(), 3);
        }
    }
}

fn handle_delete_prompt_key(key: KeyEvent, app: &mut App) {
    handle_delete_common(key, app, |path| {
        trash::delete(path).map_err(|e| e.to_string())?;
        Ok(format!("deleted: {}", path.display()))
    });
}

fn handle_delete_permanent_prompt_key(key: KeyEvent, app: &mut App) {
    handle_delete_common(key, app, |path| {
        if path.is_dir() {
            std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        } else {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(format!("permanently deleted: {}", path.display()))
    });
}

fn handle_delete_common<F>(key: KeyEvent, app: &mut App, delete_fn: F)
where
    F: Fn(&Path) -> Result<String, String>,
{
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(path) = app.delete_target_path.clone() {
                match delete_fn(&path) {
                    Ok(msg) => {
                        app.set_error(msg, 3);
                        crate::tree_ops::apply_deletion_to_state(app, &path);
                        app.update_tree_lines();
                    }
                    Err(e) => {
                        app.set_error(format!("delete failed: {}", e), 5);
                    }
                }
            }
            app.delete_target_path = None;
            app.mode = AppMode::Browsing;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.delete_target_path = None;
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

fn handle_help_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('?') | KeyCode::Esc => {
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

fn handle_time_help_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Browsing;
            app.time_help_scroll = 0;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.time_help_scroll = app.time_help_scroll.saturating_add(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.time_help_scroll = app.time_help_scroll.saturating_sub(1);
        }
        _ => {}
    }
}

fn handle_command_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char(c) if app.command_input.len() < 200 => {
            app.command_input.push(c);
            app.update_command_matches();
            app.command_history_idx = None;
        }
        KeyCode::Backspace => {
            app.command_input.pop();
            app.update_command_matches();
            app.command_history_idx = None;
        }
        KeyCode::Tab if !app.command_matches.is_empty() => {
            app.command_selected = (app.command_selected + 1) % app.command_matches.len();
            app.command_input = app.command_matches[app.command_selected].to_string();
            app.command_history_idx = None;
        }
        KeyCode::BackTab if !app.command_matches.is_empty() => {
            app.command_selected = if app.command_selected == 0 {
                app.command_matches.len() - 1
            } else {
                app.command_selected - 1
            };
            app.command_input = app.command_matches[app.command_selected].to_string();
            app.command_history_idx = None;
        }
        KeyCode::Up | KeyCode::Char('k') if !app.command_history.is_empty() => {
            let idx = match app.command_history_idx {
                Some(i) if i > 0 => i - 1,
                None => app.command_history.len() - 1,
                _ => return,
            };
            app.command_history_idx = Some(idx);
            app.command_input = app.command_history[idx].clone();
            app.update_command_matches();
        }
        KeyCode::Down | KeyCode::Char('j') if app.command_history_idx.is_some() => {
            let idx = app.command_history_idx.unwrap();
            if idx + 1 < app.command_history.len() {
                app.command_history_idx = Some(idx + 1);
                app.command_input = app.command_history[idx + 1].clone();
            } else {
                app.command_history_idx = None;
                app.command_input.clear();
            }
            app.update_command_matches();
        }
        KeyCode::Enter => {
            let cmd = if !app.command_matches.is_empty() {
                app.command_matches[app.command_selected].to_string()
            } else {
                app.command_input.clone()
            };
            app.mode = AppMode::Browsing;
            execute_command(app, &cmd);
        }
        KeyCode::Esc => {
            app.clear_command_state();
            app.mode = AppMode::Browsing;
        }
        _ => {}
    }
}

fn execute_command(app: &mut App, cmd: &str) {
    let cmd = cmd.trim();

    if cmd.is_empty() {
        app.clear_command_state();
        return;
    }

    app.push_command_history(cmd);

    if cmd.eq_ignore_ascii_case("Scan") {
        app.clear_command_state();
        start_scan(app);
        return;
    }

    if cmd.eq_ignore_ascii_case("Consolidate") {
        app.clear_command_state();
        if app.server_mode {
            let uds_path = app
                .daemon_client
                .as_ref()
                .map(|_| crate::config::TuiConfig::default().daemon.uds_path.clone())
                .unwrap_or_default();
            let tx = app.tx.clone();
            tokio::spawn(async move {
                match IpcClient::connect(&uds_path).await {
                    Ok(mut client) => match client.request_consolidation().await {
                        Ok(count) => {
                            let _ = tx
                                .send(AppMessage::Info(format!("consolidated {count} events")))
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(AppMessage::Info(format!("consolidation failed: {e}")))
                                .await;
                        }
                    },
                    Err(e) => {
                        let _ = tx
                            .send(AppMessage::Info(format!("daemon connect failed: {e}")))
                            .await;
                    }
                }
            });
        } else {
            app.set_error("not in server mode".into(), 3);
        }
        return;
    }

    match app.execute_command(cmd) {
        Ok(msg) => {
            app.set_error(msg, 3);
        }
        Err(e) => {
            app.set_error(e, 4);
        }
    }
    app.clear_command_state();
}

// ── Filter pane key handling ─────────────────────────────────────────────────

fn handle_filter_pane_key(key: KeyEvent, app: &mut App) {
    let skip_time = app.time_custom;
    match key.code {
        KeyCode::Tab | KeyCode::Char('\t') => {
            app.filter_focus = cycle_filter_focus(app.filter_focus, skip_time, true);
        }
        KeyCode::BackTab => {
            app.filter_focus = cycle_filter_focus(app.filter_focus, skip_time, false);
        }
        KeyCode::Char(ch) if ch.is_ascii_digit() && app.filter_focus == FilterFocus::DeltaValue => {
            let digit = (ch as u8 - b'0') as u64;
            if app.delta_filter_active {
                app.delta_filter_value = app
                    .delta_filter_value
                    .saturating_mul(10)
                    .saturating_add(digit);
            } else {
                app.delta_filter_active = true;
                app.delta_filter_value = digit;
            }
            app.refresh_filtered_lines();
        }
        KeyCode::Backspace
            if app.filter_focus == FilterFocus::DeltaValue && app.delta_filter_active =>
        {
            app.delta_filter_value /= 10;
            if app.delta_filter_value == 0 {
                app.delta_filter_active = false;
            }
            app.refresh_filtered_lines();
        }
        KeyCode::Char('j') | KeyCode::Down => adjust_filter_focus(app, true),
        KeyCode::Char('k') | KeyCode::Up => adjust_filter_focus(app, false),
        KeyCode::Char('c') => {
            app.clear_filter_pane();
        }
        KeyCode::Enter => {
            if app.filter_focus == FilterFocus::DeltaValue && app.delta_filter_active {
                app.refresh_filtered_lines();
            }
            app.focus = Focus::Tree;
        }
        KeyCode::Esc => {
            app.focus = Focus::Tree;
        }
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Char(':') => {
            app.mode = AppMode::Command;
            app.clear_command_state();
            app.command_history_idx = None;
            app.update_command_matches();
        }
        _ => {}
    }
}

fn cycle_filter_focus(current: FilterFocus, skip_time: bool, forward: bool) -> FilterFocus {
    if skip_time {
        match current {
            FilterFocus::DeltaValue => FilterFocus::DeltaUnit,
            _ => FilterFocus::DeltaValue,
        }
    } else {
        let order = [
            FilterFocus::TimePreset,
            FilterFocus::DeltaValue,
            FilterFocus::DeltaUnit,
        ];
        let pos = order.iter().position(|f| *f == current).unwrap_or(0);
        let next = if forward {
            (pos + 1) % 3
        } else {
            (pos + 2) % 3
        };
        order[next]
    }
}

fn adjust_filter_focus(app: &mut App, forward: bool) {
    let skip_time = app.time_custom;
    match app.filter_focus {
        FilterFocus::TimePreset if !skip_time => {
            let count = crate::app::TIME_PRESET_COUNT;
            let next = if forward {
                (app.time_preset + 1) % count
            } else {
                (app.time_preset + count - 1) % count
            };
            let label = App::time_preset_label(next);
            crate::app::log_msg(
                &app.log_path,
                &format!(
                    "filter: {} key, preset={next} ({label})",
                    if forward { "j" } else { "k" }
                ),
            );
            app.set_time_preset(next);
            app.request_delta_refresh();
        }
        FilterFocus::TimePreset => {}
        FilterFocus::DeltaValue => {
            app.delta_filter_active = true;
            if forward {
                app.delta_filter_inc();
            } else {
                app.delta_filter_dec();
            }
            app.refresh_filtered_lines();
        }
        FilterFocus::DeltaUnit => {
            app.delta_filter_active = true;
            if forward {
                app.delta_filter_cycle_unit();
            } else {
                app.delta_filter_unit = (app.delta_filter_unit + 2) % 3;
            }
            app.refresh_filtered_lines();
        }
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

fn move_cursor(app: &mut App, delta: isize) {
    if app.filtered_tree_lines.is_empty() {
        return;
    }
    let new_cursor = app.cursor as isize + delta;
    if new_cursor < 0 {
        app.cursor = 0;
    } else if new_cursor >= app.filtered_tree_lines.len() as isize {
        app.cursor = app.filtered_tree_lines.len() - 1;
    } else {
        app.cursor = new_cursor as usize;
    }
}

pub fn set_root_to_selected(app: &mut App) {
    let Some(line) = app.selected_line().cloned() else {
        return;
    };
    if !line.node.is_dir() {
        return;
    }
    if let Some(full_path) = app.selected_node_full_path() {
        if full_path == app.view_root_path {
            return;
        }
        app.view_root_path = full_path;
        app.rebuild_tree();
    }
}

pub fn start_scan(app: &mut App) {
    app.scanning = true;
    app.scan_progress = None;
    app.scan_spinner = 0;
    app.scan_spinner_tick = Instant::now();
    app.scan_started_at = Some(Instant::now());
    app.cancel_scan = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel = app.cancel_scan.clone();
    let tx = app.tx.clone();
    let path = app.view_root_path.clone();
    let scan_skip_dirs: Vec<String> = app.config.browsing.skip_dirs.clone();

    tokio::task::spawn_blocking(move || {
        let (progress_tx, progress_rx) =
            std::sync::mpsc::channel::<argus_core::scanner::ProgressUpdate>();

        // Forward progress updates from blocking scan to UI via dedicated thread
        let tx_clone = tx.clone();
        std::thread::spawn(move || {
            while let Ok(update) = progress_rx.recv() {
                let _ = tx_clone.blocking_send(AppMessage::ScanProgress {
                    file_count: update.file_count,
                    total_bytes: update.total_bytes,
                });
            }
        });

        match argus_core::scan_path(&path, &cancel, Some(progress_tx), &scan_skip_dirs) {
            Ok(snapshot) => {
                let _ = tx.blocking_send(AppMessage::ScanComplete(snapshot));
            }
            Err(e) => {
                let _ = tx.blocking_send(AppMessage::Error(format!("scan failed: {}", e)));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{TreeLine, TreeNode};
    use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};
    use std::fs;
    use std::path::PathBuf;
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

    #[test]
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
    fn test_delete_prompt_yes_with_trash() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("trash_test.txt");
        fs::write(&file_path, "content").unwrap();
        assert!(file_path.exists());

        let root_path = PathBuf::from("/tmp/test");
        let arena = vec![dir_node("test", vec![])];
        let snap = Snapshot::new(root_path.clone(), arena, 0);
        let scan_snap = Snapshot::new(root_path.clone(), vec![file_node("test", 0)], 0);
        let mut app = make_app(snap, scan_snap);
        app.mode = AppMode::DeletePrompt;
        app.delete_target_path = Some(file_path.clone());

        handle_delete_prompt_key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()),
            &mut app,
        );

        assert!(!file_path.exists());
        assert_eq!(app.mode, AppMode::Browsing);
        assert!(app.delete_target_path.is_none());
    }
}
