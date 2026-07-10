# AGENTS.md — Argus Development Guide

Guide for AI agents contributing to Argus. Read fully before each session.

## 1. Project Overview

Argus = personal desktop disk intelligence tool. Rust, Cargo Workspace.

```
argus-core/  core logic (scan, diff, AI feature extraction)
argus-cli/   CLI client (quick validation, automated testing)
argusd/      daemon (background monitoring)
argus-tui/   TUI client (ratatui)
argus-gui/   GUI client (future)
```

Full requirements: `docs/requirements/index.md`

## 2. Core Architecture

### 2.1 Layered Decoupling

- **argus-core**: pure logic library. No client/UI deps. Reusable by any client.
- **argusd**: standalone process, communicates via UDS. Not embedded in any client.
- Clients (CLI/TUI/GUI) share no code between them — each depends on argus-core.

### 2.2 Dual-Mode Drive

Every client supports two modes:
- **Standalone**: calls argus-core directly, reads/writes local snapshot files. No daemon needed.
- **Client-Server**: connects to argusd via UDS for real-time incremental data.

Design Standalone first. Server mode is an enhancement.

TUI-specific rule:
- `scan_cache` is an in-process materialized cache, not the source of truth.
- When the current tree root changes, reload the latest snapshot for that root from SQLite before rebuilding the tree.
- This keeps the TUI aligned with daemon-driven or cross-session DB updates.

### 2.3 AI Is a Plugin, Not Core

- AI defaults off. All core features (scan, diff, browse, delete) work fully without AI.
- AI code gated behind feature flags. Zero impact on core compile size.

## 3. Development Discipline

### 3.1 TDD First

- **Write tests first**. Core algorithms (diff, tree merge, feature extraction) must have unit tests.
- Cover all edge cases: empty dirs, single file, deep nesting, symmetric/asymmetric diff.
- CLI commands validated via integration tests (`cargo test --test integration`).
- Use mock data, not real filesystem (except integration tests).

### 3.2 Docs <> Code Sync (Two-Way Chain)

**Forward sync** (requirements docs ← code): update requirements docs immediately after implementation changes.

| Code change | Sync target |
|-------------|-------------|
| Data structure change | `docs/requirements/08-data-model.md` |
| New/modified CLI command | `docs/requirements/05-ux-interaction.md` |
| Config change | `docs/requirements/04-configuration.md` |
| New dependency | Explain rationale in PR description |

**Backward sync** (code → user/developer docs): update docs after adding or changing features.

| Code change | Sync target |
|-------------|-------------|
| New CLI arg/command | `README.md` usage examples, `--help` output |
| Config change | `README.md` config section |
| New runtime dependency | `README.md` prerequisites |
| Build/test process change | `CONTRIBUTING.md` or root `README.md` |
| New module / public API refactor | Module-level doc comments (`///`), `lib.rs` module docs |

- Docs are not one-time work. Ask on every commit: **does this change need a doc update?**
- If no dedicated doc exists (e.g. `CONTRIBUTING.md`), at minimum update `README.md`.
- If the change is stable behavior that should feed README or wiki later, also update `docs/notes/tui-current-behavior.md` as the bridge note.

### 3.3 Progressive Architecture

- Don't over-engineer for the future. Current Phase solves current problems.
- If a decision affects future phases (e.g. data structure must support daemon IPC), add `// FUTURE:` comment.
- Design public APIs with extension points (e.g. `enum` over `bool`), but don't implement unused abstractions.

## 4. Code Standards

### 4.1 Rust Style

- `cargo fmt` + `cargo clippy` (0 warnings, must pass).
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).
- Public APIs need doc comments. Internal functions: comment *why*, not *what*.
- Errors: `thiserror`. No `unwrap()`/`expect()` (except tests and unreachable paths).
- Async: `tokio`. Sync code doesn't use async.

### 4.2 Naming

- Types: PascalCase (`FileNode`, `Snapshot`)
- Functions/methods: snake_case (`compare_trees`, `scan_path`)
- Modules: snake_case, short (`model`, `scanner`, `diff`)
- Error types: suffix `Error` (`ScanError`, `DiffError`)
- Traits: verbs (`Scanner`, `Differ`), not `-able` suffixes

### 4.3 File Organization

- One module per file. Split into submodules if >500 lines.
- Tests in `#[cfg(test)] mod tests { ... }` at end of module file (except integration tests in `tests/`).
- Public types re-exported from `lib.rs`. External code never references deep paths.

### 4.4 Maintainability

- Functions ≤50 lines. Split helpers otherwise.
- Max nesting depth 3. Use early returns or combinators.
- Core algorithms (e.g. `compare_trees`) need ASCII diagram or pseudocode comment.
- Run `cargo test && cargo clippy && cargo fmt --check` after every commit.

### 4.5 Clean Code Principles

- **DRY**: Don't duplicate business logic, constants, parsing rules, or error mapping. Extract a helper only after real duplication appears.
- **KISS**: Prefer straightforward concrete code over clever generic abstractions.
- **YAGNI**: Don't implement hooks, traits, factories, or feature switches before a current requirement needs them.
- **Single responsibility**: each function/module should have one clear reason to change.
- Prefer clear names over comments. Comments explain non-obvious *why*, not obvious *what*.
- Keep business rules centralized: protected paths, exit codes, config defaults, snapshot versions, and data format versions must have one authoritative definition.
- Make small, scoped changes. Avoid unrelated refactors while implementing a feature.
- Tests should verify observable behavior and edge cases, not private implementation details.

## 5. Test Strategy

| Layer | Tool | Coverage |
|-------|------|----------|
| Unit | `#[cfg(test)]` | Every core fn, mock data |
| Integration | `tests/` | CLI end-to-end |
| Snapshot | `insta` (optional) | Diff output format |
| Performance | `criterion` (optional) | Large dir scan |

Naming: `test_<function>_<scenario>` (e.g. `test_compare_trees_both_empty`).

## 6. AI Dev Principles

### 6.1 Code Generation

- Reuse existing patterns in the codebase first. Don't reinvent.
- For iterative changes to the same file, use `read + edit` not repeated `write`.
- Check `Cargo.toml` for existing alternatives before adding a new dep.
- Don't add abstractions not required by current needs (e.g. "factory pattern for future use").

### 6.2 Implementation Order (Phase 1)

```
model.rs → scanner.rs (w/ tests) → diff.rs (w/ tests)
  → ai_feature.rs (w/ tests) → cli/main.rs → integration tests
```

Run `cargo test` after each module before moving to the next.

### 6.3 Stuck?

- If interface design is unclear, check `docs/requirements/` specs.
- If a Rust compiler error persists, simplify (fewer generics, concrete types).
- If a lib behavior doesn't match expectation, read its docs.

## 7. Hard Constraints

- **NO** `std::process::Command` for shell commands (e.g. `rm -rf`). Use std lib or OS APIs.
- **NO** GUI/TUI deps in argus-core.
- **NO** `*` version in `Cargo.toml`.
- **NO** API keys, tokens, or credentials in code. Use env vars or config files.
- **MUST** handle all `Result` and `Option`. No `unwrap()` (except tests).
- **MUST** enable required features explicitly in `Cargo.toml`. No transitive feature reliance.

## 8. Collaboration with Architect

- Before implementing, output a brief technical proposal (design choice + rationale). Get confirmation first.
- Core algorithm and data structure changes need review.
- If requirements doc design proves impractical, propose alternatives instead of forcing it.
