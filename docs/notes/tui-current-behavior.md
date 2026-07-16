# Argus TUI Current Behavior Notes

This note captures the stable behavior of `argus-tui` as implemented now. It is intended as a bridge between the codebase, `README.md`, and any future wiki pages.

## Source Of Truth

- All scans are in-memory — no SQLite persistence.
- `scan_cache` is a session-only cache populated by pressing `s`.
- The database (`~/.config/argus/argus.db`) is reserved for future daemon delta data.
- Architecture: flat-mode (ncdu-like) directory browsing. Single directory level visible at a time.

## Navigation Model

- Flat directory view: shows only direct children of the current directory.
- `l` / `→`: enter selected directory (push to dir_stack, reload children).
- `h` / `←`: return to parent directory (pop from dir_stack, reload children).
- `H`: return to view_root (clear dir_stack).
- `u`: within tree → go to parent; at tree root → change view_root to filesystem parent.
- `w`: re-root view to current directory.
- Navigation stack (`dir_stack`) tracks visited directories within the current tree root.

## Scanning

- Press `s` to scan the current directory (the directory being browsed, not necessarily view_root).
- Scan uses `jwalk` (parallel walker, replaces old `ignore` scanner). Respects `.gitignore`.
- Progress shown in a centered popup: current path, file count, bytes, spinner, cancel hint.
- After scan completes, the snapshot is cached in `scan_cache` keyed by its path.
- Summary (total size, disk usage, file count, duration) appears in the **title bar**.
- If a subdirectory is scanned and then entered, the view switches to that scan's root.

## Data Display

### File Tree Columns (left to right)

| Column | Content |
|--------|---------|
| Name | entry name + `/` for directories |
| Delta | `+1.2 MB` / `-500 KB` / `-` (daemon mode only) |
| Percent | percentage of current directory's total disk usage |
| Size | `disk_usage` if scan data available, else `size` |

### Status Bar

Displays current directory stats:
- **Disk**: total disk usage (blocks × block size)
- **Apparent**: total logical size (bytes)
- **Items**: number of visible entries
- Error/info messages (color-coded: red = error, green = info)
- Filter/time range indicators (daemon mode)
- Sort mode indicator (Name / Size / Delta)

### Title Bar

Shows breadcrumb path for the current directory.

## Search Behavior

- `/` enters search mode. Type query, press `Enter` to activate.
- Search highlights matching characters in entry names; **non-matching items stay visible**.
- `n` / `N` cycle through match indices (within `search_match_indices`).
- `Esc` clears search; `Enter` (in active mode) re-edits the query.
- Search is constrained to the current directory's children only (O(C) not O(N)).

## Theme

- Semantic `ColorTheme` with light/dark auto-detection (`terminal-light` crate).
- `color_scheme` config: `"system"` (default), `"light"`, `"dark"`.
- Full set: `text`, `accent`, `success`, `danger`, `warning`, `hidden`, `text_secondary`, `text_tertiary`, `text_highlight`, `selected_bg`, `selection_fg`, `focus_fg`, `match_bg`, `search_match_selected_bg`, `border_unfocused`, `bg`.

## Supported Features

| Feature | Status |
|---------|--------|
| Flat directory browsing | ✅ |
| Full scan (jwalk) | ✅ |
| Lazy directory listing (list_dir) | ✅ |
| Sort: Name / Size (disk_usage) / Delta | ✅ |
| Hidden file toggle (.) | ✅ |
| Search with highlight (no hide) | ✅ |
| Multi-select (Tab) + batch delete | ✅ |
| Delete (Trash / Permanent) | ✅ |
| Delta display (daemon mode) | ✅ |
| Time range filter (daemon mode) | ✅ |
| Delta filter (daemon mode) | ✅ |
| Delta detail popup (K) | ✅ |
| Go to Path (finder, Ctrl-P) | ✅ |
| Color theme (light/dark auto) | ✅ |
| Command mode (:) | ✅ |
| Status messages (error/info) | ✅ |
| Disk Usage + Apparent Size tracking | ✅ |
| Scan summary in title bar | ✅ |
| Daemon connection status in header | ✅ |

## UI Consistency Rules

- Disk Usage, Apparent Size, and scan summary use the same root snapshot data.
- Whenever the current root changes, rebuild the tree from `scan_cache` or `list_dir`.
- Delta display must treat `is_agg` rows as subtree coverage, not as extra child rows.
- Search does not filter; non-matching entries remain visible.

## Good README / Wiki Targets

- README "TUI" overview: flat directory browsing, scan-on-demand, in-memory cache.
- Wiki "How scanning works": jwalk parallel walker, scan_cache, session scope.
- Wiki "Search vs Filter": search highlights, delta filter hides non-matching.
- README/daemon notes: directory delta is subtree-wide coverage, not sum of visible leaf rows.
