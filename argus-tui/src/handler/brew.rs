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
    if state.report.is_some() && matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
        if let Some(ref mut s) = app.brew_state {
            s.report = None;
        }
        return;
    }

    // If info popup is showing, only Esc/q dismiss it (modal)
    if state.show_info {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            if let Some(ref mut s) = app.brew_state {
                s.show_info = false;
                s.info_path = None;
                s.info_metadata = None;
                s.info_ai = None;
            }
        }
        return;
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
                    s.cursor = s
                        .cursor
                        .saturating_add(1)
                        .min(filtered_len.saturating_sub(1));
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

    // In filter mode: Esc/Enter exit filter mode; chars go to search; rest fall through
    if let Some(ref mut s) = app.brew_state {
        if s.filter_mode {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    s.filter_mode = false;
                    return;
                }
                KeyCode::Char(c) => {
                    s.search_word.push(c);
                    apply_brew_sort_and_filter(s);
                    return;
                }
                _ => {}
            }
        }
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
                s.cursor = s
                    .cursor
                    .saturating_add(1)
                    .min(filtered_len.saturating_sub(1));
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
                    // Kick off the dependency lookup for the confirm dialog:
                    // uninstall runs with --force, so the user must see what
                    // else depends on the package before committing.
                    let pkg_idx = s.filtered.get(s.cursor).copied();
                    if let Some(idx) = pkg_idx {
                        if let Some(pkg) = s.packages.get(idx) {
                            let name = pkg.name.clone();
                            let dependents_checked = pkg.dependents > 0;
                            if !dependents_checked {
                                let tx = app.tx.clone();
                                std::thread::spawn(move || {
                                    let names = argus_core::brew_dependents_of(&name);
                                    let _ = tx.blocking_send(AppMessage::BrewDependentsReady {
                                        pkg_index: idx,
                                        names,
                                    });
                                });
                            }
                        }
                    }
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
        KeyCode::Char('i') => {
            handle_brew_info_popup(app);
        }
        KeyCode::Char('y') => {
            handle_brew_copy_path(app);
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

fn handle_brew_info_popup(app: &mut App) {
    let Some(ref state) = app.brew_state else {
        return;
    };
    let Some(&pkg_idx) = state.filtered.get(state.cursor) else {
        return;
    };
    let Some(pkg) = state.packages.get(pkg_idx) else {
        return;
    };
    let prefix = argus_core::brew_prefix();
    let path = argus_core::keg_path(&prefix, &pkg.name, &pkg.package_type).join(&pkg.version);
    match std::fs::metadata(&path) {
        Ok(meta) => {
            let info_ai = app.ai_cache.get(&path).cloned().or_else(|| {
                let path_str = path.to_string_lossy();
                if let Ok(conn) = argus_core::open_db(&argus_core::default_db_path()) {
                    if let Ok(Some(data)) = argus_core::get_ai_analysis(&conn, &path_str) {
                        if let Ok(verdict) = serde_json::from_slice(&data) {
                            return Some(verdict);
                        }
                    }
                }
                None
            });
            if let Some(ref mut s) = app.brew_state {
                s.show_info = true;
                s.info_path = Some(path);
                s.info_metadata = Some(meta);
                s.info_ai = info_ai;
            }
        }
        Err(e) => app.set_error(format!("stat failed: {}", e), 3),
    }
}

fn handle_brew_copy_path(app: &mut App) {
    let Some(ref state) = app.brew_state else {
        return;
    };
    let Some(&pkg_idx) = state.filtered.get(state.cursor) else {
        return;
    };
    let Some(pkg) = state.packages.get(pkg_idx) else {
        return;
    };
    let prefix = argus_core::brew_prefix();
    let path = argus_core::keg_path(&prefix, &pkg.name, &pkg.package_type).join(&pkg.version);
    let path_str = path.display().to_string();
    match arboard::Clipboard::new() {
        Ok(mut cb) => {
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
