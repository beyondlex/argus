# Argus Agent Progress

Living progress file for AI agents working on Argus.
Read after `AGENTS.md` and relevant requirements docs, then update when task status changes.

## Last Updated

- 2026-07-12

## Current State

- Phase 1: complete
- Phase 2: code complete, doc updated, integration tests pending
- Phase 3: code complete (argusd, IPC, DB, TUI client), integration tests pending

## Active Work

### Phase 1 — MVP

- [x] Initialize Cargo workspace.
- [x] Implement `argus-core` data model.
- [x] Implement scanner and unit tests.
- [x] Implement diff engine and unit tests.
- [x] Implement AI context generation stubs.
- [x] Implement `argus-cli` commands.
- [x] Manual acceptance testing (§4.1-4.5).
- [x] Config loading for ignore rules.

### Phase 2 — Standalone FS Navigation Refactor

- [x] Design doc: `docs/plans/standalone-fs-navigation-refactor.md`
- [x] Updated: `02-architecture.md`, `03-core-features.md`, `04-configuration.md`
- [x] Updated: `05-ux-interaction.md`, `08-data-model.md`, `12-phase2-guide.md`
- [x] `argus-core`: `list_dir()` + tests
- [x] `argus-tui/app.rs`: new fields, scan_cache, rebuild_tree
- [x] `argus-tui/handler.rs`: new navigation, s = scan tree root, no prompt
- [x] `argus-tui/event.rs`: remove empty/scan prompts
- [x] `argus-tui/file_tree.rs`: `"- "` rendering for unscanned dirs
- [x] `argus-tui/config.rs`: BrowsingConfig
- [x] `argus-tui/main.rs`: auto_scan_on_start
- [ ] Integration tests, manual acceptance

### Phase 3 — Daemon Automation

- [x] Design doc: `docs/plans/phase3-daemon-design.md`
- [x] Step 1: Data structures + DB schema (model.rs DeltaEvent/DeltaEntry, db.rs query API)
- [x] Step 2: IPC protocol types (argus-core/src/ipc.rs)
- [x] Step 3: `argusd` crate creation + main.rs skeleton
- [x] Step 4: watcher module (notify, size cache, hardlink dedup)
- [x] Step 5: debounce module (buffer, merge, delayed write)
- [x] Step 6: IPC Server (UDS listener + request dispatch)
- [x] Step 7: Daemon main flow (config, signal handling, graceful shutdown)
- [x] Step 8: TUI IPC Client (UDS connect, auto-detect, fallback)
- [x] Step 9: TUI delta overlay (delta column, time filter bar, detail popup)
- [ ] Step 10: Integration tests (end-to-end daemon, UDS, delta query)

## Recent Completed Work

- Phase 1: model/scanner/diff/ai_feature + cli (scan/diff/explain)
- Phase 2 (code): list_dir, scan_cache, rebuild_tree, navigation, "-" rendering, BrowsingConfig, auto_scan_on_start
- Phase 3 design: `docs/plans/phase3-daemon-design.md`
- Phase 3 (code): db.rs schema + query, ipc.rs protocol, argusd (watcher/debounce/ipc_server/config/retention), TUI ipc_client + delta overlay
- docs/requirements/index.md: added P3 reference
