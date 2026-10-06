use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, AppMessage, AppMode};
use crate::ipc_client::IpcClient;
use crate::util::log_msg;
pub(crate) fn handle_command_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char(c) if app.command_input.len() < 200 => {
            app.command_input.push(c);
            app.update_command_matches();
            app.command_history_idx = None;
            app.command_scroll = 0;
        }
        KeyCode::Backspace => {
            app.command_input.pop();
            app.update_command_matches();
            app.command_history_idx = None;
            app.command_scroll = 0;
        }
        KeyCode::Tab if !app.command_matches.is_empty() => {
            app.command_selected = (app.command_selected + 1) % app.command_matches.len();
            app.command_input = app.command_matches[app.command_selected].to_string();
            app.command_history_idx = None;
            app.command_scroll = app.command_selected.saturating_sub(7);
        }
        KeyCode::BackTab if !app.command_matches.is_empty() => {
            app.command_selected = if app.command_selected == 0 {
                app.command_matches.len() - 1
            } else {
                app.command_selected - 1
            };
            app.command_input = app.command_matches[app.command_selected].to_string();
            app.command_history_idx = None;
            app.command_scroll = app.command_selected.saturating_sub(7);
        }
        // Up/Down are reserved for history recall — they used to navigate the
        // completion list, which is non-empty for almost any input, making
        // history effectively unreachable (review note #29). Completion
        // cycling stays on Tab/BackTab; j/k now type literally, so command
        // arguments can contain them (review note #27).
        KeyCode::Up if !app.command_history.is_empty() => {
            let idx = match app.command_history_idx {
                Some(i) if i > 0 => i - 1,
                None => app.command_history.len() - 1,
                _ => return,
            };
            app.command_history_idx = Some(idx);
            app.command_input = app.command_history[idx].clone();
            app.update_command_matches();
            app.command_scroll = 0;
        }
        KeyCode::Down if app.command_history_idx.is_some() => {
            let idx = app.command_history_idx.unwrap();
            if idx + 1 < app.command_history.len() {
                app.command_history_idx = Some(idx + 1);
                app.command_input = app.command_history[idx + 1].clone();
            } else {
                app.command_history_idx = None;
                app.command_input.clear();
            }
            app.update_command_matches();
            app.command_scroll = 0;
        }
        KeyCode::Enter => {
            let input = app.command_input.trim().to_string();
            let Some(cmd) =
                resolve_enter_command(&input, &app.command_matches, app.command_selected)
            else {
                // Empty input: close the bar instead of executing whatever
                // happens to be the first completion (used to open Brew).
                app.clear_command_state();
                app.mode = AppMode::Browsing;
                return;
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

/// Decide what Enter executes. The typed input wins unless it is a
/// case-insensitive prefix of the highlighted completion (plain
/// abbreviations like `cle` still expand to `Clean`). Selection by fuzzy
/// subsequence alone used to replace typed aliases with an unrelated
/// command: `sn` (sort-by-name) is a subsequence of `Scan`, so Enter
/// started a full scan instead.
pub(crate) fn resolve_enter_command(
    input: &str,
    matches: &[&'static str],
    selected: usize,
) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    if let Some(&m) = matches.get(selected) {
        if m.to_lowercase().starts_with(&input.to_lowercase()) {
            return Some(m.to_string());
        }
    }
    Some(input.to_string())
}

pub(crate) fn execute_command(app: &mut App, cmd: &str) {
    let cmd = cmd.trim();

    if cmd.is_empty() {
        app.clear_command_state();
        return;
    }

    app.push_command_history(cmd);

    if cmd.eq_ignore_ascii_case("Clean") {
        app.clear_command_state();
        app.enter_cleanup(crate::types::CleanupMode::Clean);
        return;
    }

    if cmd.eq_ignore_ascii_case("Purge") {
        app.clear_command_state();
        app.enter_cleanup(crate::types::CleanupMode::Purge);
        return;
    }

    if cmd.eq_ignore_ascii_case("Uninstall") {
        app.clear_command_state();
        app.enter_uninstall();
        return;
    }

    if cmd.eq_ignore_ascii_case("Brew") {
        app.clear_command_state();
        app.enter_brew();
        return;
    }

    if cmd.eq_ignore_ascii_case("Scan") {
        app.clear_command_state();
        crate::handler::start_scan(app);
        return;
    }

    if cmd.eq_ignore_ascii_case("Connect") {
        app.clear_command_state();
        let uds_path = app.config.daemon.uds_path.clone();
        let tx = app.tx.clone();
        let log_path = app.log_path.clone();
        tokio::spawn(async move {
            if let Ok(mut client) = IpcClient::connect(&uds_path).await {
                if client.ping().await.is_ok() {
                    log_msg(&log_path, ":connect: connected to daemon");
                    let _ = tx.send(AppMessage::DaemonConnected(client)).await;
                    return;
                }
            }
            let _ = tx
                // Failure must use the error channel: Info renders in the
                // success color, so "daemon connect failed" read as if the
                // connect had succeeded (same fix as :Consolidate, review 12).
                .send(AppMessage::Error("daemon connect failed".into()))
                .await;
        });
        return;
    }
    if cmd.eq_ignore_ascii_case("Consolidate") {
        app.clear_command_state();
        if app.server_mode {
            app.request_consolidation();
        } else {
            app.set_error("not in server mode".into(), 3);
        }
        return;
    }

    match app.execute_command(cmd) {
        Ok(msg) => {
            app.set_info(msg, 3);
        }
        Err(e) => {
            app.set_error(e, 4);
        }
    }
    app.clear_command_state();
}
