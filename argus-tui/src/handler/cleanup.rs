use crate::app::{App, AppMessage};
use crate::types::UninstallPhase;
use crossterm::event::{KeyCode, KeyEvent};
use std::path::{Path, PathBuf};

pub(crate) fn handle_cleanup_key(key: KeyEvent, app: &mut App) {
    if app.cleanup_state.as_ref().is_some_and(|s| s.detail_pending) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('i') => {
                if let Some(ref mut s) = app.cleanup_state {
                    s.detail_pending = false;
                    s.detail_items = None;
                }
            }
            _ => {}
        }
        return;
    }

    if app
        .cleanup_state
        .as_ref()
        .is_some_and(|s| s.confirm_pending)
    {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let (items, selected, dry_run) = {
                    let s = app.cleanup_state.as_ref().unwrap();
                    (s.items.clone(), s.selected.clone(), s.dry_run)
                };
                let to_delete: Vec<argus_core::CleanItem> = items
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| selected.contains(i))
                    .map(|(_, item)| item)
                    .collect();
                let tx = app.tx.clone();
                app.cleanup_state.as_mut().unwrap().confirm_pending = false;
                // Dry-run must not touch the filesystem: the panel advertises
                // "[DRY-RUN]", so confirmation only reports what WOULD be freed.
                // (exec_clean's second parameter is ignored by core, so passing
                // the flag through used to delete for real in dry-run mode.)
                if dry_run {
                    let freed: u64 = to_delete.iter().map(|i| i.size).sum();
                    let report = argus_core::CleanReport {
                        total_attempted: to_delete.len() as u64,
                        total_succeeded: 0,
                        total_failed: 0,
                        freed_bytes: freed,
                        errors: Vec::new(),
                    };
                    let _ = tx.blocking_send(AppMessage::CleanupExecComplete(report));
                } else {
                    std::thread::spawn(move || {
                        let report = argus_core::exec_clean(&to_delete);
                        match report {
                            Ok(r) => {
                                let _ = tx.blocking_send(AppMessage::CleanupExecComplete(r));
                            }
                            Err(e) => {
                                let _ = tx
                                    .blocking_send(AppMessage::Error(format!("clean failed: {e}")));
                            }
                        }
                    });
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                if let Some(ref mut s) = app.cleanup_state {
                    s.confirm_pending = false;
                }
            }
            _ => {}
        }
        return;
    }

    let Some(ref state) = app.cleanup_state else {
        return;
    };

    // Allow q/Esc to leave during the (potentially minutes-long) target scan;
    // late CleanupScanComplete messages hit the None state guard and no-op.
    if state.scanning {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            app.exit_cleanup();
        }
        return;
    }

    let item_count = state.items.len();

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ref mut s) = app.cleanup_state {
                s.cursor = s.cursor.saturating_add(1).min(item_count.saturating_sub(1));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut s) = app.cleanup_state {
                s.cursor = s.cursor.saturating_sub(1);
            }
        }
        KeyCode::Char('g') => {
            if app.pending_gg {
                if let Some(ref mut s) = app.cleanup_state {
                    s.cursor = 0;
                }
                app.pending_gg = false;
            } else {
                app.pending_gg = true;
            }
        }
        KeyCode::Char('G') => {
            if let Some(ref mut s) = app.cleanup_state {
                s.cursor = item_count.saturating_sub(1);
            }
            app.pending_gg = false;
        }
        KeyCode::Char(' ') => {
            if let Some(ref mut s) = app.cleanup_state {
                if s.selected.contains(&s.cursor) {
                    s.selected.remove(&s.cursor);
                } else {
                    s.selected.insert(s.cursor);
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(ref mut s) = app.cleanup_state {
                s.dry_run = !s.dry_run;
            }
        }
        KeyCode::Char('i') => {
            let path = {
                let s = app.cleanup_state.as_ref().unwrap();
                s.items.get(s.cursor).map(|item| item.path.clone())
            };
            if let Some(path) = path {
                app.cleanup_state.as_mut().unwrap().detail_pending = true;
                let tx = app.tx.clone();
                std::thread::spawn(move || {
                    let details = scan_dir_details(&path);
                    let _ = tx.blocking_send(AppMessage::CleanupDetailReady(details));
                });
            }
        }
        KeyCode::Enter => {
            if let Some(ref mut s) = app.cleanup_state {
                if !s.selected.is_empty() {
                    s.confirm_pending = true;
                }
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.exit_cleanup();
        }
        _ => {}
    }
}

pub(crate) fn handle_uninstall_key(key: KeyEvent, app: &mut App) {
    if app
        .uninstall_state
        .as_ref()
        .is_some_and(|s| s.confirm_pending)
    {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // The confirm panel's per-item leftover toggles are the source
                // of truth: the bool-only core call re-ran find_leftovers and
                // trashed everything, silently deleting items the user had
                // deselected (and re-walking every subtree it had just sized).
                let (app_info, leftover_paths) = {
                    let s = app.uninstall_state.as_ref().unwrap();
                    let app_idx = s.selected_app.unwrap_or(0);
                    let app_info = s.apps.get(app_idx).cloned();
                    let leftover_paths = match (s.remove_leftovers, s.leftovers.as_ref()) {
                        (true, Some(leftovers)) => leftovers
                            .leftover_paths
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| s.selected_leftovers.contains(i))
                            .map(|(_, p)| p.clone())
                            .collect(),
                        _ => Vec::new(),
                    };
                    (app_info, leftover_paths)
                };
                let Some(app_info) = app_info else { return };
                let tx = app.tx.clone();
                app.uninstall_state.as_mut().unwrap().confirm_pending = false;
                std::thread::spawn(move || {
                    let report =
                        argus_core::uninstall_app_with_leftovers(&app_info, &leftover_paths);
                    match report {
                        Ok(r) => {
                            let _ = tx.blocking_send(AppMessage::UninstallComplete(r));
                        }
                        Err(e) => {
                            let _ = tx
                                .blocking_send(AppMessage::Error(format!("uninstall failed: {e}")));
                        }
                    }
                });
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                if let Some(ref mut s) = app.uninstall_state {
                    s.confirm_pending = false;
                }
            }
            _ => {}
        }
        return;
    }

    let Some(ref state) = app.uninstall_state else {
        return;
    };

    // Allow q/Esc to leave during the app-list scan (one mdls spawn per app);
    // late AppListReady messages hit the None state guard and no-op.
    if state.scanning {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            app.exit_uninstall();
        }
        return;
    }

    match state.phase {
        UninstallPhase::SelectApp => handle_uninstall_select_app(key, app),
        UninstallPhase::Confirm => handle_uninstall_confirm(key, app),
    }
}

fn handle_uninstall_select_app(key: KeyEvent, app: &mut App) {
    let is_filter_mode = app.uninstall_state.as_ref().is_some_and(|s| s.filter_mode);

    if is_filter_mode {
        match key.code {
            // Enter closes the filter like the brew panel: it used to be
            // swallowed, forcing Esc-then-Enter to act on the filtered list.
            KeyCode::Esc | KeyCode::Enter => {
                if let Some(ref mut s) = app.uninstall_state {
                    s.filter_mode = false;
                }
            }
            KeyCode::Char(c) => {
                if let Some(ref mut s) = app.uninstall_state {
                    s.search_word.push(c);
                    s.filtered = s
                        .apps
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| {
                            a.name
                                .to_lowercase()
                                .contains(&s.search_word.to_lowercase())
                        })
                        .map(|(i, _)| i)
                        .collect();
                    s.cursor = 0;
                }
            }
            KeyCode::Backspace => {
                if let Some(ref mut s) = app.uninstall_state {
                    s.search_word.pop();
                    s.filtered = s
                        .apps
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| {
                            a.name
                                .to_lowercase()
                                .contains(&s.search_word.to_lowercase())
                        })
                        .map(|(i, _)| i)
                        .collect();
                    s.cursor = 0;
                }
            }
            _ => {}
        }
        return;
    }

    let item_count = {
        let s = app.uninstall_state.as_ref().unwrap();
        s.filtered.len()
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = s.cursor.saturating_add(1).min(item_count.saturating_sub(1));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = s.cursor.saturating_sub(1);
            }
        }
        KeyCode::Char('g') => {
            if app.pending_gg {
                if let Some(ref mut s) = app.uninstall_state {
                    s.cursor = 0;
                }
                app.pending_gg = false;
            } else {
                app.pending_gg = true;
            }
        }
        KeyCode::Char('G') => {
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = item_count.saturating_sub(1);
            }
            app.pending_gg = false;
        }
        KeyCode::Char('/') => {
            if let Some(ref mut s) = app.uninstall_state {
                s.filter_mode = true;
            }
        }
        KeyCode::Char('o') => {
            if let Some(ref mut s) = app.uninstall_state {
                s.sort_mode = (s.sort_mode + 1) % 3;
                match s.sort_mode {
                    0 => s.apps.sort_by_key(|a| std::cmp::Reverse(a.size)),
                    1 => s.apps.sort_by(|a, b| {
                        let a_t = a.last_used.unwrap_or_default();
                        let b_t = b.last_used.unwrap_or_default();
                        b_t.cmp(&a_t)
                    }),
                    _ => s.apps.sort_by_key(|a| a.name.to_lowercase()),
                }
                s.filtered = (0..s.apps.len()).collect();
                s.cursor = 0;
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut s) = app.uninstall_state {
                s.search_word.pop();
                s.filtered = s
                    .apps
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| {
                        a.name
                            .to_lowercase()
                            .contains(&s.search_word.to_lowercase())
                    })
                    .map(|(i, _)| i)
                    .collect();
                s.cursor = 0;
            }
        }
        KeyCode::Enter => {
            let (selected_idx, _search_word) = {
                let s = app.uninstall_state.as_ref().unwrap();
                (s.filtered.get(s.cursor).copied(), s.search_word.clone())
            };
            if let Some(idx) = selected_idx {
                if let Some(ref mut s) = app.uninstall_state {
                    s.selected_app = Some(idx);
                    s.phase = UninstallPhase::Confirm;
                    s.scanning = true;
                    s.cursor = 0;
                }
                let app_info = app.uninstall_state.as_ref().unwrap().apps[idx].clone();
                let tx = app.tx.clone();
                std::thread::spawn(move || match argus_core::find_leftovers(&app_info) {
                    Ok(leftovers) => {
                        let _ = tx.blocking_send(AppMessage::UninstallLeftoversReady(leftovers));
                    }
                    Err(e) => {
                        let _ = tx
                            .blocking_send(AppMessage::Error(format!("leftover scan failed: {e}")));
                    }
                });
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.exit_uninstall();
        }
        _ => {}
    }
}

fn handle_uninstall_confirm(key: KeyEvent, app: &mut App) {
    let leftover_count = {
        let s = app.uninstall_state.as_ref().unwrap();
        s.leftovers
            .as_ref()
            .map(|l| l.leftover_paths.len())
            .unwrap_or(0)
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = s
                    .cursor
                    .saturating_add(1)
                    .min(leftover_count.saturating_sub(1));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(ref mut s) = app.uninstall_state {
                s.cursor = s.cursor.saturating_sub(1);
            }
        }
        KeyCode::Char(' ') => {
            if let Some(ref mut s) = app.uninstall_state {
                if s.selected_leftovers.contains(&s.cursor) {
                    s.selected_leftovers.remove(&s.cursor);
                } else {
                    s.selected_leftovers.insert(s.cursor);
                }
            }
        }
        KeyCode::Char('t') | KeyCode::Tab => {
            if let Some(ref mut s) = app.uninstall_state {
                s.remove_leftovers = !s.remove_leftovers;
            }
        }
        KeyCode::Enter => {
            if let Some(ref mut s) = app.uninstall_state {
                s.confirm_pending = true;
            }
        }
        KeyCode::Esc => {
            if let Some(ref mut s) = app.uninstall_state {
                s.phase = UninstallPhase::SelectApp;
                s.leftovers = None;
                s.cursor = 0;
            }
        }
        _ => {}
    }
}

/// One post-order pass computing every directory's cumulative size.
///
/// The old implementation called [`argus_core::dir_size`] once per directory
/// entry, re-walking each subtree once per ancestor — quadratic on deep trees
/// (`~/Library/Caches` with nested per-app dirs made the detail popup spin for
/// minutes). Bottom-up accumulation walks the tree exactly once.
fn scan_dir_details(path: &Path) -> Vec<(String, u64)> {
    let mut entries = Vec::new();
    if !path.is_dir() {
        if let Ok(meta) = path.metadata() {
            entries.push((
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                meta.len(),
            ));
        }
        return entries;
    }

    /// A directory whose children are being resolved. When `pending` is
    /// exhausted, the frame's size is `file_bytes + child_sizes.sum()`.
    struct DirFrame {
        rel: String,
        file_bytes: u64,
        pending: std::vec::IntoIter<(PathBuf, String)>,
        child_sizes: Vec<u64>,
    }

    /// Read `dir` in one pass: files are recorded and accumulated right away,
    /// child directories are queued for their own frames. `rel` is `dir`'s
    /// display path relative to the scan root ("app" → child "app/log.txt");
    /// the root frame carries an empty rel.
    fn push_frame(
        stack: &mut Vec<DirFrame>,
        entries: &mut Vec<(String, u64)>,
        dir: &Path,
        rel: &str,
    ) {
        let mut file_bytes = 0u64;
        let mut pending: Vec<(PathBuf, String)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                let child_rel = if rel.is_empty() {
                    name
                } else {
                    format!("{rel}/{name}")
                };
                if meta.is_dir() {
                    // entry.metadata() is lstat-based: symlinks to dirs (or
                    // files) fall through both branches, matching the
                    // dir_size contract this replaces.
                    pending.push((entry.path(), child_rel));
                } else if meta.is_file() {
                    let size = meta.len();
                    file_bytes += size;
                    entries.push((child_rel, size));
                }
            }
        }
        stack.push(DirFrame {
            rel: rel.to_string(),
            file_bytes,
            pending: pending.into_iter(),
            child_sizes: Vec::new(),
        });
    }

    let mut stack: Vec<DirFrame> = Vec::new();
    push_frame(&mut stack, &mut entries, path, "");

    while let Some(frame) = stack.last_mut() {
        let next_child = frame.pending.next();
        let Some((child, child_rel)) = next_child else {
            // All children resolved: finalize this directory.
            let frame = stack
                .pop()
                .unwrap_or_else(|| unreachable!("frame just peeked"));
            let total = frame.child_sizes.iter().copied().sum::<u64>() + frame.file_bytes;
            if let Some(parent) = stack.last_mut() {
                parent.child_sizes.push(total);
            }
            // The root (empty rel) is the item itself — the panel header
            // already shows its total, so only nested dirs become entries.
            if !frame.rel.is_empty() {
                entries.push((frame.rel, total));
            }
            continue;
        };

        push_frame(&mut stack, &mut entries, &child, &child_rel);
    }

    entries.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    entries.truncate(200);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_dir_details_single_file() {
        let tmp = std::env::temp_dir().join("_argus_detail_single");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("item.bin");
        fs::write(&file, [0u8; 512]).unwrap();

        let entries = scan_dir_details(&file);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "item.bin");
        assert_eq!(entries[0].1, 512);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Nested directories must report cumulative subtree sizes computed in
    /// one walk: parent = own files + all nested content.
    #[test]
    fn test_scan_dir_details_cumulative_dir_sizes() {
        let tmp = std::env::temp_dir().join("_argus_detail_nested");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("app/cache")).unwrap();
        fs::write(tmp.join("app/cache/blob.bin"), [0u8; 300]).unwrap();
        fs::write(tmp.join("app/log.txt"), [0u8; 100]).unwrap();

        let entries = scan_dir_details(&tmp);
        let find = |name: &str| entries.iter().find(|(r, _)| r == name).map(|(_, s)| *s);
        assert_eq!(find("app/log.txt"), Some(100));
        assert_eq!(find("app/cache/blob.bin"), Some(300));
        assert_eq!(find("app/cache"), Some(300));
        assert_eq!(find("app"), Some(400));
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Symlinked directories must not be followed or listed (their targets
    /// would double-count shared data), matching the dir_size contract.
    #[cfg(unix)]
    #[test]
    fn test_scan_dir_details_skips_symlink_dirs() {
        use std::os::unix::fs::symlink;
        let tmp = std::env::temp_dir().join("_argus_detail_symlink");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("real")).unwrap();
        fs::write(tmp.join("real/big.bin"), [0u8; 1000]).unwrap();
        symlink(tmp.join("real"), tmp.join("alias")).unwrap();

        let entries = scan_dir_details(&tmp);
        assert!(
            !entries.iter().any(|(r, _)| r == "alias"),
            "symlinked dir must not appear"
        );
        let _ = fs::remove_dir_all(&tmp);
    }
}
