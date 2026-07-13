# File Tree Search & Jump: Implementation Guide

## Overview

Phase 1 (correctness bugs) is the priority. Each fix below includes:
- Why it breaks
- What changes
- Test strategy
- Backward compat check

---

## Fix #1: `jump_to_next_match` O(n²) fallback → HashMap O(1)

### Why It Breaks

File: `argus-tui/src/search.rs:56-75`

When delta filter hides the target match, fallback loops over ALL `match_indices`:

```rust
for offset in 1..total {
    let try_path = &app.match_indices[try_idx].path;
    if let Some(pos) = app.tree_lines.iter().position(|line| line.path == *try_path) {
        if let Some(cursor_pos) = app.filtered_tree_lines.iter().position(|&i| i == pos) {
```

Two nested `.position()` = O(n) + O(m) per iteration. 90K files + 1K matches → 90M path comparisons worst case.

### What Changes

1. Add `path_to_tree_idx: HashMap<Vec<String>, usize>` to `App` struct — built once in `update_tree_lines()` and `expand_path_in_tree()` after `tree_lines` is set.
2. Replace fallback loops with O(1) HashMap lookup.
3. Wipe HashMap on every `tree_lines` reassignment (in `update_tree_lines`).

### Code: `app.rs` — Add field

```rust
pub path_to_tree_idx: HashMap<Vec<String>, usize>,
```

Initialize in `new()`:
```rust
path_to_tree_idx: HashMap::new(),
```

### Code: `app.rs` — Build in `update_tree_lines`

After `self.tree_lines = lines;` (line 242), add:

```rust
self.path_to_tree_idx = self.tree_lines.iter().enumerate().map(|(i, l)| (l.path.clone(), i)).collect();
```

Also after `expand_path_in_tree` splice (line 450), rebuild same map.

### Code: `app.rs` — Rebuild in `expand_path_in_tree`

After `self.tree_lines.splice(pos + 1..pos + 1, new_lines);`, add:

```rust
self.path_to_tree_idx = self.tree_lines.iter().enumerate().map(|(i, l)| (l.path.clone(), i)).collect();
```

### Code: `search.rs` — Replace fallback

Replace lines 56-75 with:

```rust
// Fast path: target match visible in filtered view
if let Some(&pos) = app.path_to_tree_idx.get(&target_path) {
    if let Some(cursor_pos) = app.filtered_tree_lines.iter().position(|&i| i == pos) {
        app.cursor = cursor_pos;
        return;
    }
}

// Fallback: find next visible match using HashMap O(1) per lookup
let total = app.match_indices.len();
for offset in 1..total {
    let try_idx = if delta >= 0 {
        (new_idx + offset) % total
    } else {
        (new_idx + total - offset) % total
    };
    let try_path = &app.match_indices[try_idx].path;
    if let Some(&pos) = app.path_to_tree_idx.get(try_path) {
        if let Some(cursor_pos) = app.filtered_tree_lines.iter().position(|&i| i == pos) {
            app.current_match = try_idx;
            app.cursor = cursor_pos;
            return;
        }
    }
}
```

Also update lines 45-53 (fast path) to use HashMap instead of `.position()`.

### Tests

Add to `search.rs` tests:

1. `test_jump_fallback_with_delta_filter` — set up tree with 2 matches, enable delta filter that hides the first target match, call `jump_to_next_match`, verify it lands on the visible match.
2. `test_jump_path_to_tree_idx_after_expand` — expand dir then jump, verify HashMap stays in sync.

### Backward Compat

- `path_to_tree_idx` is internal cache. External API unchanged.
- All existing `tree_lines.iter().position(|line| line.path == path)` calls remain in `expand_path_in_tree` and `collapse_or_navigate_up` — those are not in hot paths.

---

## Fix #2: `DeltaData` handler misses `update_tree_lines()` when `sort_mode == Delta`

### Why It Breaks

`app.rs:307-316`:

```rust
AppMessage::DeltaData(deltas) => {
    self.delta_pending = false;
    self.delta_cache = deltas;
    self.refresh_filtered_lines();  // does NOT rebuild tree_lines!
}
```

When `sort_mode == Delta`, `tree_lines` is sorted by delta values. New delta data arrives → old sort order is stale. `refresh_filtered_lines` only rebuilds the filtered view index, not the tree itself. User sees lines sorted by old delta values.

### What Changes

Check `sort_mode` before deciding which refresh to call:

```rust
AppMessage::DeltaData(deltas) => {
    let t0 = Instant::now();
    self.delta_pending = false;
    self.delta_cache = deltas;
    if self.sort_mode == SortMode::Delta {
        self.update_tree_lines();  // rebuild tree_lines with new delta order
    } else {
        self.refresh_filtered_lines();  // tree_lines unchanged, just re-filter
    }
    log_msg(/* same */);
}
```

### Why This Is Safe

- `update_tree_lines()` internally calls `refresh_filtered_lines()` + `recompute_matches()`. So delta sort refresh gets everything.
- Non-delta sort modes only need filter refresh — avoids unnecessary O(90K) tree rebuild.

### Tests

Add to `app.rs` tests:

1. `test_delta_data_triggers_tree_rebuild_when_delta_sort` — set `sort_mode = SortMode::Delta`, `delta_cache` empty, `tree_lines` with 3 lines. Send `DeltaData` via message handler. Verify `tree_lines` changed.
2. `test_delta_data_skips_tree_rebuild_when_size_sort` — same setup but `sort_mode = SortMode::Size`. Verify `tree_lines.len()` changed only by `update_tree_lines`.

### Backward Compat

- All callers of `handle_message` unchanged. Only internal branching added.
- `CmdSort` (`command.rs:206`) already calls `update_tree_lines()` after toggling sort mode. No conflict.

---

## Fix #3: `clear_filter_pane` missing `request_delta_refresh()`

### Why It Breaks

`app.rs:592-600`:

```rust
pub fn clear_filter_pane(&mut self) {
    self.delta_filter_active = false;
    self.delta_filter_value = 100;
    self.delta_filter_unit = 1;
    self.set_time_preset(0);      // time range reset to 1h
    self.focus = Focus::Tree;
    self.refresh_filtered_lines();
    self.recompute_matches();
    // BUG: no request_delta_refresh()!
}
```

`set_time_preset(0)` resets `time_from`/`time_to` to last 1h, but `delta_cache` still holds data from the old time range (e.g. 7d). Subsequent delta filter and sort are based on stale cache.

Compare with `adjust_filter_focus` (`handler/filter.rs:108`) which correctly calls `request_delta_refresh()` after `set_time_preset(next)`.

### What Changes

Add one line:

```rust
pub fn clear_filter_pane(&mut self) {
    self.delta_filter_active = false;
    self.delta_filter_value = 100;
    self.delta_filter_unit = 1;
    self.set_time_preset(0);
    self.focus = Focus::Tree;
    self.refresh_filtered_lines();
    self.recompute_matches();
    if self.server_mode {
        self.request_delta_refresh();  // fetch fresh delta data for new time range
    }
}
```

Guard with `server_mode` check because `request_delta_refresh()` spawns a tokio task that connects to daemon — no-op in standalone mode but safe.

### Tests

1. `test_clear_filter_pane_requests_delta_refresh` — set `server_mode = true`, `delta_pending = false`, `time_preset = 5 (7d)`. Call `clear_filter_pane()`. Verify `delta_pending` is now `true` (means `request_delta_refresh` was called).

---

## Fix #4: `refresh_filtered_lines` stop pruning `match_indices`

### Why It Breaks

`app.rs:543-551`:

```rust
if !self.match_indices.is_empty() {
    let visible: HashSet<usize> = self.filtered_tree_lines.iter().copied().collect();
    self.match_indices.retain(|m| m.tree_idx.map_or(true, |idx| visible.contains(&idx)));
    self.current_match = self.current_match.min(self.match_indices.len().saturating_sub(1));
}
```

`match_indices` uses `tree_idx` which is a DFS walk counter set during `recompute_matches()`. It's **not** a `tree_lines` index. There are two problems:

1. **`tree_idx` is the walk index on the **filtered** tree** — it counts only visible nodes. When `refresh_filtered_lines` removes lines visually, this walk index still refers to positions in the original DFS walk. The prune is comparing apples to oranges.

2. **Pruning removes matches permanently** — after pruning, the removed matches are gone even if the filter changes back. The user must re-enter search to find them again.

**But wait** — looking at the actual code path: `tree_idx` in `collect_matches_in_order` is set to `visible_count`, which increments only when `is_visible` is true. So `tree_idx` is actually the position in the **filtered** tree (not `tree_lines` index). And `filtered_tree_lines` contains `tree_lines` indices. So the comparison `visible.contains(&idx)` where `idx` is a `tree_idx` (filtered position) and `visible` is a set of `tree_lines` indices is indeed comparing mismatched index spaces.

This means: after `refresh_filtered_lines` with `delta_filter_active`, **all** matches with a `tree_idx` get removed because no `tree_idx` value equals a `tree_lines` index.

The prune logic is fundamentally broken. Remove it entirely.

### What Changes

Delete lines 543-551 (the match_indices pruning block). Keep the cursor clamp:

```rust
pub fn refresh_filtered_lines(&mut self) {
    if !self.delta_filter_active {
        self.filtered_tree_lines = (0..self.tree_lines.len()).collect();
    } else {
        // ... existing filter logic ...
    }
    if self.cursor >= self.filtered_tree_lines.len() && !self.filtered_tree_lines.is_empty() {
        self.cursor = self.filtered_tree_lines.len() - 1;
    }
    // DELETED: match_indices pruning block
}
```

### Tests

Update existing test `test_refresh_filtered_lines_keeps_hidden_matches` (`search.rs:464-499`) — it was already testing that hidden matches survive `refresh_filtered_lines`. After this fix, the test still passes but now proves that:

- Hidden matches (in collapsed subtree, `tree_idx=None`) survive (already worked).
- Visible matches (`tree_idx=Some(...)`) also survive (was broken, now fixed).

Add:

1. `test_refresh_filtered_lines_keeps_all_matches` — set up tree with 2 visible matches, enable delta filter that hides one, call `refresh_filtered_lines`, verify all 2 matches still in `match_indices`.

### Backward Compat

- Safe. Only removes incorrect pruning.
- `current_match` clamp stays — prevents out-of-bounds if `match_indices` was already small.
- After this fix, `current_match` may point to a match hidden by delta filter. That's fine — `jump_to_next_match` already handles this via the fallback loop.

---

## Fix #7: MAX_DIR_CHILDREN 2000 → 500

### Why It Matters

`tree_ops.rs:11`: Currently `pub(crate) const MAX_DIR_CHILDREN: usize = 2000;`

When a dir with 2000 children is expanded:
- `flatten_snapshot_tree` recurses into 2000 children
- `sort_children_snapshot` sorts 2000 items
- Each child constructs `child_path: Vec<String>` (heap alloc per child)
- 2000 `size_for_path` calls build PathBuf each

On 90K file tree with one mega-dir (e.g., `node_modules`), expanding that single dir causes visible freeze.

### What Changes

```rust
pub(crate) const MAX_DIR_CHILDREN: usize = 500;
```

### Impact

- Dirs with >500 children show as collapsed with no expand arrow (already handled: `expand_path_in_tree` returns false, `flatten_snapshot_tree` skips rendering children).
- User can still `list_dir` (filesystem fallback) but won't get inline expand.
- Most mega-dirs are build artifacts/cache — user rarely needs inline expand for those.

---

## Fix #5: Search input defer `recompute_matches` to Enter

### Why

`handler/search.rs:11-12`: Every keystroke in Input mode calls `recompute_matches()` → O(90K) tree walk. At 10 chars = 900K node traversals. CPU 100%.

Additionally, each `collect_matches_in_order` call builds `path_to_walk_idx` hashmap for all 90K nodes — wasted since the user hasn't finished typing.

### What Changes

```rust
// handler/search.rs
SearchMode::Input => {
    match key.code {
        KeyCode::Char(c) => {
            app.search_word.push(c);
            // DELETED: app.recompute_matches();
        }
        KeyCode::Backspace => {
            app.search_word.pop();
            // DELETED: app.recompute_matches();
        }
        KeyCode::Enter => {
            if app.search_word.is_empty() {
                app.search_mode = SearchMode::Inactive;
            } else {
                app.recompute_matches();  // <-- moved here
                app.search_mode = SearchMode::Active;
            }
        }
        KeyCode::Esc => {
            app.search_word.clear();
            app.recompute_matches();  // still needed to clear stale match_indices
            app.search_mode = SearchMode::Inactive;
        }
        _ => {}
    }
    true
}
```

### Tradeoff

- **No live search preview** — user types without seeing results until Enter. Acceptable for 90K perf.
- **Esc still clears** — `recompute_matches()` on Esc clears stale matches.
- **Backspace on empty** — no-op, no recompute.

---

## Fix #9: `SearchMatch.tree_idx` → `tree_line_idx`

### Why

`types.rs:91-95`: `SearchMatch.tree_idx: Option<usize>` stores the DFS visible count — an index that matches neither `tree_lines` nor `filtered_tree_lines`. This creates confusion:
- `refresh_filtered_lines` incorrectly uses it as a `tree_lines` index
- `jump_to_next_match` ignores it entirely (uses `walk_idx` for binary search)
- No consumer actually reads `tree_idx` correctly

### What Changes

Rename and repurpose:

```rust
pub struct SearchMatch {
    pub path: Vec<String>,
    pub tree_line_idx: Option<usize>,  // renamed from tree_idx
    pub walk_idx: usize,
}
```

In `collect_matches_in_order`, instead of `visible_count`, set `tree_line_idx` by doing a HashMap lookup on `tree_lines`:

```rust
// In recompute_matches, after building matches:
for m in &mut self.match_indices {
    m.tree_line_idx = self.path_to_tree_idx.get(&m.path).copied();
}
```

### Why Not Now

This requires `path_to_tree_idx` to be available in `recompute_matches`, which is called before `update_tree_lines` sets it up. The ordering is:

```
update_tree_lines:
  1. build tree_lines
  2. path_to_tree_idx = build from tree_lines  [new]
  3. refresh_filtered_lines
  4. recompute_matches        ← needs path_to_tree_idx
```

This is safe if `recompute_matches` runs after `path_to_tree_idx` is built. Currently `update_tree_lines` calls `refresh_filtered_lines` then `recompute_matches`, so the order is correct.

---

## Implementation Order

```
Fix #4 (remove prune)     → cargo test  # unlocks other fixes
Fix #1 (HashMap fallback)  → cargo test  # core jump perf
Fix #2 (Delta sort refresh) → cargo test  # server mode correctness
Fix #3 (delta refresh)     → cargo test  # filter clear correctness
Fix #7 (MAX_DIR 500)       → cargo test  # perf safety
Fix #5 (search debounce)   → cargo test  # input perf
```

Fixes #6, #8, #9, #10, #11 are Phase 2/3 — defer until Phase 1 is verified.

## Test Commands

```bash
# After each fix
cargo test -p argus-tui

# Full check before commit
cargo test -p argus-tui && cargo clippy -p argus-tui && cargo fmt --check -p argus-tui
```

## Rollback Strategy

Each fix is a single commit. If a fix causes regression:

```bash
git revert <commit-hash>
```

The fixes are independent — reverting one does not affect others (except Fix #1 and #9 share `path_to_tree_idx` — if both applied, revert together).