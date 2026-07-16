# Standalone Mode Refactoring: File Tree as First-Class Navigation

## Status: Approved

This document supersedes the Phase 2 standalone mode design in `12-phase2-guide.md`.
See that doc for implementation checklist updates.
The newer `file-tree-size-design.md` is the source of truth for size / placeholder / delta
semantics. Treat this plan as historical for navigation flow only.

## 1. Motivation

Previously, TUI startup depended on snapshots: no snapshots = blank screen with "press s to scan".
This was a poor first-run experience and forced the user to commit to scanning a directory before
they could see or navigate anything.

**New model**: The file tree is always present. Its root is the user's current working directory (cwd)
when `argus-tui` is launched. Size and delta data are **optional metadata** attached to tree nodes.
A node that has been scanned shows its size/delta; a node not yet scanned shows `"-"` for directory
size and the real file size for files (since single-file `stat` is cheap). Structural placeholder
nodes from shallow scans show `"..."`.

## 2. Core Principle — HARD RULE

The TUI always shows a browsable filesystem tree. Scanning is an enhancement, not a prerequisite.

**The tree always reflects the current filesystem state.** Scan records (historical snapshots)
overlay size data, and diff data overlays delta values. They NEVER change the tree structure —
no node appears or disappears based on filter state. Expanding/collapsing directories works
identically regardless of whether a time filter is active.

```
┌─────────────────────────────────────────────────────────┐
│  Always:  FS tree (lazy-listed from disk)               │
│  Optional: size/delta data (from scan cache / snapshots) │
└─────────────────────────────────────────────────────────┘
```

## 3. Two Data Layers

### Layer 1: FS Listing (always available)

- Read directory entries from disk on demand (one level at a time)
- Files: real size from `metadata().len()`
- Directories: size starts at `0` and is later overlaid from scan history when available
- Directories remain expandable in the live filesystem tree

### Layer 2: Scan Data (optional, cached)

- Stored as SQLite-backed snapshots materialized into `scan_cache`
- On startup, scan history is loaded into `scan_cache: HashMap<PathBuf, Snapshot>`
- When viewing a directory, check cache first:
  - **Cache hit**: keep the current FS tree and overlay cached sizes onto matching nodes
  - **Cache miss**: render FS listing (files have real size, dirs show `"-"`)
- Same path with ≥2 snapshots enables filter bar diff
- Scan cache is keyed by **absolute path** of the scanned root

## 4. Startup Flow

```
open TUI
  ├── load scan history into scan_cache
  ├── determine cwd (std::env::current_dir())
  ├── check scan_cache for cwd
  │     ├── hit → render FS tree for cwd + size overlay
  │     └── miss → list_dir(cwd) → render shallow FS tree
  │
  └── if config.browsing.auto_scan_on_start:
        start_scan(cwd) in background (scanning state visible in status bar)
```

## 5. Navigation

### Keys

| Key | Action |
|-----|--------|
| `j`/`k` | Move cursor up/down |
| `l`/`Right`/`Enter` | If cursor on collapsed dir: expand it (check scan cache first) |
| `h`/`Left`/`Backspace` | If cursor on root dir: navigate up (change tree root to parent). Otherwise: collapse or move to parent node |
| `s` | **Scan current tree root** (no prompt dialog) |
| `o` | Toggle sort mode |
| `d` | Delete (with confirmation) |
| `Tab` | Toggle focus between Tree and FilterBar |
| `?` | Help |
| `q`/`Esc` | Quit/cancel |

### Expand Semantics

When user presses `l` on a directory:

1. Check `scan_cache` for the directory's absolute path
2. **Cache hit**: keep the FS tree and fill size data from cache
3. **Cache miss**: call `list_dir(path)` to get one level of FS entries, inline them into the current tree

### Navigate Up Semantics

When user presses `h` on the root node:

1. Change `view_root_path` to parent directory
2. Rebuild tree from `view_root_path`:
   - Check scan cache for new path
   - If hit: FS tree + size overlay
   - If miss: list_dir

### Key Change: `s` No Longer Opens a Prompt

Previously, `s` opened an overlay with a path input box. Now `s` immediately:

1. Determines the current tree root path (`view_root_path`)
2. Spawns a `tokio` task calling `argus_core::scan_path(view_root_path, ...)`
3. On completion: saves snapshot + updates `scan_cache` + rebuilds tree
4. Status bar shows progress during scan

## 6. Data Flow

### App State Changes

```rust
pub struct App {
    // ... existing fields ...

    // NEW: tree root path (always Some, initialized to cwd)
    pub view_root_path: PathBuf,

    // NEW: scan cache (path → snapshot, for scanned dirs)
    pub scan_cache: HashMap<PathBuf, Snapshot>,

    // NEW: available_snapshots scoped to view_root_path's hash
    pub available_snapshots: Vec<SnapshotInfo>,

    // NEW: diff_lookup overlays delta onto the FS tree
    pub diff_lookup: HashMap<Vec<String>, (u64, i64)>,

    // The tree itself is still kept in memory as the current FS view.
    pub tree_root: Option<TreeNode>,
    pub tree_lines: Vec<TreeLine>,

    // REMOVED: scan_prompt_open / scan_path_input (s now scans tree root)
}
```

### TreeLine Changes

```rust
pub struct TreeLine {
    pub depth: usize,
    pub node: TreeNode,
    pub expanded: bool,
    pub has_scan_data: bool,  // whether this dir has scanned size data
    pub delta: i64,           // overlay delta from the current diff query
}
```

### Render Behavior

| Node Type | Has Scan Data? | Size Display | Delta Display |
|-----------|----------------|-------------|---------------|
| File | — | Real file size (always) | If in diff mode |
| Directory | Yes | Aggregated scanned size | If in diff mode |
| Directory | No | `"-"` (gray) | N/A |
| Structural placeholder | No | `"..."` (gray) | N/A |

## 7. Scan Cache

On startup, scan history is loaded from the SQLite database (`~/.config/argus/argus.db`):

```rust
fn load_from_db(&mut self) {
    let conn = open_db(&self.db_path)?;
    let scans = query_scan_timestamps(&conn, &self.view_root_path);
    self.available_snapshots = scans.into_iter().map(|(id, ts, size, files)| {
        SnapshotInfo { scan_id: id, timestamp: ts, total_size: size, total_files: files }
    }).collect();

    if let Ok(snapshot) = rebuild_snapshot(&conn, &self.view_root_path) {
        self.scan_cache.insert(self.view_root_path.clone(), snapshot);
    }
}
```

The latest snapshot for each root path is materialized into `scan_cache` for size display.
The filter bar lists all available timestamps for the current `view_root_path` from SQLite.

## 8. Config Changes

New `[browsing]` group in `config.toml`:

```toml
[browsing]
# When true, scan the current working directory on startup.
# When false, show FS listing (no sizes for dirs) until user presses s.
auto_scan_on_start = false
```

## 9. argus-core Changes

### New API: `list_dir`

```rust
/// List one level of a directory. Returns a FileNode with children populated
/// from the directory's immediate entries.
///
/// - Files: size = metadata().len() (real file size)
/// - Directories: size = 0 until a scan overlay is available
/// - Directory children are shallow live entries; deeper structure is loaded on demand
///
/// Errors: PathNotFound, PermissionDenied, Io
pub fn list_dir(path: &Path) -> Result<FileNode, ScanError>
```

Exported from `lib.rs`:

```rust
pub use scanner::list_dir;
```

## 10. Affected Files

| File | Change |
|------|--------|
| `argus-core/src/scanner.rs` | Add `list_dir()` function + tests |
| `argus-core/src/lib.rs` | Export `list_dir` |
| `argus-tui/src/app.rs` | Keep FS tree state, add `view_root_path`, `scan_cache`, `db_path`; new methods: `load_from_db`, `rebuild_tree` |
| `argus-tui/src/handler.rs` | `s` writes scan to SQLite; expand uses `list_dir` + size overlay |
| `argus-tui/src/event.rs` | Remove `render_empty_prompt`, `render_scan_prompt`; filter bar scoped to current path |
| `argus-tui/src/components/file_tree.rs` | Handle `"-"` and `"..."` rendering |
| `argus-tui/src/components/filter_bar.rs` | Accept `available_snapshots` from SQLite |
| `argus-tui/src/components/metadata.rs` | Show scan status and current node size semantics |
| `argus-tui/src/config.rs` | Add `BrowsingConfig` struct |
| `argus-tui/src/main.rs` | Pass `db_path` to App; call `load_from_db()` |

## 11. Implementation Order

```
 1. argus-core:   list_dir() + tests
 2. cargo test
 3. argus-tui/app.rs:   tree root state, load_from_db(), rebuild_tree()
 4. argus-tui/handler.rs:  new navigation (h on root = up, l = expand, s = scan tree root)
 5. argus-tui/event.rs:    remove empty/scan prompts, update filter bar data source
 6. argus-tui/file_tree.rs:   "-" rendering for unscanned dirs
 7. argus-tui/filter_bar.rs:  scoped to current path
 8. argus-tui/metadata.rs:    scan status display
 9. argus-tui/config.rs:  BrowsingConfig
10. argus-tui/main.rs:   auto_scan_on_start logic
11. cargo build + clippy + test
```

## 12. Unchanged Parts

| Module | Reason |
|--------|--------|
| `model.rs` | FileNode/Snapshot/DiffNode structure unchanged |
| `diff.rs` | compare_trees/filter_by_threshold unchanged |
| `util.rs` | format_size/format_delta unchanged |
| SQLite schema | `scan_events` / `path_records` (see sqlite-storage-backend.md) |
| Trash-based delete | Unchanged |
| Keybinding config schema | Unchanged (s still maps to scan) |
| Sort modes | Unchanged |
