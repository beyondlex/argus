# Argus

Argus is a personal desktop disk intelligence tool.

Scans are written to `~/.config/argus/argus.db`. The TUI reads scan history from SQLite to enrich directory sizes.

## CLI

```bash
# Scan the current tree root into SQLite
argus scan --path ~/Downloads

# List scan timestamps for one root
argus list-scans --path ~/Downloads

# List all scanned roots
argus list-scans
```

## TUI

- starts from the current working directory
- shows live filesystem navigation even when no scan data exists
- reloads the latest snapshot from SQLite when switching roots, so previously scanned directories regain size data
- shows `-` for ordinary unscanned directories, real sizes for files, and `...` for structural placeholder nodes
- shows directory sizes when SQLite scan history is available
- keeps the status bar compact with `[Tree]`, live scan progress, and the latest scan summary

## Docs

- `docs/requirements/index.md`
- `docs/plans/sqlite-storage-backend.md`
- `docs/notes/tui-current-behavior.md`
