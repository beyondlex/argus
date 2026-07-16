# Archive — Outdated / Superseded Documents

Documents here are preserved for reference but **not** current. Do not rely on them for implementation decisions.

## Rationale

| Reason | Files |
|--------|-------|
| **Old data model** (enum `FileNode`, `name: String`, per-node `children` HashMap, JSON snapshots) — data model has since been rewritten to compact arena + CSR layout | `sqlite-storage-backend*` |
| **Phase complete** — implementation guide for a finished phase; data model details no longer match current code | `10-phase1-guide.md`, `12-phase2-guide.md` |
| **Fully implemented** — plan checked off, no remaining actionable items | `performance-optimizations.md`, `standalone-fs-navigation-refactor.md` |

## Current Replacements

| Archived | Replaced by |
|----------|-------------|
| `10-phase1-guide.md` | Current core model: `argus-core/src/model.rs`, `scanner.rs` |
| `12-phase2-guide.md` | Current TUI: `argus-tui/src/`, `docs/notes/tui-current-behavior.md`, `docs/flat-mode-design.md` |
| `performance-optimizations.md` | All items implemented (P12 deferred) |
| `standalone-fs-navigation-refactor.md` | Navigation flow in `docs/plans/file-tree-size-design.md` |
| `sqlite-storage-backend.md` | SQLite delta store lives in `argusd/` and `argus-core/src/db/` |
