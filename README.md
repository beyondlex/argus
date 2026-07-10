# Argus

Argus is a personal desktop disk intelligence tool.

The current design direction is SQLite-first:

- scans are written to `~/.config/argus/argus.db`
- deltas are queried by `--path`, `--from`, and `--to`
- the TUI uses the current working directory as its tree root and reads scan history from SQLite

## CLI

```bash
# Scan the current tree root into SQLite
argus scan --path ~/Downloads

# Compare two timestamps for the current root
argus diff --path ~/Downloads --from 2026-06-01T00:00:00Z --to 2026-06-15T00:00:00Z

# List scan timestamps for one root
argus list-scans --path ~/Downloads

# List all scanned roots
argus list-scans
```

## TUI

- starts from the current working directory
- shows live filesystem navigation even when no scan data exists
- shows `-` for ordinary unscanned directories, real sizes for files, and `...` for structural placeholder nodes
- shows directory sizes when SQLite scan history is available
- uses the filter bar for time range and delta threshold selection

## Docs

- `docs/requirements/index.md`
- `docs/plans/sqlite-storage-backend.md`
