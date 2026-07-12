use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, AppMode};
use crate::ipc_client::IpcClient;

pub(crate) fn handle_command_key(key: KeyEvent, app: &mut App) {
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

pub(crate) fn execute_command(app: &mut App, cmd: &str) {
    let cmd = cmd.trim();

    if cmd.is_empty() {
        app.clear_command_state();
        return;
    }

    app.push_command_history(cmd);

    if cmd.eq_ignore_ascii_case("Scan") {
        app.clear_command_state();
        crate::handler::start_scan(app);
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
                                .send(crate::app::AppMessage::Info(format!(
                                    "consolidated {count} events"
                                )))
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(crate::app::AppMessage::Info(format!(
                                    "consolidation failed: {e}"
                                )))
                                .await;
                        }
                    },
                    Err(e) => {
                        let _ = tx
                            .send(crate::app::AppMessage::Info(format!(
                                "daemon connect failed: {e}"
                            )))
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
