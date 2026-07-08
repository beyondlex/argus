# Argus Agent Progress

This is the living progress file for AI agents working on Argus.
Read this after `AGENTS.md` and the relevant requirements docs, then update it when a task is completed, blocked, or handed off.

## How To Use

- Update task status in place when work changes.
- Keep `Last updated` current.
- When starting a new session, read this file first to recover context quickly.
- When handing work off, record the exact next action and any blockers.

## Last Updated

- 2026-07-08

## Current State

- Phase 1 MVP is complete: `argus-core` (model/scanner/diff/ai_feature) + `argus-cli` (scan/diff/explain).
- 37 unit tests, clippy clean, fmt clean.
- All core features (scan, diff, AI prompt gen) work without AI being required.
- Workspace: argus-core + argus-cli, Cargo workspace with aligned deps.

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

### Later Phases

- [ ] TUI shell and keybinding layer.
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
