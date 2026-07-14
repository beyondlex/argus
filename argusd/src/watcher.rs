use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use crate::SHOULD_QUIT;

use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use argus_core::DeltaEvent;

/// Cap for `size_cache` / `hardlink_cache`. Past this, drop roughly half so
/// long-lived daemons do not grow unbounded. Hardlink dedup still works for
/// entries that remain after eviction.
const MAX_CACHE_ENTRIES: usize = 100_000;

pub struct WatcherState {
    pub size_cache: HashMap<PathBuf, u64>,
    pub hardlink_cache: HashMap<(u64, u64), PathBuf>,
}

impl WatcherState {
    pub fn new() -> Self {
        Self {
            size_cache: HashMap::new(),
            hardlink_cache: HashMap::new(),
        }
    }

    fn trim_caches_if_needed(&mut self) {
        trim_map_half_if_over(&mut self.size_cache, MAX_CACHE_ENTRIES);
        trim_map_half_if_over(&mut self.hardlink_cache, MAX_CACHE_ENTRIES);
    }

    pub fn file_size(&mut self, path: &Path) -> Option<u64> {
        if let Ok(meta) = std::fs::metadata(path) {
            let ino = {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    (meta.dev(), meta.ino())
                }
                #[cfg(not(unix))]
                {
                    (0, 0)
                }
            };

            #[cfg(unix)]
            {
                if meta.nlink() > 1 {
                    if let Some(existing) = self.hardlink_cache.get(&ino) {
                        if existing != path {
                            tracing::trace!("hardlink detected: {existing:?} -> {path:?}");
                            if let Some(&cached_size) = self.size_cache.get(existing) {
                                self.size_cache.insert(path.to_path_buf(), cached_size);
                                self.trim_caches_if_needed();
                                return Some(cached_size);
                            }
                        }
                    }
                    self.hardlink_cache.insert(ino, path.to_path_buf());
                }
            }

            let size = meta.len();
            self.size_cache.insert(path.to_path_buf(), size);
            self.trim_caches_if_needed();
            Some(size)
        } else {
            None
        }
    }

    pub fn remove(&mut self, path: &Path) -> Option<u64> {
        self.size_cache.remove(path)
    }

    pub fn update_size(&mut self, path: &Path, size: u64) {
        self.size_cache.insert(path.to_path_buf(), size);
        self.trim_caches_if_needed();
    }

    pub fn last_known_size(&self, path: &Path) -> Option<u64> {
        self.size_cache.get(path).copied()
    }
}

fn trim_map_half_if_over<K, V>(map: &mut HashMap<K, V>, max: usize)
where
    K: Eq + std::hash::Hash + Clone,
{
    if map.len() <= max {
        return;
    }
    let remove_count = map.len() / 2;
    let keys: Vec<K> = map.keys().take(remove_count).cloned().collect();
    for k in keys {
        map.remove(&k);
    }
}

fn event_to_delta(
    kind: &EventKind,
    paths: &[PathBuf],
    state: &mut WatcherState,
    timestamp: u64,
) -> Vec<DeltaEvent> {
    let mut events = Vec::new();

    for path in paths {
        if is_ignored(path) {
            continue;
        }

        let event = match kind {
            EventKind::Create(CreateKind::File) | EventKind::Create(CreateKind::Any) => {
                state.file_size(path).map(|size| DeltaEvent {
                    path: path.clone(),
                    delta_size: size as i64,
                    event_type: "create".into(),
                    timestamp,
                    is_agg: false,
                    process_info: None,
                })
            }
            EventKind::Modify(ModifyKind::Data(_)) => {
                let old_size = state.last_known_size(path).unwrap_or(0);
                state.file_size(path).and_then(|new_size| {
                    let delta = (new_size as i64) - (old_size as i64);
                    if delta != 0 {
                        state.update_size(path, new_size);
                        Some(DeltaEvent {
                            path: path.clone(),
                            delta_size: delta,
                            event_type: "modify".into(),
                            timestamp,
                            is_agg: false,
                            process_info: None,
                        })
                    } else {
                        None
                    }
                })
            }
            EventKind::Remove(RemoveKind::File) => state.remove(path).map(|size| DeltaEvent {
                path: path.clone(),
                delta_size: -(size as i64),
                event_type: "delete".into(),
                timestamp,
                is_agg: false,
                process_info: None,
            }),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                state.remove(path).map(|size| DeltaEvent {
                    path: path.clone(),
                    delta_size: -(size as i64),
                    event_type: "delete".into(),
                    timestamp,
                    is_agg: false,
                    process_info: None,
                })
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                state.file_size(path).map(|size| DeltaEvent {
                    path: path.clone(),
                    delta_size: size as i64,
                    event_type: "create".into(),
                    timestamp,
                    is_agg: false,
                    process_info: None,
                })
            }
            _ => None,
        };

        if let Some(ev) = event {
            events.push(ev);
        }
    }

    events
}

fn is_ignored(path: &Path) -> bool {
    let name = match path.file_name() {
        Some(n) => n.to_string_lossy(),
        None => return true,
    };

    name.starts_with('.') && name != ".ds_store"
        || name == "~"
        || name.ends_with(".swp")
        || name.ends_with(".swx")
        || name.ends_with("~")
}

pub fn start_watcher(
    watch_dirs: Vec<PathBuf>,
    event_tx: mpsc::Sender<DeltaEvent>,
) -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Event, notify::Error>>();

        let mut watcher: RecommendedWatcher =
            Watcher::new(tx, Config::default()).expect("failed to create watcher");

        for dir in &watch_dirs {
            if dir.exists() {
                watcher
                    .watch(dir, RecursiveMode::Recursive)
                    .unwrap_or_else(|e| {
                        tracing::warn!("cannot watch {dir:?}: {e}");
                    });
                tracing::info!("watching {dir:?}");
            } else {
                tracing::warn!("watch dir {dir:?} does not exist, skipping");
            }
        }

        let mut state = WatcherState::new();

        while running_clone.load(Ordering::Relaxed) && !SHOULD_QUIT.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Ok(event)) => {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);

                    let delta_events =
                        event_to_delta(&event.kind, &event.paths, &mut state, timestamp);

                    for ev in delta_events {
                        if event_tx.blocking_send(ev).is_err() {
                            tracing::error!("event channel closed");
                            return;
                        }
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!("watcher error: {e}");
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("watcher channel disconnected");
                    break;
                }
            }
        }
    });

    running
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_create_event() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("new.txt");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"hello").unwrap();

        let events = event_to_delta(
            &EventKind::Create(CreateKind::File),
            &[file.clone()],
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, 5);
        assert_eq!(events[0].event_type, "create");
        assert_eq!(events[0].path, file);
    }

    #[test]
    fn test_modify_event() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("mod.txt");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"hello").unwrap();
        state.file_size(&file);

        fs::write(&file, b"hello world").unwrap();

        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            &[file.clone()],
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, 6);
        assert_eq!(events[0].event_type, "modify");
    }

    #[test]
    fn test_modify_no_change() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("same.txt");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"hello").unwrap();
        state.file_size(&file);

        fs::write(&file, b"hello").unwrap();

        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            &[file],
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_remove_event() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("del.txt");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"delete me").unwrap();
        state.file_size(&file);

        fs::remove_file(&file).unwrap();

        let events = event_to_delta(
            &EventKind::Remove(RemoveKind::File),
            &[file.clone()],
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, -9);
        assert_eq!(events[0].event_type, "delete");
    }

    #[test]
    fn test_ignored_dotfile() {
        let dir = tempdir().unwrap();
        let file = dir.path().join(".hidden");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"secret").unwrap();

        let events = event_to_delta(
            &EventKind::Create(CreateKind::File),
            &[file],
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_rename_from_as_remove() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("old_name.txt");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"rename me").unwrap();
        state.file_size(&file);

        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            &[file],
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, -9);
        assert_eq!(events[0].event_type, "delete");
    }

    #[test]
    fn test_size_cache_update() {
        let mut state = WatcherState::new();
        let path = PathBuf::from("/tmp/test.txt");

        assert!(state.last_known_size(&path).is_none());
        state.update_size(&path, 100);
        assert_eq!(state.last_known_size(&path), Some(100));
        state.update_size(&path, 200);
        assert_eq!(state.last_known_size(&path), Some(200));
        state.remove(&path);
        assert!(state.last_known_size(&path).is_none());
    }

    #[test]
    fn test_cache_evicts_when_over_max() {
        let mut state = WatcherState::new();
        // Use a tiny local max via direct trim to avoid inserting 100k entries.
        for i in 0..10 {
            state
                .size_cache
                .insert(PathBuf::from(format!("/tmp/f{i}")), i as u64);
        }
        assert_eq!(state.size_cache.len(), 10);
        trim_map_half_if_over(&mut state.size_cache, 5);
        assert!(state.size_cache.len() <= 5);
        assert!(!state.size_cache.is_empty());
    }
}
