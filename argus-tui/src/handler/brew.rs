use crate::app::{App, AppMessage};
use crate::types::{BrewFilterType, BrewSortMode};
use crossterm::event::{KeyCode, KeyEvent};

pub(crate) fn handle_brew_key(key: KeyEvent, app: &mut App) {
    if app.brew_state.as_ref().is_some_and(|s| s.confirm_pending) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let pkg = {
                    let s = app.brew_state.as_ref().unwrap();
                    s.selected_pkg
                        .and_then(|i| s.filtered.get(i).copied())
                        .and_then(|i| s.packages.get(i).cloned())
                };
                let Some(pkg) = pkg else { return };
                let tx = app.tx.clone();
                if let Some(ref mut s) = app.brew_state {
                    s.confirm_pending = false;
                    s.uninstalling = true;
                }
                std::thread::spawn(move || {
                    let report = argus_core::uninstall_brew_package(&pkg);
                    match report {
                        Ok(r) => {
                            let _ = tx.blocking_send(AppMessage::BrewUninstallComplete(r));
                        }
                        Err(e) => {
                            let _ = tx.blocking_send(AppMessage::Error(format!(
                                "brew uninstall failed: {e}"
                            )));
                        }
                    }
                });
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                if let Some(ref mut s) = app.brew_state {
                    s.confirm_pending = false;
                    s.selected_pkg = None;
                }
            }
            _ => {}
        }
        return;
    }

    let Some(ref state) = app.brew_state else {
        return;
    };

    // Allow q/Esc to cancel scan
    if state.scanning {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            app.exit_brew();
        }
        return;
    }

    if state.uninstalling {
        return;
    }

    // If report is showing, Esc clears it (returns to pkg list)
    if state.report.is_some() {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            if let Some(ref mut s) = app.brew_state {
                s.report = None;
            }
            return;
        }
    }

    let filtered_len = state.filtered.len();

    // Multi-select mode key handling
    if state.multi_select {
        match key.code {
            KeyCode::Char(' ') => {
                if let Some(ref mut s) = app.brew_state {
                    let pkg_idx = s.filtered.get(s.cursor).copied();
                    if let Some(idx) = pkg_idx {
                        if s.selected_pkgs.contains(&idx) {
                            s.selected_pkgs.remove(&idx);
                        } else {
                            s.selected_pkgs.insert(idx);
                        }
                    }
                    s.cursor = s.cursor.saturating_add(1).min(filtered_len.saturating_sub(1));
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if let Some(ref s) = app.brew_state {
                    if s.selected_pkgs.is_empty() {
                        app.set_info("no packages selected".into(), 3);
                    } else {
                        app.enter_brew_ai_review();
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                if let Some(ref mut s) = app.brew_state {
                    if s.selected_pkgs.is_empty() {
                        s.multi_select = false;
                    } else {
                        s.multi_select = false;
                        s.selected_pkgs.clear();
                    }
                }
            }
            _ => {
                // Fall through to normal movement keys
                handle_brew_navigation(key, app, filtered_len);
            }
        }
        return;
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            handle_brew_navigation(key, app, filtered_len);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            handle_brew_navigation(key, app, filtered_len);
        }
        KeyCode::Char('g') => {
            if app.pending_gg {
                if let Some(ref mut s) = app.brew_state {
                    s.cursor = 0;
                }
                app.pending_gg = false;
            } else {
                app.pending_gg = true;
            }
        }
        KeyCode::Char('G') => {
            if let Some(ref mut s) = app.brew_state {
                s.cursor = filtered_len.saturating_sub(1);
            }
            app.pending_gg = false;
        }
        KeyCode::Char(' ') => {
            if let Some(ref mut s) = app.brew_state {
                s.multi_select = true;
                let pkg_idx = s.filtered.get(s.cursor).copied();
                if let Some(idx) = pkg_idx {
                    s.selected_pkgs.insert(idx);
                }
                s.cursor = s.cursor.saturating_add(1).min(filtered_len.saturating_sub(1));
            }
        }
        KeyCode::Char('/') => {
            if let Some(ref mut s) = app.brew_state {
                s.filter_mode = true;
                s.search_word.clear();
            }
        }
        KeyCode::Char('o') => {
            if let Some(ref mut s) = app.brew_state {
                s.sort_mode = match s.sort_mode {
                    BrewSortMode::Time => BrewSortMode::Size,
                    BrewSortMode::Size => BrewSortMode::Name,
                    BrewSortMode::Name => BrewSortMode::Type,
                    BrewSortMode::Type => BrewSortMode::Time,
                };
                apply_brew_sort_and_filter(s);
            }
        }
        KeyCode::Char('t') => {
            if let Some(ref mut s) = app.brew_state {
                s.filter_type = match s.filter_type {
                    BrewFilterType::All => BrewFilterType::Formula,
                    BrewFilterType::Formula => BrewFilterType::Cask,
                    BrewFilterType::Cask => BrewFilterType::All,
                };
                apply_brew_sort_and_filter(s);
            }
        }
        KeyCode::Enter => {
            if let Some(ref mut s) = app.brew_state {
                if !s.filtered.is_empty() {
                    s.selected_pkg = Some(s.cursor);
                    s.confirm_pending = true;
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut s) = app.brew_state {
                s.search_word.pop();
                apply_brew_sort_and_filter(s);
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.exit_brew();
        }
        _ => {}
    }
}

fn handle_brew_navigation(_key: KeyEvent, app: &mut App, filtered_len: usize) {
    match _key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ref mut s) = app.brew_state {
                s.cursor = s
                    .cursor
                    .saturating_add(1)
                    .min(filtered_len.saturating_sub(1));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut s) = app.brew_state {
                s.cursor = s.cursor.saturating_sub(1);
            }
        }
        _ => {}
    }
}

fn apply_brew_sort_and_filter(state: &mut crate::types::BrewState) {
    // Filter by type
    let mut indices: Vec<usize> = state
        .packages
        .iter()
        .enumerate()
        .filter(|(_, p)| match state.filter_type {
            BrewFilterType::All => true,
            BrewFilterType::Formula => p.package_type == argus_core::BrewPackageType::Formula,
            BrewFilterType::Cask => p.package_type == argus_core::BrewPackageType::Cask,
        })
        .map(|(i, _)| i)
        .collect();

    // Filter by search
    if !state.search_word.is_empty() {
        let query = state.search_word.to_lowercase();
        indices.retain(|&i| {
            let p = &state.packages[i];
            p.name.to_lowercase().contains(&query) || p.description.to_lowercase().contains(&query)
        });
    }

    // Sort by the selected mode
    let packages = &state.packages;
    indices.sort_by(|&a, &b| {
        let pa = &packages[a];
        let pb = &packages[b];
        match state.sort_mode {
            BrewSortMode::Time => match (&pa.last_used, &pb.last_used) {
                (None, None) => pb.size.cmp(&pa.size),
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (Some(a_dt), Some(b_dt)) => a_dt.cmp(b_dt),
            },
            BrewSortMode::Size => pb.size.cmp(&pa.size),
            BrewSortMode::Name => pa.name.to_lowercase().cmp(&pb.name.to_lowercase()),
            BrewSortMode::Type => pa
                .package_type
                .label()
                .cmp(pb.package_type.label())
                .then_with(|| pa.name.to_lowercase().cmp(&pb.name.to_lowercase())),
        }
    });

    state.filtered = indices;
    state.cursor = 0;
}

/// Format a timestamp as relative time string for display.
pub(crate) fn format_brew_time(dt: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match dt {
        None => "never".to_string(),
        Some(dt) => {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(dt);
            let days = duration.num_days();
            if days == 0 {
                "today".to_string()
            } else if days == 1 {
                "yesterday".to_string()
            } else if days < 30 {
                format!("{}d ago", days)
            } else if days < 365 {
                format!("{}mo ago", days / 30)
            } else {
                format!("{}y ago", days / 365)
            }
        }
    }
}
