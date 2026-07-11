# Argus TUI Current Behavior Notes

This note captures the stable behavior of `argus-tui` as implemented now. It is intended as a bridge between the codebase, `README.md`, and any future wiki pages.

## Source Of Truth

- Scans are in-memory only — no SQLite persistence.
- `scan_cache` is a session-only cache populated by pressing `s`.
- The tree always starts from the current working directory with pure FS structure.
- The database (`~/.config/argus/argus.db`) is reserved for future daemon delta data.

## Current Tree Behavior

- The tree always starts from the current working directory.
- The tree always reflects the current filesystem structure.
- Without a scan, ordinary directories show `-` and files show real size.
- Structural placeholder nodes show `...`.
- Symlinks are rendered distinctly from regular files.
- Switching roots with `u` or `.` resets to FS tree (no persisted scan data).

## Scan And Status Bar Behavior

- Pressing `s` scans the current tree root in-memory.
- Scan data is cached for the current session in `scan_cache`.
- Navigating away loses scan data for the old root; re-scan when needed.
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
- Whenever the current root changes, rebuild the tree from FS (no DB load).
- Any daemon-driven refresh should update the in-memory cache; the TUI can then materialize the latest state from `scan_cache`.

## Good README / Wiki Targets

- README "TUI" overview: current root, in-memory scan, tree size semantics, status bar behavior.
- Wiki "How scanning works": in-memory flow, `scan_cache`, session scope.
- Wiki "Troubleshooting": why a root shows no size before pressing `s`.
