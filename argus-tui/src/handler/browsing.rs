use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::app::{App, AppMessage, AppMode, FilterFocus, Focus, SearchMode};
use crate::ipc_client::IpcClient;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn handle_browsing_key(key: KeyEvent, app: &mut App) {
    // If delta detail popup is open, intercept j/k for scroll and Esc to dismiss
    if app.delta_detail.is_some() {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(ref mut state) = app.delta_detail {
                    if delta_detail_needs_scroll(state) && state.scroll + 1 < state.entries.len() {
                        state.scroll += 1;
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(ref mut state) = app.delta_detail {
                    if delta_detail_needs_scroll(state) {
                        state.scroll = state.scroll.saturating_sub(1);
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                app.delta_detail = None;
            }
            _ => {}
        }
        return;
    }

    if app.scanning {
        if key.code == KeyCode::Esc {
            app.cancel_scan.store(true, Ordering::Relaxed);
        }
        return;
    }

    if app.focus == Focus::FilterPane {
        // Exit multi-select when focusing filter pane
        if app.multi_select {
            app.exit_multi_select();
        }
        crate::handler::filter::handle_filter_pane_key(key, app);
        return;
    }

    if crate::handler::search::handle_search_keys(key, app) {
        return;
    }

    if app.search_mode == SearchMode::Inactive && matches!(key.code, KeyCode::Char('/')) {
        app.search_mode = SearchMode::Input;
        return;
    }

    // Handle multi-select mode Tab / Esc
    if app.multi_select {
        match key.code {
            KeyCode::Tab => {
                app.toggle_selection();
                move_cursor(app, 1);
                app.pending_gg = false;
                return;
            }
            KeyCode::Esc => {
                app.exit_multi_select();
                app.pending_gg = false;
                return;
            }
            KeyCode::Char('d') => {
                handle_multi_delete_action(app, false);
                app.pending_gg = false;
                return;
            }
            KeyCode::Char('D') => {
                handle_multi_delete_action(app, true);
                app.pending_gg = false;
                return;
            }
            _ => {}
        }
    } else {
        // Enter multi-select mode on Tab
        if key.code == KeyCode::Tab {
            if app.filtered_tree_lines.is_empty() {
                app.pending_gg = false;
                return;
            }
            app.enter_multi_select();
            app.toggle_selection();
            move_cursor(app, 1);
            app.pending_gg = false;
            return;
        }
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => move_cursor(app, 1),
        KeyCode::Char('k') | KeyCode::Up => move_cursor(app, -1),
        KeyCode::Char('g') => handle_gg_double_tap(app),
        KeyCode::Char('G') => {
            let len = if !app.current_children.is_empty() {
                app.current_filtered.len()
            } else {
                app.filtered_tree_lines.len()
            };
            if len > 0 {
                app.cursor = len - 1;
            }
            app.pending_gg = false;
        }
        KeyCode::Char('l') | KeyCode::Right => {
            if !app.current_children.is_empty() {
                app.enter_directory();
            } else {
                crate::tree_ops::expand_node(app);
            }
        }
        KeyCode::Char('h') | KeyCode::Left => {
            if !app.current_children.is_empty() {
                app.go_to_parent();
            } else {
                crate::tree_ops::collapse_or_navigate_up(app);
            }
        }
        KeyCode::Char('H') => {
            if !app.current_children.is_empty() {
                app.go_to_root();
            } else {
                crate::tree_ops::collapse_all_children(app);
            }
        }
        KeyCode::Char('u') => {
            if !app.current_children.is_empty() {
                app.go_to_root();
            } else {
                crate::tree_ops::navigate_up_root(app);
            }
        }
        KeyCode::Enter if !app.search_word.is_empty() => app.search_mode = SearchMode::Input,
        KeyCode::Char('s') => {
            if app.multi_select {
                app.exit_multi_select();
            }
            start_scan(app);
        }
        KeyCode::Char('.') => {
            app.show_hidden = !app.show_hidden;
            app.set_error(
                if app.show_hidden {
                    "hidden files shown".into()
                } else {
                    "hidden files hidden".into()
                },
                2,
            );
            app.update_tree_lines();
        }
        KeyCode::Char('w') => {
            if app.current_children.is_empty() {
                set_root_to_selected(app);
            }
        }
        KeyCode::Char('o') => {
            if app.multi_select {
                app.exit_multi_select();
            }
            app.sort_mode = app.sort_mode.toggle();
            if !app.current_children.is_empty() {
                crate::handler::browsing::sort_children_flat(app);
            } else {
                app.update_tree_lines();
            }
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
        KeyCode::Char('t') if app.server_mode => {
            if app.multi_select {
                app.exit_multi_select();
            }
            handle_time_toggle(app);
        }
        KeyCode::Char('R') if !app.server_mode => handle_daemon_reconnect(app),
        KeyCode::Char('i') => handle_info_popup(app),
        KeyCode::Char('K') => handle_delta_detail_popup(app),
        KeyCode::Char('q') => {
            if app.info_data.is_some() {
                app.info_data = None;
            } else {
                app.should_quit = true;
            }
        }
        KeyCode::Char('y') => handle_copy_path(app),
        KeyCode::Char('f') if app.server_mode => {
            if app.multi_select {
                app.exit_multi_select();
            }
            app.focus = Focus::FilterPane;
            app.filter_focus = FilterFocus::TimePreset;
        }
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => app.should_quit = true,
        KeyCode::Char('c') => {
            if app.multi_select {
                app.exit_multi_select();
            }
            app.clear_filter_pane();
        }
        KeyCode::Esc => {
            app.info_data = None;
            app.delta_detail = None;
        }
        _ => {}
    }
    if key.code != KeyCode::Char('g') {
        app.pending_gg = false;
    }
}

/// Handle delete action in multi-select mode
fn handle_multi_delete_action(app: &mut App, permanent: bool) {
    if app.selected_paths.is_empty() {
        app.set_info("no items selected".into(), 3);
        return;
    }
    let mut paths = app.selected_paths_full();
    // Filter out protected paths
    paths.retain(|p| !crate::util::is_protected_path(p));
    if paths.is_empty() {
        app.set_error("all selected paths are protected".into(), 3);
        return;
    }
    // Filter out root directory
    let root_name = app
        .view_root_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    paths.retain(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_string() != root_name)
            .unwrap_or(true)
    });
    if paths.is_empty() {
        app.set_error("cannot delete root directory".into(), 3);
        return;
    }
    app.delete_target_paths = paths;
    app.mode = if permanent {
        AppMode::DeletePermanentPrompt
    } else {
        AppMode::DeletePrompt
    };
}

pub(crate) fn handle_gg_double_tap(app: &mut App) {
    if app.pending_gg {
        app.cursor = 0;
        app.pending_gg = false;
    } else {
        app.pending_gg = true;
    }
}

pub(crate) fn handle_time_toggle(app: &mut App) {
    if app.time_custom {
        app.set_time_preset(0);
        app.set_info(format!("time range: {}", App::time_preset_label(0)), 2);
    } else {
        let next = (app.time_preset + 1) % crate::app::TIME_PRESET_COUNT;
        app.set_time_preset(next);
        app.set_info(format!("time range: {}", App::time_preset_label(next)), 2);
    }
    app.request_delta_refresh();
}

pub(crate) fn handle_daemon_reconnect(app: &mut App) {
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

pub(crate) fn handle_info_popup(app: &mut App) {
    let Some(path) = app.selected_node_full_path() else {
        return;
    };
    match std::fs::metadata(&path) {
        Ok(meta) => app.info_data = Some((path, meta)),
        Err(e) => app.set_error(format!("stat failed: {}", e), 3),
    }
}

pub(crate) fn handle_delta_detail_popup(app: &mut App) {
    let Some(path) = app.selected_node_full_path() else {
        return;
    };
    crate::components::delta_detail::load_delta_detail(app, &path);
}

pub(crate) fn handle_copy_path(app: &mut App) {
    let Some(path) = app.selected_node_full_path() else {
        return;
    };
    match arboard::Clipboard::new() {
        Ok(mut cb) => {
            let path_str = path.display().to_string();
            if cb.set_text(path_str.clone()).is_ok() {
                app.set_info(format!("copied: {}", path_str), 2);
            } else {
                app.set_error("clipboard write failed".into(), 3);
            }
        }
        Err(_) => {
            app.set_error("clipboard unavailable".into(), 3);
        }
    }
}

pub(crate) fn handle_delete_action(app: &mut App, permanent: bool) {
    let root_name = app
        .view_root_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if !app.current_children.is_empty() {
        let Some(entry) = app.selected_entry() else {
            return;
        };
        if entry.is_dir && entry.node.name() == root_name {
            app.set_error("cannot delete root directory".into(), 3);
            return;
        }
    } else {
        let Some(line) = app.selected_line() else {
            return;
        };
        if line.node.is_dir() && line.node.name() == root_name {
            app.set_error("cannot delete root directory".into(), 3);
            return;
        }
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

pub(crate) fn move_cursor(app: &mut App, delta: isize) {
    let len = if !app.current_children.is_empty() {
        app.current_filtered.len()
    } else {
        app.filtered_tree_lines.len()
    };
    if len == 0 {
        return;
    }
    let new_cursor = app.cursor as isize + delta;
    if new_cursor < 0 {
        app.cursor = 0;
    } else if new_cursor >= len as isize {
        app.cursor = len - 1;
    } else {
        app.cursor = new_cursor as usize;
    }
}

pub(crate) fn set_root_to_selected(app: &mut App) {
    if !app.current_children.is_empty() {
        let Some(entry) = app.selected_entry().cloned() else {
            return;
        };
        if !entry.is_dir {
            return;
        }
        if let Some(full_path) = app.selected_node_full_path() {
            if full_path == app.view_root_path {
                return;
            }
            app.view_root_path = full_path;
            app.rebuild_tree();
        }
        return;
    }
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

/// Returns true if delta detail entries exceed the popup viewport (scroll needed).
fn delta_detail_needs_scroll(state: &crate::types::DeltaDetailState) -> bool {
    let visible_rows = crossterm::terminal::size()
        .ok()
        .map(|(_, h)| ((h as f64 * 0.65) as usize).saturating_sub(4))
        .unwrap_or(10);
    state.entries.len() > visible_rows
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
    tokio::task::spawn_blocking(move || {
        let (progress_tx, progress_rx) =
            std::sync::mpsc::channel::<argus_core::scanner::ProgressUpdate>();

        let tx_clone = tx.clone();
        std::thread::spawn(move || {
            while let Ok(update) = progress_rx.recv() {
                if tx_clone
                    .try_send(AppMessage::ScanProgress {
                        file_count: update.file_count,
                        total_bytes: update.total_bytes,
                        current_path: update.current_path,
                    })
                    .is_err()
                {
                    // Channel full — drop this update; next one will carry the latest state
                }
            }
        });

        match argus_core::scan_path(&path, &cancel, Some(progress_tx)) {
            Ok(snapshot) => {
                let _ = tx.blocking_send(AppMessage::ScanComplete(snapshot));
            }
            Err(e) => {
                let _ = tx.blocking_send(AppMessage::Error(format!("scan failed: {}", e)));
            }
        }
    });
}

/// Re-sort flat-mode children in-place and refresh filtered view.
pub fn sort_children_flat(app: &mut App) {
    crate::app::sort_children(&mut app.current_children, app.sort_mode, &app.delta_cache);
    app.refresh_current_filtered();
}
