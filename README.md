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

## Configuration (`~/.config/argus/config.toml`)

```toml
[daemon]
watch_dirs = ["/Users/lex/Downloads", "/Users/lex/Desktop"]
debounce_seconds = 10
uds_path = "/tmp/argusd.sock"

# delta 事件保留天数（超过自动清理）
delta_retention_days = 30

# 目录级事件合并（减少 DB 条目数）
[daemon.consolidation]
sibling_threshold = 500   # 子级变更数超过此值则合并
interval_minutes = 60     # 后台合并间隔
```

## Docs

- `docs/requirements/index.md`
- `docs/plans/sqlite-storage-backend.md`
- `docs/notes/tui-current-behavior.md`