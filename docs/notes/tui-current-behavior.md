# Argus TUI Current Behavior Notes

This note captures the stable behavior of `argus-tui` as implemented now. It is intended as a bridge between the codebase, `README.md`, and any future wiki pages.

## Source Of Truth

- All scans are in-memory — snapshots are session-only, never persisted.
- `scan_cache` is a session-only cache populated by pressing `s`.
- The database (`~/.config/argus/argus.db`) is used for daemon delta events and the AI analysis cache (TUI reads/writes it for verdicts; scans themselves are never stored).
- Architecture: flat-mode (ncdu-like) directory browsing. Single directory level visible at a time.

## Navigation Model

- Flat directory view: shows only direct children of the current directory.
- `l` / `→`: enter selected directory (push to dir_stack, reload children).
- `h` / `←`: return to parent directory (pop from dir_stack, reload children).
- `H`: return to view_root (clear dir_stack).
- `u`: within tree → go to parent; at tree root → change view_root to filesystem parent.
- Navigation stack (`dir_stack`) tracks visited directories within the current tree root.

## Scanning

- Press `s` to scan the current directory (the directory being browsed, not necessarily view_root).
- Scan uses `jwalk` in serial mode (`Parallelism::Serial`), recursive, hidden files included (no `.gitignore` handling).
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
- A query in progress (typing or active) **survives background refreshes** —
  delta data arriving from the daemon, AI analysis completion, and delete
  completion all reload children without discarding it; match indices are
  recomputed against the fresh children.

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
| Multi-select (Space) + batch delete | ✅ |
| Delete (Trash / Permanent) | ✅ |
| Delta display (daemon mode) | ✅ |
| Time range filter (daemon mode) | ✅ |
| Delta filter (daemon mode) | ✅ |
| Delta detail popup (K) | ✅ |
| Go to Path (finder, `:Finder`) | ✅ |
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

## Cleanup / Uninstall / Brew Panels (C / P / U / B)

- `C` Clean（缓存/日志目标清理）、`P` Purge（构建产物发现）、`U` Uninstall（应用卸载+残留）、`B` Brew（按最近使用排序）。
- Clean 面板 `d` 切换 dry-run 标记，标记保持到再次按 `d`（j/k 移动光标不再重置它）：dry-run 下确认（Enter → y）只生成预览报告，**不触碰文件系统**；真实删除仅在非 dry-run 且确认后发生。
- 空格多选、Enter 触发确认、Esc/q 退出；`i` 展开目录明细（后台线程扫描，最多 200 行）。
- Brew 面板按 last_used 升序（never 最前）；卸载需要 y/N 确认。

## Good README / Wiki Targets

- README "TUI" overview: flat directory browsing, scan-on-demand, in-memory cache.
- Wiki "How scanning works": jwalk serial walker, scan_cache, session scope.
- Wiki "Search vs Filter": search highlights, delta filter hides non-matching.
- README/daemon notes: directory delta is subtree-wide coverage, not sum of visible leaf rows.
