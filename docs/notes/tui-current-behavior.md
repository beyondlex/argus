# Argus TUI Current Behavior Notes

This note captures the stable behavior of `argus-tui` as implemented now. It is intended as a bridge between the codebase, `README.md`, and any future wiki pages.

## Source Of Truth

- SQLite is the persistent store for scan history.
- `scan_cache` is an in-process materialized cache of the latest snapshot for a root path.
- `load_from_db()` refreshes the current root's cache from SQLite.
- `rebuild_tree()` and `show_normal_tree()` both reload the current root from SQLite before rebuilding the tree.

## Current Tree Behavior

- The tree always starts from the current working directory.
- The tree always reflects the current filesystem structure.
- Scan history only enriches node sizes; it never changes tree shape.
- When a directory has no scan data, ordinary directories show `-`.
- Structural placeholder nodes show `...`.
- Symlinks are rendered distinctly from regular files.
- Switching roots with `u` or `.` reloads the latest snapshot for that root if one exists in SQLite.

## Scan And Status Bar Behavior

- Pressing `s` scans the current tree root.
- Scan progress shows:
  - current path
  - `Size`
  - `Items`
  - `Took`
  - spinner
  - cancel hint
- After scan completion, the status bar shows the latest summary for the root:
  - path
  - total size
  - item count
  - duration
- The status bar keeps `[Tree]` as the focus label.

## UI Consistency Rules

- Tree size, metadata size, and scan summary size should use the same root snapshot size.
- Whenever the current root changes, the UI should reload from SQLite first, then rebuild the tree.
- Any daemon-driven refresh should update SQLite first; the TUI can then materialize the latest state from `scan_cache` / `load_from_db()`.

## Good README / Wiki Targets

- README "TUI" overview: current root, scan history, tree size semantics, status bar behavior.
- Wiki "How scanning works": SQLite-first flow, `scan_cache`, root reload behavior.
- Wiki "Troubleshooting": why a root may briefly show no size before the cache refresh completes.
