# Argus Agent Progress

This is the living progress file for AI agents working on Argus.
Read this after `AGENTS.md` and the relevant requirements docs, then update it when a task is completed, blocked, or handed off.

## How To Use

- Update task status in place when work changes.
- Keep `Last updated` current.
- When starting a new session, read this file first to recover context quickly.
- When handing work off, record the exact next action and any blockers.

## Last Updated

- 2026-07-09

## Current State

- Phase 1 MVP is complete: `argus-core` (model/scanner/diff/ai_feature) + `argus-cli` (scan/diff/explain).
- 37 unit tests, clippy clean, fmt clean.
- Phase 2 TUI structural refactoring in progress: standalone mode file tree redesign.
- Design approved in `docs/plans/standalone-fs-navigation-refactor.md`.

## Active Work

### Phase 1

- [x] Initialize Cargo workspace.
- [x] Implement `argus-core` data model.
- [x] Implement scanner and unit tests.
- [x] Implement diff engine and unit tests.
- [x] Implement AI context generation stubs.
- [x] Implement `argus-cli` commands.
- [x] Manual acceptance testing (§4.1-4.5).
- [ ] Implement config.rs (§2.4) for ignore config loading.

### Phase 2 — Standalone FS Navigation Refactor

- [x] Design doc: `docs/plans/standalone-fs-navigation-refactor.md`
- [x] Updated: `02-architecture.md`, `03-core-features.md`, `04-configuration.md`
- [x] Updated: `05-ux-interaction.md`, `08-data-model.md`, `12-phase2-guide.md`
- [ ] `argus-core`: implement `list_dir()` + tests
- [ ] `argus-tui/app.rs`: new fields, scan_cache, rebuild_tree
- [ ] `argus-tui/handler.rs`: new navigation, s = scan tree root, no prompt
- [ ] `argus-tui/event.rs`: remove empty/scan prompts
- [ ] `argus-tui/file_tree.rs`: `"- "` rendering for unscanned dirs
- [ ] `argus-tui/filter_bar.rs`: scoped to current path hash
- [ ] `argus-tui/metadata.rs`: scan status display
- [ ] `argus-tui/config.rs`: BrowsingConfig
- [ ] `argus-tui/main.rs`: auto_scan_on_start
- [ ] Integration tests, manual acceptance

### Later Phases

- [ ] Daemon and IPC protocol.
- [ ] AI API integration and token tracking.
- [ ] GUI client.

## Recent Completed Work

- Phase 1 code: model.rs (FileNode/Snapshot/DiffNode/error types + parse_human_size).
- Phase 1 code: scanner.rs (ignore::WalkBuilder, hardlink dedup, cancel via AtomicBool, progress via mpsc).
- Phase 1 code: diff.rs (Tree Merge algorithm, threshold filter with 11 edge case tests).
- Phase 1 code: ai_feature.rs (extract_feature, generate_prompt, find_subtree).
- Phase 1 code: argus-cli (scan/diff/explain, --threshold/--format, exit code contract).
- All strings localized to English (errors, prompts, CLI output).
- .idea/ added to .gitignore.
