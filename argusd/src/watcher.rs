use std::collections::{HashMap, HashSet};
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

use crate::config::WatchDir;
use argus_core::DeltaEvent;

/// Cap for `size_cache` / `hardlink_cache`. Past this, drop roughly half so
/// long-lived daemons do not grow unbounded. Hardlink dedup still works for
/// entries that remain after eviction.
const MAX_CACHE_ENTRIES: usize = 100_000;

pub struct WatcherState {
    pub size_cache: HashMap<PathBuf, u64>,
    pub hardlink_cache: HashMap<(u64, u64), PathBuf>,
    /// Paths whose create was suppressed as a duplicate hard link. Their
    /// size_cache entry is only a baseline (the shared data is booked under
    /// the first link), so a later remove of such a path must book nothing —
    /// it used to subtract the full baseline, recording a phantom `-size`
    /// for `cp -l big.bin link.bin && rm link.bin`.
    pub dup_link_seeds: HashSet<PathBuf>,
}

impl WatcherState {
    pub fn new() -> Self {
        Self {
            size_cache: HashMap::new(),
            hardlink_cache: HashMap::new(),
            dup_link_seeds: HashSet::new(),
        }
    }

    fn trim_caches_if_needed(&mut self) {
        trim_map_half_if_over(&mut self.size_cache, MAX_CACHE_ENTRIES);
        trim_map_half_if_over(&mut self.hardlink_cache, MAX_CACHE_ENTRIES);
        trim_set_half_if_over(&mut self.dup_link_seeds, MAX_CACHE_ENTRIES);
    }

    /// Current size of `path`, observing it into the caches. Directories and
    /// symlinks never take part in size accounting: a directory's stat size
    /// (~4 KiB of entry data) is not user data, and `stat` on a symlink
    /// reports the *target's* size, which booked phantom deltas whenever a
    /// link was created or removed (Homebrew and `node_modules/.bin` create
    /// many). Matches the scanner, which counts symlinks as size 0.
    pub fn file_size(&mut self, path: &Path) -> Option<u64> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        if meta.is_dir() || meta.is_symlink() {
            return None;
        }
        #[cfg(unix)]
        {
            // Track every observed inode, not only nlink > 1: the first link
            // of a pair is observed while it still looks like a regular file,
            // and create-dedup needs that first path recorded.
            self.hardlink_cache
                .insert((meta.dev(), meta.ino()), path.to_path_buf());
        }
        let size = meta.len();
        self.size_cache.insert(path.to_path_buf(), size);
        // From here on the path has real accounting (its modifies book deltas);
        // a later remove must negate them like any ordinary file's.
        self.dup_link_seeds.remove(path);
        self.trim_caches_if_needed();
        Some(size)
    }

    /// Size delta to book for a Create event. `None` when the path is a
    /// duplicate hard link of an already-accounted file: the shared data was
    /// counted under the first link, and booking the full size again
    /// double-counted (`cp -l big.bin link.bin` booked +size for a no-op).
    /// The scanner dedups the same case — see `SeenInodes` in argus-core.
    ///
    /// A stale inode mapping must not suppress the create: renames often
    /// surface as remove+create and the mapping still points at the vanished
    /// source path, so the mapped path is verified to still exist *and* hold
    /// the same (device, inode) before treating the event as a dup link.
    pub fn create_size(&mut self, path: &Path) -> Option<u64> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        if meta.is_dir() || meta.is_symlink() {
            return None;
        }
        #[cfg(unix)]
        {
            let ino = (meta.dev(), meta.ino());
            if let Some(existing) = self.hardlink_cache.get(&ino).cloned() {
                let still_same_file = std::fs::symlink_metadata(&existing)
                    .map(|m| m.dev() == ino.0 && m.ino() == ino.1)
                    .unwrap_or(false);
                if existing != path && still_same_file {
                    // Seed the new path's baseline so later modifies measure
                    // against the shared size, but book no delta — and mark
                    // it so the matching remove books nothing either.
                    let size = meta.len();
                    self.size_cache.insert(path.to_path_buf(), size);
                    self.dup_link_seeds.insert(path.to_path_buf());
                    self.trim_caches_if_needed();
                    return None;
                }
            }
            self.hardlink_cache.insert(ino, path.to_path_buf());
        }
        let size = meta.len();
        self.size_cache.insert(path.to_path_buf(), size);
        self.trim_caches_if_needed();
        Some(size)
    }

    /// Negative delta for a Remove event: the cached size, or `None` when the
    /// path was a duplicate hard link whose data still lives under the first
    /// link (nothing is freed, so nothing may be subtracted).
    pub fn remove(&mut self, path: &Path) -> Option<u64> {
        if self.dup_link_seeds.remove(path) {
            self.size_cache.remove(path);
            return None;
        }
        self.size_cache.remove(path)
    }

    /// Negative delta for a whole removed directory: the sum of every still
    /// cached size under `dir`, which is dropped from the caches together
    /// with its dup-link seeds and hardlink mappings.
    ///
    /// Backends usually report a recursive delete file-by-file, so by the
    /// time the folder-level remove arrives the cache under the prefix is
    /// already empty and this returns nothing (no double counting). When
    /// coalescing swallows the per-file events instead (FSEvents under
    /// pressure reports `MUST_SCAN_SUBDIRS` / only the top dir), the folder
    /// event is the only delete that will ever be seen, and this recovers
    /// the subtree's churn from the cache.
    pub fn remove_tree(&mut self, dir: &Path) -> Option<u64> {
        let keys: Vec<PathBuf> = self
            .size_cache
            .keys()
            .filter(|p| p.starts_with(dir))
            .cloned()
            .collect();
        if keys.is_empty() {
            return None;
        }
        let mut total = 0u64;
        for key in keys {
            if let Some(size) = self.size_cache.remove(&key) {
                total += size;
            }
            self.dup_link_seeds.remove(&key);
        }
        self.hardlink_cache.retain(|_, p| !p.starts_with(dir));
        Some(total)
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

fn trim_set_half_if_over<T>(set: &mut HashSet<T>, max: usize)
where
    T: Eq + std::hash::Hash + Clone,
{
    if set.len() <= max {
        return;
    }
    let remove_count = set.len() / 2;
    let keys: Vec<T> = set.iter().take(remove_count).cloned().collect();
    for k in keys {
        set.remove(&k);
    }
}

/// Find the matching watch dir with the longest path prefix.
/// Returns None if the event path is not under any watched directory.
fn match_watch_dir<'a>(path: &Path, watch_dirs: &'a [WatchDir]) -> Option<&'a WatchDir> {
    watch_dirs
        .iter()
        .filter(|wd| path.starts_with(&wd.path))
        .max_by_key(|wd| wd.path.as_os_str().len())
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
                state.create_size(path).map(|size| DeltaEvent {
                    path: path.clone(),
                    delta_size: size as i64,
                    event_type: "create".into(),
                    timestamp,
                    is_agg: false,
                    process_info: None,
                })
            }
            EventKind::Modify(ModifyKind::Data(_)) | EventKind::Modify(ModifyKind::Any) => {
                // Without a cached baseline the size delta is unknowable.
                // Treating the baseline as 0 used to book the whole file size
                // as a phantom +size on the first modify after daemon start
                // (or after a cache eviction) — e.g. touching a 5 GB file
                // recorded +5 GB. Observe now (seeds the baseline), emit only
                // when the previous size is known and actually changed.
                let old_size = state.last_known_size(path);
                state.file_size(path).and_then(|new_size| {
                    old_size.and_then(|old| {
                        let delta = (new_size as i64) - (old as i64);
                        if delta != 0 {
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
                })
            }
            EventKind::Remove(RemoveKind::File) | EventKind::Remove(RemoveKind::Any) => {
                state.remove(path).map(|size| DeltaEvent {
                    path: path.clone(),
                    delta_size: -(size as i64),
                    event_type: "delete".into(),
                    timestamp,
                    is_agg: false,
                    process_info: None,
                })
            }
            EventKind::Remove(RemoveKind::Folder) => {
                // A directory-level delete. Directories are never size-cached,
                // so this books only what is still cached *under* the folder —
                // see `remove_tree` for why that matters after coalescing.
                state.remove_tree(path).and_then(|total| {
                    if total > 0 {
                        Some(DeltaEvent {
                            path: path.clone(),
                            delta_size: -(total as i64),
                            event_type: "delete".into(),
                            timestamp,
                            is_agg: false,
                            process_info: None,
                        })
                    } else {
                        None
                    }
                })
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Any)) => {
                // FSEvents (macOS) and kqueue report each rename side as its
                // own RenameMode::Any event with no From/To pairing — the
                // paired modes below never fire there, so renames used to be
                // completely unaccounted (source stayed on the books, the
                // destination had no baseline for future modifies). Decide by
                // current existence: the side that is still there is the
                // destination (create), the side that vanished is the source
                // (remove). inotify/kqueue rename flows never emit Any for a
                // paired rename, so this cannot double-book on Linux.
                if path.exists() {
                    state.create_size(path).map(|size| DeltaEvent {
                        path: path.clone(),
                        delta_size: size as i64,
                        event_type: "create".into(),
                        timestamp,
                        is_agg: false,
                        process_info: None,
                    })
                } else {
                    state.remove(path).map(|size| DeltaEvent {
                        path: path.clone(),
                        delta_size: -(size as i64),
                        event_type: "delete".into(),
                        timestamp,
                        is_agg: false,
                        process_info: None,
                    })
                }
            }
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
                // create_size, not file_size: a rename landing on a duplicate
                // hard link must book nothing (the shared data is accounted
                // under the first link, same as an ordinary dup create).
                // Plain renames still book — the moved-away path no longer
                // exists, so the stale-mapping guard passes and the To half
                // cancels the From half.
                state.create_size(path).map(|size| DeltaEvent {
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

    // Dotfiles are watcher noise (.git internals, caches). This deliberately
    // includes .DS_Store — Finder rewrites it constantly and it would otherwise
    // generate endless modify events.
    name.starts_with('.')
        || name == "~"
        || name.ends_with(".swp")
        || name.ends_with(".swx")
        || name.ends_with("~")
}

/// True when any component between the watch root and the path is hidden.
/// The per-entry `is_ignored` only sees the file name, so churn *inside*
/// `.git` (pack files can reach hundreds of MB) was fully accounted even
/// though the comment claims .git internals are noise. Skipped when the
/// watch dir declares an explicit include glob — that is the user opting
/// into exactly those paths.
fn has_hidden_ancestor(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .map(|rel| {
            rel.components().any(|c| {
                let s = c.as_os_str().to_string_lossy();
                s.starts_with('.') || s == "~" || s.ends_with(".swp") || s.ends_with(".swx")
            })
        })
        .unwrap_or(false)
}

pub fn start_watcher(
    watch_dirs: Vec<WatchDir>,
    event_tx: mpsc::Sender<DeltaEvent>,
) -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Event, notify::Error>>();

        let mut watcher = match RecommendedWatcher::new(tx, Config::default()) {
            Ok(w) => w,
            // A panic here would silently kill the watch thread for the whole
            // daemon lifetime; degrade to a logged no-op watcher instead.
            Err(e) => {
                tracing::error!("failed to create watcher: {e}");
                return;
            }
        };

        for wd in &watch_dirs {
            let dir = wd.path();
            if dir.exists() {
                watcher
                    .watch(dir, RecursiveMode::Recursive)
                    .unwrap_or_else(|e| {
                        tracing::warn!("cannot watch {dir:?}: {e}");
                    });
                tracing::info!("watching {dir:?}");
                if wd.include.is_some() || wd.exclude.is_some() {
                    tracing::info!(
                        "  filters: include={:?}, exclude={:?}",
                        wd.include.as_ref().map(|g| g.glob()),
                        wd.exclude.as_ref().map(|g| g.glob()),
                    );
                }
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
                        let Some(wd) = match_watch_dir(&ev.path, &watch_dirs) else {
                            continue;
                        };
                        if !wd.matches(&ev.path) {
                            continue;
                        }
                        if wd.include.is_none() && has_hidden_ancestor(&wd.path, &ev.path) {
                            continue;
                        }
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
    use globset::GlobBuilder;
    use std::fs;
    use tempfile::tempdir;

    fn glob(pattern: &str) -> globset::Glob {
        GlobBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .unwrap()
    }

    #[test]
    fn test_match_watch_dir_longest_prefix_wins() {
        let dirs = vec![
            WatchDir {
                path: PathBuf::from("/home/user"),
                include: None,
                exclude: None,
            },
            WatchDir {
                path: PathBuf::from("/home/user/downloads"),
                include: Some(glob("*.pdf")),
                exclude: None,
            },
        ];

        // Under /home/user/downloads -> second watch dir (longer prefix)
        let matched = match_watch_dir(
            PathBuf::from("/home/user/downloads/report.pdf").as_path(),
            &dirs,
        );
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().path, PathBuf::from("/home/user/downloads"));
        assert!(matched.unwrap().include.is_some());

        // Under /home/user but not /home/user/downloads -> first watch dir
        let matched = match_watch_dir(PathBuf::from("/home/user/docs/file.txt").as_path(), &dirs);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().path, PathBuf::from("/home/user"));

        // Outside all watch dirs
        let matched = match_watch_dir(PathBuf::from("/other/file.txt").as_path(), &dirs);
        assert!(matched.is_none());
    }

    #[test]
    fn test_match_watch_dir_identical_paths() {
        let dirs = vec![
            WatchDir {
                path: PathBuf::from("/tmp"),
                include: None,
                exclude: None,
            },
            WatchDir {
                path: PathBuf::from("/tmp"),
                include: Some(glob("*.log")),
                exclude: None,
            },
        ];

        // Both have same prefix length; any match is valid
        let matched = match_watch_dir(PathBuf::from("/tmp/test.log").as_path(), &dirs);
        assert!(matched.is_some());
    }

    #[test]
    fn test_create_event() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("new.txt");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"hello").unwrap();

        let events = event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&file),
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
            std::slice::from_ref(&file),
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
            std::slice::from_ref(&file),
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

    /// Churn *inside* a hidden directory must be filtered too: `.git` pack
    /// files carry real sizes and the per-entry name check used to let them
    /// all through.
    #[test]
    fn test_hidden_ancestor_filters_git_internals() {
        let root = PathBuf::from("/watch");
        let git_internal = root.join(".git/objects/ab/cdef1234");
        assert!(has_hidden_ancestor(&root, &git_internal));

        // Normal nested paths are unaffected.
        assert!(!has_hidden_ancestor(&root, &root.join("src/lib/main.rs")));
        // Events outside the watch root: treat as not hidden (the watch-dir
        // match already rejects them).
        assert!(!has_hidden_ancestor(&root, &PathBuf::from("/elsewhere/x")));

        // A watch root that is itself hidden must not suppress everything.
        let hidden_root = PathBuf::from("/Users/lex/.config");
        assert!(!has_hidden_ancestor(
            &hidden_root,
            &hidden_root.join("argus/cache.bin")
        ));
    }

    /// .DS_Store is a dotfile and must be ignored: Finder rewrites it
    /// constantly, which would otherwise flood the delta log with noise.
    #[test]
    fn test_ignored_ds_store() {
        let dir = tempdir().unwrap();
        let file = dir.path().join(".DS_Store");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"finder junk").unwrap();

        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            &[file],
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 0);
    }

    /// Directory stat size is not user data; modify/create events carrying a
    /// directory path must not inject directory-entry sizes into accounting.
    #[test]
    fn test_directory_events_not_accounted() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).unwrap();

        let mut state = WatcherState::new();
        let timestamp = 1000;

        // Create(Any) on a directory: no event.
        let events = event_to_delta(
            &EventKind::Create(CreateKind::Any),
            std::slice::from_ref(&sub),
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 0);

        // Modify(Any) on a directory: no event, and nothing cached.
        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Any),
            std::slice::from_ref(&sub),
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 0);
        assert!(state.last_known_size(&sub).is_none());

        // Removing an untracked directory: no event (nothing cached).
        let events = event_to_delta(
            &EventKind::Remove(RemoveKind::Any),
            std::slice::from_ref(&sub),
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 0);
    }

    /// A modify event for a file the watcher has no baseline for (daemon just
    /// started, or cache evicted) must not book the whole file size as a
    /// phantom delta. The first observe only seeds the baseline.
    #[test]
    fn test_modify_without_baseline_records_nothing() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("big.bin");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        // 1 GB file touched without the watcher ever having seen it before.
        fs::write(&file, b"x").unwrap();
        let f = fs::File::create(&file).unwrap();
        f.set_len(1_000_000_000).unwrap();
        drop(f);

        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Any),
            std::slice::from_ref(&file),
            &mut state,
            timestamp,
        );
        assert!(events.is_empty());
        // Baseline is now seeded; a real growth is measured from it.
        use std::io::Write;
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        f.write_all(b"more data").unwrap();
        drop(f);
        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Any),
            std::slice::from_ref(&file),
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, 9);
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

    /// A second hard link to an already-accounted inode must not book the
    /// shared data a second time (the scanner dedups the same case). Its
    /// baseline must still be seeded so later modifies measure correctly.
    #[cfg(unix)]
    #[test]
    fn test_hardlink_create_not_double_counted() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.bin");
        fs::write(&a, b"12345").unwrap();

        let mut state = WatcherState::new();
        let events = event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&a),
            &mut state,
            1000,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, 5);

        let b = dir.path().join("b.bin");
        fs::hard_link(&a, &b).unwrap();
        let events = event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&b),
            &mut state,
            1001,
        );
        assert!(events.is_empty(), "duplicate link must book no delta");
        assert_eq!(state.last_known_size(&b), Some(5), "baseline seeded");

        // Modify on the new link measures against its own baseline.
        fs::write(&b, b"1234567890").unwrap();
        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Any),
            std::slice::from_ref(&b),
            &mut state,
            1002,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, 5);
    }

    /// Removing a duplicate hard link whose data still lives under the first
    /// link must book nothing: `cp -l big.bin link.bin && rm link.bin` frees
    /// no data, but the seeded baseline used to be subtracted in full — a
    /// phantom `-size` per dup-link removal.
    #[cfg(unix)]
    #[test]
    fn test_hardlink_copy_then_remove_books_nothing() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.bin");
        fs::write(&a, b"12345").unwrap();

        let mut state = WatcherState::new();
        assert_eq!(
            event_to_delta(
                &EventKind::Create(CreateKind::File),
                std::slice::from_ref(&a),
                &mut state,
                1000
            )
            .len(),
            1
        );

        let b = dir.path().join("b.bin");
        fs::hard_link(&a, &b).unwrap();
        assert!(event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&b),
            &mut state,
            1001
        )
        .is_empty());

        // rm b: no data freed, no delta.
        fs::remove_file(&b).unwrap();
        assert!(
            event_to_delta(
                &EventKind::Remove(RemoveKind::Any),
                std::slice::from_ref(&b),
                &mut state,
                1002
            )
            .is_empty(),
            "removing a dup link must book no negative delta"
        );

        // rm a: the last link is gone, the real -size lands.
        fs::remove_file(&a).unwrap();
        let events = event_to_delta(
            &EventKind::Remove(RemoveKind::Any),
            std::slice::from_ref(&a),
            &mut state,
            1003,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, -5);
    }

    /// Once a dup-link seed has real accounting (a modify booked a delta), a
    /// later remove negates like an ordinary file again — the seed mark is
    /// cleared by the observe.
    #[cfg(unix)]
    #[test]
    fn test_modified_seed_remove_books_negative() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.bin");
        fs::write(&a, b"12345").unwrap();

        let mut state = WatcherState::new();
        event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&a),
            &mut state,
            1000,
        );

        let b = dir.path().join("b.bin");
        fs::hard_link(&a, &b).unwrap();
        assert!(event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&b),
            &mut state,
            1001
        )
        .is_empty());

        // Modify through b: its churn is accounted at b, clearing the seed.
        fs::write(&b, b"1234567890").unwrap();
        assert_eq!(
            event_to_delta(
                &EventKind::Modify(ModifyKind::Any),
                std::slice::from_ref(&b),
                &mut state,
                1002
            )[0]
            .delta_size,
            5
        );

        fs::remove_file(&b).unwrap();
        let events = event_to_delta(
            &EventKind::Remove(RemoveKind::Any),
            std::slice::from_ref(&b),
            &mut state,
            1003,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, -10);
    }

    /// Renaming a duplicate link must not book the shared data again at the
    /// destination (RenameMode::To used to go through file_size, re-adding
    /// the full size the first link already accounted).
    #[cfg(unix)]
    #[test]
    fn test_rename_of_dup_link_books_nothing() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.bin");
        fs::write(&a, b"12345").unwrap();

        let mut state = WatcherState::new();
        event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&a),
            &mut state,
            1000,
        );

        let b = dir.path().join("b.bin");
        fs::hard_link(&a, &b).unwrap();
        assert!(event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&b),
            &mut state,
            1001
        )
        .is_empty());

        let c = dir.path().join("c.bin");
        fs::rename(&b, &c).unwrap();

        // From half: seeded path, nothing.
        assert!(event_to_delta(
            &EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            std::slice::from_ref(&b),
            &mut state,
            1002
        )
        .is_empty());

        // To half: still a dup of a, nothing.
        assert!(event_to_delta(
            &EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            std::slice::from_ref(&c),
            &mut state,
            1003
        )
        .is_empty());
    }

    /// FSEvents (macOS) and kqueue report each rename side as its own
    /// RenameMode::Any event. The vanished side must book the negative delta
    /// and the appearing side the positive one — both used to fall through
    /// to `_ => None`, so macOS renames were completely unaccounted.
    #[cfg(unix)]
    #[test]
    fn test_rename_any_books_both_sides() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.bin");
        fs::write(&a, b"12345").unwrap();

        let mut state = WatcherState::new();
        assert_eq!(
            event_to_delta(
                &EventKind::Create(CreateKind::File),
                std::slice::from_ref(&a),
                &mut state,
                1000
            )
            .len(),
            1
        );

        fs::rename(&a, dir.path().join("b.bin")).unwrap();

        // Source side (path gone): delete event.
        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
            std::slice::from_ref(&a),
            &mut state,
            1001,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "delete");
        assert_eq!(events[0].delta_size, -5);

        // Destination side (path exists): create event.
        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
            &[dir.path().join("b.bin")],
            &mut state,
            1002,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "create");
        assert_eq!(events[0].delta_size, 5);
    }

    /// A folder-level delete must book the still-cached sizes under the
    /// prefix in one delete event and clear them: FSEvents coalescing can
    /// surface a recursive delete as nothing but the folder event.
    #[test]
    fn test_folder_remove_books_cached_subtree_once() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("gone");
        fs::create_dir_all(sub.join("nested")).unwrap();
        let f1 = sub.join("a.bin");
        let f2 = sub.join("nested").join("b.bin");
        fs::write(&f1, b"12345").unwrap();
        fs::write(&f2, b"12").unwrap();

        let mut state = WatcherState::new();
        for path in [&f1, &f2] {
            event_to_delta(
                &EventKind::Create(CreateKind::File),
                std::slice::from_ref(path),
                &mut state,
                1000,
            );
        }

        fs::remove_dir_all(&sub).unwrap();
        let events = event_to_delta(
            &EventKind::Remove(RemoveKind::Folder),
            std::slice::from_ref(&sub),
            &mut state,
            1001,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, -7);
        assert!(state.last_known_size(&f1).is_none());
        assert!(state.last_known_size(&f2).is_none());

        // A second folder event (per-file removes already drained the cache)
        // must book nothing — this is the Linux inotify ordering.
        assert!(event_to_delta(
            &EventKind::Remove(RemoveKind::Folder),
            std::slice::from_ref(&sub),
            &mut state,
            1002
        )
        .is_empty());
    }

    /// Directory renames are unaccounted by design (dirs have no size
    /// baseline): a RenameMode::Any for a moved directory must book nothing
    /// on either side.
    #[cfg(unix)]
    #[test]
    fn test_rename_any_of_directory_books_nothing() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("d1");
        fs::create_dir_all(&sub).unwrap();

        let mut state = WatcherState::new();
        fs::rename(&sub, dir.path().join("d2")).unwrap();

        // Source side: gone, nothing was ever cached under it.
        assert!(event_to_delta(
            &EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
            std::slice::from_ref(&sub),
            &mut state,
            1000
        )
        .is_empty());

        // Destination side: exists but is a dir.
        assert!(event_to_delta(
            &EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
            &[dir.path().join("d2")],
            &mut state,
            1001
        )
        .is_empty());
    }

    /// A rename often surfaces as remove+create; the inode mapping then
    /// points at the vanished source path and must NOT suppress the create,
    /// or the rename would book a permanent -size.
    #[cfg(unix)]
    #[test]
    fn test_rename_as_remove_create_still_books() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.bin");
        fs::write(&a, b"12345").unwrap();

        let mut state = WatcherState::new();
        let events = event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&a),
            &mut state,
            1000,
        );
        assert_eq!(events.len(), 1);

        let b = dir.path().join("b.bin");
        fs::rename(&a, &b).unwrap();

        let events = event_to_delta(
            &EventKind::Remove(RemoveKind::Any),
            std::slice::from_ref(&a),
            &mut state,
            1001,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delta_size, -5);

        let events = event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&b),
            &mut state,
            1002,
        );
        assert_eq!(events.len(), 1, "rename's create half must book");
        assert_eq!(events[0].delta_size, 5);
    }

    /// Creating a symlink must not book the target's size (`stat` follows
    /// links; the scanner counts symlinks as size 0 too), and removing one
    /// must not subtract anything.
    #[cfg(unix)]
    #[test]
    fn test_symlink_create_not_accounted() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.bin");
        fs::write(&target, vec![0u8; 100]).unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();

        let mut state = WatcherState::new();
        let events = event_to_delta(
            &EventKind::Create(CreateKind::File),
            std::slice::from_ref(&link),
            &mut state,
            1000,
        );
        assert!(
            events.is_empty(),
            "symlink create must not book target size"
        );
        assert!(state.last_known_size(&link).is_none());

        let events = event_to_delta(
            &EventKind::Remove(RemoveKind::Any),
            &[link],
            &mut state,
            1001,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn test_size_cache_update() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sized.txt");

        let mut state = WatcherState::new();
        assert!(state.last_known_size(&path).is_none());

        fs::write(&path, b"100bytes!!").unwrap();
        assert_eq!(state.file_size(&path), Some(10));
        assert_eq!(state.last_known_size(&path), Some(10));

        fs::write(&path, b"100bytes!! and more").unwrap();
        assert_eq!(state.file_size(&path), Some(19));
        assert_eq!(state.last_known_size(&path), Some(19));

        state.remove(&path);
        assert!(state.last_known_size(&path).is_none());
    }

    #[test]
    fn test_cache_evicts_when_over_max() {
        let mut state = WatcherState::new();
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

    #[test]
    fn test_modify_any_kind() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("any.txt");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"hello").unwrap();
        state.file_size(&file);
        fs::write(&file, b"hello world").unwrap();

        let events = event_to_delta(
            &EventKind::Modify(ModifyKind::Any),
            std::slice::from_ref(&file),
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "modify");
        assert_eq!(events[0].delta_size, 6);
    }

    #[test]
    fn test_remove_any_kind() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("gone.txt");
        let mut state = WatcherState::new();
        let timestamp = 1000;

        fs::write(&file, b"bye").unwrap();
        state.file_size(&file);
        fs::remove_file(&file).unwrap();

        let events = event_to_delta(
            &EventKind::Remove(RemoveKind::Any),
            &[file],
            &mut state,
            timestamp,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "delete");
        assert_eq!(events[0].delta_size, -3);
    }
}
