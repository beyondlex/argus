# Argus

Argus is a personal desktop disk intelligence tool.

Scans run in memory only — no persistence. The database (`~/.config/argus/argus.db`)
is reserved for future daemon delta data.

## CLI

```bash
# Scan a path and print summary
argus scan --path ~/Downloads
```

## TUI

- starts from the current working directory with a pure FS tree (dirs show `-`)
- press `s` to scan the current root — sizes appear in-memory for the session
- navigating away loses scan data for the old root; re-scan when needed
- keeps the status bar compact with `[Tree]`, live scan progress, and scan summary

## Docs

- `docs/requirements/index.md`
- `docs/plans/sqlite-storage-backend.md`
- `docs/notes/tui-current-behavior.md`
