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

- Documentation cleanup is in progress and the requirement docs have been normalized for Phase 1 wording, thresholds, safety rules, and scan behavior.
- `AGENTS.md` now includes explicit Clean Code principles, including DRY, KISS, YAGNI, and single-responsibility guidance.
- No Rust implementation work has started in this workspace yet.

## Active Work

### Docs

- [x] Clean up requirement document conflicts and phase boundaries.
- [x] Add Clean Code guidance to `AGENTS.md`.
- [ ] Decide whether to add a lightweight README pointer to this file.
- [ ] Keep `docs/requirements/index.md` in sync if new progress docs are added.

### Phase 1

- [ ] Initialize Cargo workspace.
- [ ] Implement `argus-core` data model.
- [ ] Implement scanner and unit tests.
- [ ] Implement diff engine and unit tests.
- [ ] Implement AI context generation stubs.
- [ ] Implement `argus-cli` commands.
- [ ] Add integration tests for `scan`, `diff`, and `explain`.

### Later Phases

- [ ] TUI shell and keybinding layer.
- [ ] Daemon and IPC protocol.
- [ ] AI API integration and token tracking.
- [ ] GUI client.

## Handoff Template

Use this block when continuing in a new session:

```text
Read AGENTS.md, docs/requirements/index.md, and AGENT_PROGRESS.md first.
Current focus: <one sentence>
Completed: <bullet list>
Next: <bullet list>
Blocked by: <optional blocker>
```

## Recent Completed Work

- Requirement docs now have a single Phase 1 entry point and fewer conflicting definitions.
- CLI threshold wording is aligned to `--threshold <SIZE>`.
- Scan cancellation is now documented as a hard stop in Phase 1 rather than a partial snapshot return.
