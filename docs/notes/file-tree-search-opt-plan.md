# File Tree Search & Jump: Optimization Plan

## Problem

Three categories of issues in file tree search/jump:

1. **Correctness bugs** — jump skips, cursor desync, stale delta data
2. **Performance** — 90K files freeze on `n` key, directory expand, search input
3. **Architecture fragility** — implicit index coupling between `tree_lines` / `filtered_tree_lines` / `match_indices`

## Plan: 3 Phases

### Phase 1 — Correctness (P0, must fix)

| # | Bug | File | Fix |
|---|-----|------|-----|
| 1 | `jump_to_next_match` O(n²) fallback when delta filter hides target | `search.rs:56-75` | Add `path_to_tree_idx: HashMap<Vec<String>, usize>` for O(1) `tree_lines` lookup. Replace the two nested `.position()` calls. |
| 2 | `DeltaData` handler skips `update_tree_lines()` when `sort_mode == Delta` | `app.rs:307-316` | After `delta_cache = deltas`, if `sort_mode == Delta` call `update_tree_lines()` instead of just `refresh_filtered_lines()`. |
| 3 | `clear_filter_pane()` missing `request_delta_refresh()` | `app.rs:592-600` | Add `self.request_delta_refresh()` at end of method. `delta_cache` stays stale after clear. |
| 4 | `refresh_filtered_lines` prunes `match_indices` with stale `tree_idx` | `app.rs:543-551` | Remove the `match_indices.retain()` block. Move pruning logic to `recompute_matches()` only. `current_match` clamp stays. |

### Phase 2 — Performance (P1)

| # | Issue | Location | Fix |
|---|-------|----------|-----|
| 5 | Search input re-runs `recompute_matches()` O(90K) per keystroke | `handler/search.rs:11-12` | Defer: `Input` mode only stores char; `recompute_matches()` only on `Enter`. This breaks live search preview — accept as tradeoff for 90K perf. |
| 6 | `flatten_snapshot_tree` allocates `path.clone()` per node | `tree_ops.rs:274-328` | Pass `path: &mut Vec<String>`, push/pop in place, clone only when storing to `TreeLine.path`. Already mostly done — verify `path_key = path.clone()` in `size_for_path` call is the only remaining alloc. |
| 7 | `MAX_DIR_CHILDREN = 2000` too high | `tree_ops.rs:11` | Lower to `500`. 2000 concurrent children → 2K `flatten_snapshot_tree` calls + 2K sort items. 500 is safe boundary. |
| 8 | `refresh_filtered_lines` full O(90K) scan every delta filter change | `app.rs:519-551` | Cache per-line filter result. On delta filter value change, only re-evaluate lines whose delta changed. Requires `dirty` flag per line or incremental approach. |

### Phase 3 — Architecture (P2)

| # | Issue | Location | Fix |
|---|-------|----------|-----|
| 9 | `SearchMatch.tree_idx` is fragile — relies on DFS walk count matching `tree_lines` index | `types.rs:91-95` | Rename to `tree_line_idx: Option<usize>`. In `recompute_matches()`, set it to the actual `tree_lines` position (binary search by path). This decouples from `refresh_filtered_lines` index space. |
| 10 | `recompute_matches()` always runs even when `tree_lines` unchanged | `app.rs:327-370` | Add `tree_lines_version: u64` counter. Increment on any `tree_lines` mutation. `recompute_matches` skips if version unchanged. |
| 11 | `fuzzy_match_indices` heap-alloc per node (2x String) | `search.rs:209-219` | Replace `to_lowercase()` with `chars().map(|c| c.to_ascii_lowercase()).zip()` iteration. No allocation for case-insensitive compare. |

## Order

```
Phase 1 (#1 → #4 → #2 → #3) → Phase 2 (#7 → #5 → #6 → #8) → Phase 3 (#9 → #10 → #11)
```

Phase 1 first: bugs only. Each fix gets its own test. Run `cargo test` after each.

## Verification

- `cargo test` in argus-tui (unit + integration)
- `cargo clippy` — 0 warnings
- Manual: 90K file dir, search + n/N + delta filter + clear + expand/collapse cycle