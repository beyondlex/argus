# Argus Agent Progress

Living progress file for AI agents working on Argus.
Read after `AGENTS.md` and relevant requirements docs, then update when task status changes.

## Last Updated

- 2026-10-05

## Current State

- Phase 1: complete
- Phase 2: complete (integration tests in `argus-tui/tests/integration.rs`)
- Phase 3: code complete (argusd, IPC, DB, TUI client); end-to-end daemon integration tests (Step 10) still pending
- Review rounds 1-13 complete (see `docs/notes/code-review-2026-09.md`); full suite: 408 tests, clippy --all-targets --all-features 0 warnings, fmt clean

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
- [x] Integration tests (state-logic level, `argus-tui/tests/integration.rs`); manual acceptance performed across review rounds

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
- Review round 8 (2026-09-30): fixed hung orphan-scan unit test (injectable home/apps), sandbox-container false orphans (dotted bundle ids), retention=0 purge, cleanup/uninstall scan escape, TUI config parse warning, AI-review protected-path gate + failed-delete accounting, delta-detail ghost socket, CJK/symbol display-width metrics, dead public API cleanup; open items #41-#45 recorded
- Review round 9 (2026-10-01): fixed stale-errno process-alive probe (single-instance guard), dir_size symlink-argument follow, `:time` range panic on rescaling chars, uninstall confirm ignoring the per-item leftover selection (core gains `uninstall_app_with_leftovers`), multi-select surviving view-root changes (wrong-path deletes), missing finder nav-history entry; open items #46-#51 recorded
- Review round 10 (2026-10-02): fixed multi-select surviving `b`/`f` cross-root nav steps (fifth root-change point), root-dir delete guard over-blocking same-named children (#47 landed), duplicate-hardlink removal/rename booking phantom deltas (daemon), binary Info.plist losing the bundle id (plutil fallback), AI prompt budget vs response cap split into `max_response_tokens` (#46 landed), CLI clean orphan section says display-only (#51 landed); open items #52-#53 recorded
- Review round 11 (2026-10-03): fixed AI error preview slicing mid-UTF-8-char (core), cleanup detail scan re-walking every subtree once per directory (single post-order pass, minutes→seconds on deep trees), AI-review scroll underflow on ≤4-row terminals, daemonize now chdir's to / (stale cwd pinned its volume); open items #54-#56 recorded (leftover fuzzy-match over-matching, classify_risk contains-boundary, search highlight offset on exotic lowercasing); doc-poste overview verified still accurate (no behavior-level changes this round)
- Review round 12 (2026-10-04): landed open items #54 (Application Support leftovers now match by exact name or bundle id — short names no longer claim unrelated dirs) and #56 (search highlight maps through per-char lowercase expansion); closed #55 as reasoned won't-fix (boundary tightening would make risk classification less conservative); fixed consolidation failures reporting via success-styled Info, uninstall confirm copy contradicting the remove-leftovers toggle, per-frame whole-state clones in the three cleanup panels, `find_artifacts` missing nested workspaces (depth-bounded walk, depth ≤ 4), `truncate_utf8` test not gated behind the `ai` feature (default-feature `cargo test -p argus-core` did not compile); new open items #57-#60 recorded; doc-poste argus overview synced (purge nesting + leftover matching)
- Review round 13 (2026-10-05): landed open items #57 (unit tests construct `App` without touching the real user DB), #58 (CLI selects resolve by index, not re-formatted strings) and #60 (delta-detail popup fills its row area, footer pct matches); fixed consolidation writing an unreachable junk agg row for the filesystem root (delete prefix `//` matched nothing, count over-reported), panel scans stuck on the scanning screen after a failure (Error never cleared `cleanup_state`/`uninstall_state.scanning`) with uninstall Confirm-phase Esc now returning to the app list plus a stale-leftover phase guard, `query_db_size` missing the WAL sidecar, nondeterministic AI-review delete order; new open items #61-#63 recorded; doc-poste argus overview verified still accurate (no behavior-level user-facing changes this round)
