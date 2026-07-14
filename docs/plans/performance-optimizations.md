# Performance Optimizations — Full Change Plan

## Status: Implemented (A–D, P12 deferred)

Cross-layer plan for Argus performance work at ~90K+ files and long-lived daemon use.
Covers TUI, scanner, SQLite, IPC, and watcher. Does **not** replace correctness fixes already tracked in search notes; it consolidates remaining heat and newly identified gaps.

**Implemented:** Phase A–D items P1–P11, P13–P15 (2026-07-15). **Deferred:** P12 incremental filter refresh.

**Related docs (partial overlap / partially stale):**

| Doc | Relationship |
|-----|----------------|
| `docs/notes/file-tree-search-perf.md` | Diagnosis of TUI search/jump; some line numbers and “per-keystroke” claims are stale |
| `docs/notes/file-tree-search-opt-plan.md` | Earlier TUI-only plan; Phase 1 + #5/#7 largely done |
| `docs/plans/sqlite-storage-backend.md` | Future full-tree SQLite store — out of scope here except where delta query design interacts |

---

## 1. Goals & Non-Goals

### 1.1 Goals

- Keep TUI interactive on ~90K-node trees (expand, `n`/`N`, delta filter, search Enter).
- Cut skip-dir and wide-directory scan cost without changing snapshot semantics.
- Make daemon delta flush and `GetDelta` scale with event volume / time window.
- Preserve dual-mode (standalone + server), TDD discipline, and existing public behavior unless a change is explicitly traded off below.

### 1.2 Non-Goals

- Criterion benches as a hard gate (optional follow-up).
- Full scan-history SQLite backend (see `sqlite-storage-backend.md`).
- True fuzzy search (current “fuzzy” is substring match — rename optional, algorithm change out of scope).
- GUI / AI paths.

### 1.3 Success criteria (manual + tests)

| Scenario | Target |
|----------|--------|
| Jump `n` with search active, 90K files, few matches | No multi-second UI freeze; expand ancestors without full rematch when query unchanged |
| Expand a 500-child dir | Snappy; capped by `MAX_DIR_CHILDREN` |
| Skip-dir `node_modules` ~large | Single FS walk of skipped trees (not three) |
| Debounce flush hundreds of events | One SQLite transaction per batch |
| `GetDelta` for large window | Totals without always shipping every detail row (when client only needs totals) |

Verification each slice: `cargo test && cargo clippy && cargo fmt --check`.

---

## 2. Already done (do not redo)

These landed relative to older notes; treat notes as historical if they still describe the old behavior.

| Item | Evidence |
|------|----------|
| Search Input does not call `recompute_matches` per keystroke | `argus-tui/src/handler/search.rs` — only mutate `search_word`; rematch on Enter/Esc |
| O(1) path → tree index for jump | `path_to_tree_idx` in `app.rs` / `search.rs` |
| `MAX_DIR_CHILDREN = 500` | `argus-tui/src/tree_ops.rs` |
| Incremental splice expand (not full-tree flatten on every expand) | `app.rs` `expand_path_in_tree` |
| Scanner progress batching, hardlink dedup | `argus-core/src/scanner.rs` |
| SQLite WAL + `synchronous=NORMAL` | `argus-core/src/db.rs` |
| Debounce path merge before flush | `argusd/src/debounce.rs` |

---

## 3. Inventory (prioritized)

| ID | Area | Sev | Summary |
|----|------|-----|---------|
| P1 | db | high | Wrap `insert_events` in a transaction |
| P2 | tui | high | Dirty / version skip: avoid full `recompute_matches` on expand when query unchanged |
| P3 | scanner | high | Merge skip-dir into one FS walk |
| P4 | scanner | high | O(1) / log-n child lookup; stop dual name storage if practical |
| P5 | ipc | high | Split totals vs detail; avoid always shipping full `GetDelta` detail |
| P6 | tui | med–high | Reduce flatten / path / `size_for_path` allocations |
| P7 | tui | med–high | Zero-alloc case-insensitive match (search + render highlight) |
| P8 | tui | med | `Arc<Snapshot>` in `scan_cache` (stop full clone) |
| P9 | scanner | med | Parallel walk + reuse DirEntry metadata where safe |
| P10 | db | med | Delta prefix `LIKE` / `NOT EXISTS` query cost |
| P11 | db | med | Stream / SQL-side `consolidate_events` |
| P12 | tui | med | Incremental or cached `refresh_filtered_lines` |
| P13 | tui | med | Cut `path_is_visible` / match-index path clones |
| P14 | daemon | med | Bound watcher caches; improve delete / subtree accounting |
| P15 | tui | low–med | Architecture: `tree_line_idx`, decouple match indices from filter DFS |

Suggested implement order:

```text
P1 → P2 → P3 → P5 → P6 → P7 → P8 → P4 → P9 → P12 → P13 → P10 → P11 → P14 → P15
```

Rationale: small DB win first; TUI expand freeze next (user-visible); then scanner skip-dir and IPC payload; then allocation and structural polish.

---

## 4. Detailed changes

### P1 — `insert_events` transaction

**Problem:** `argus-core/src/db.rs` `insert_events` prepares a statement then loops `execute` with no `conn.transaction()`. Default SQLite auto-commit → one fsync-ish commit per row on debounce flush.

**Change:**

```rust
pub fn insert_events(conn: &mut Connection, events: &[DeltaEntry]) -> Result<(), DbError> {
    if events.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for event in events {
            let path_str = event.path.to_string_lossy();
            stmt.execute(params![
                path_str.as_ref(),
                event.delta_size,
                event.event_type,
                event.timestamp,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}
```

**API note:** Signature becomes `&mut Connection` if `transaction()` requires it — update `argusd` call sites.

**Tests:** Existing insert/query tests; add batch-size smoke if useful. Behavior unchanged.

**Docs sync:** None for user-facing; optional note in daemon design if flush semantics are documented.

---

### P2 — Skip full rematch on expand when search query unchanged

**Problem:** `expand_path_in_tree` always ends with `refresh_filtered_lines()` + `recompute_matches()`. Jump `n` expands ancestors → ~2× O(N) even when `search_word` and match set identity are unchanged. Dominant TUI freeze at 90K.

**Change:**

1. Add `tree_lines_version: u64` and `matches_tree_version: u64` (or `matches_query: String` + version) on `App`.
2. Increment `tree_lines_version` on any `tree_lines` mutation (rebuild, expand splice, collapse, sort).
3. `recompute_matches()`:
   - Always allowed when `search_word` / mode changed.
   - When called after expand-only: either **skip** and instead update existing `SearchMatch.tree_idx` / `path_to_tree_idx` for shifted indices, or rematch only if query non-empty **and** matches depend on newly visible names (prefer: rematch only if query non-empty; for empty query clear cheaply).
4. Preferred minimal behavior for expand-with-active-search:
   - Rebuild `path_to_tree_idx` from current `tree_lines` (O(visible lines), not full arena walk).
   - Remap `match_indices[].tree_idx` via path HashMap; only fall back to full `collect_matches_in_order` if a match path is missing.
5. `refresh_filtered_lines` still runs when filter/delta visibility changes; do **not** prune matches with stale indices there (if that prune still exists — move prune into rematch only; see older opt-plan #4).

**Files:** `argus-tui/src/app.rs`, `argus-tui/src/search.rs`, handlers that call rematch.

**Tests:**

- `test_expand_path_preserves_matches_without_full_rematch` (query fixed, expand, cursor/match paths stable).
- `test_recompute_on_query_change_still_runs`.

**Tradeoff:** Index remap logic must stay correct when splice shifts indices; HashMap-by-path is the source of truth.

---

### P3 — Scanner: one walk for skip-dirs

**Problem:** For each skipped path (`scanner.rs` ~309–325):

1. `walk_dir_size` — full FS walk for size  
2. `skeleton_walk` — second walk to build dirs in arena  
3. `patch_skeleton_sizes` — third walk to fill sizes  

**Change:** Introduce e.g. `walk_skip_dir(arena, parent_idx, path, cancel, seen_inodes, progress) -> u64` that in one recursive (or `WalkBuilder`) pass:

- Creates skeleton directory nodes under `parent_idx`
- Accumulates file sizes with hardlink dedup
- Sets each dir node’s `size` bottom-up in the same pass (or return sizes via stack)

Remove separate `walk_dir_size` / `patch_skeleton_sizes` call sites for this path (keep helpers only if tests still need them, or fold tests onto the unified API).

**Semantics:** Public snapshot for skip-dirs must remain: skeleton tree + correct rolled-up sizes; no file children inside skip (current behavior).

**Tests:** Extend `test_walk_dir_size_*` / skip-dir integration tests to assert single logical walk and same sizes as before (compare size totals and arena dir count).

**Docs:** `docs/requirements/03-core-features.md` if skip-dir semantics text mentions multi-pass (unlikely).

---

### P4 — Scanner: child lookup + name storage

**Problem:** `find_or_create_child` linear-scans `children: Vec<(String, NodeIndex)>`. Wide directories approach quadratic insert cost. Name is stored on `FileNode` and again in the parent’s child tuple.

**Change (two steps):**

**P4a (required):** Per-directory lookup acceleration without changing `FileNode` public layout yet:

- Option A: temporary `HashMap<String, NodeIndex>` only during scan build, flush into `children` Vec at end of dir / end of scan.
- Option B: keep sorted `children` + binary search (more churn on insert).

Prefer A for scan-time only.

**P4b (optional / FUTURE):** Store only `NodeIndex` in `children` and rely on `arena[idx].name`; update all call sites and serialization. Larger model churn — gate behind review (`docs/requirements/08-data-model.md`).

**Tests:** Insert many siblings under one parent; assert correct tree and no duplicates. Perf assertion optional (timing flaky) — prefer structural tests.

---

### P5 — IPC / GetDelta: totals vs detail

**Problem:** `argusd` `DaemonRequest::GetDelta` always runs `query_delta_total` **and** `query_delta_detail`, then ships all entries over UDS. TUI often mainly needs totals / coarse map; detail popup already has `GetDeltaDetail`.

**Change:**

1. **Protocol:** Extend request or response so GetDelta can be totals-only:
   - Prefer additive enum/fields (e.g. `include_entries: bool` or separate `GetDeltaTotal`) — use enum extension over `bool` if more modes expected (`AGENTS.md` progressive architecture).
2. **Daemon:** If totals-only, skip `query_delta_detail`.
3. **TUI:** Use totals-only for periodic filter bar / tree delta badges if current client only aggregates clientside from full list — then either:
   - Keep needing detail for ancestor rollup → move rollup server-side (SQL / daemon aggregate by path prefix), **or**
   - Request detail only when opening delta detail UI / when sort mode is Delta.

**Concrete preferred design:**

| Client need | Call |
|-------------|------|
| Status / total for path | `GetDelta` totals-only |
| Sort by delta / filter visibility | Server returns aggregated map `path → delta` for **visible roots + children of expanded dirs** (path-bounded), not every event row |
| Delta detail popup | Existing `GetDeltaDetail` |

If server-side aggregate map is too large a jump: Phase A = totals-only flag + TUI stops requesting detail on timer refresh; Phase B = bounded prefix aggregate.

**Tests:** IPC unit/integration: totals-only response has empty entries; detail unchanged. Contract tests for bincode compat — version bump IPC if needed (`argus-core/src/ipc.rs`).

**Docs:** `docs/requirements/05-ux-interaction.md` / phase3 daemon notes if request shapes are specified.

---

### P6 — Flatten / path allocation reduction

**Problem:** `flatten_snapshot_tree` clones path components per node; `size_for_path` builds `PathBuf` repeatedly.

**Change:**

1. Pass `&mut Vec<String>` path stack; push/pop; **clone once** into `TreeLine.path` when emitting a line.
2. Reuse a scratch `PathBuf` (clear + push) for size lookup, or key size cache by `&[String]` / interned id.
3. Avoid double clone where both `TreeLine` and size helper need the path.

**Files:** `argus-tui/src/tree_ops.rs`, callers in `app.rs`.

**Tests:** Existing flatten/sort/size tests; add assert on path correctness after push/pop.

---

### P7 — Zero-alloc case-insensitive match

**Problem:** `fuzzy_match_indices` does `target.to_lowercase()` + `query.to_lowercase()` per node; render path in `file_tree.rs` may repeat.

**Change:** Case-insensitive substring without heap:

- ASCII fast path: `eq_ignore_ascii_case` / sliding window on bytes.
- Non-ASCII: iterate `chars()` with `to_lowercase()` **per char into a small stack buffer** or compare via Unicode case folding without storing full strings (document ASCII-primary if product accepts).

Also have render highlight call the same helper (no second lowercasing).

**Rename (optional):** `substring_match_indices` — only if docs/help text don’t say “fuzzy”.

**Tests:** Case variants, empty query, Unicode smoke if supporting non-ASCII.

---

### P8 — `Arc<Snapshot>` in scan cache

**Problem:** `scan_cache: HashMap<PathBuf, Snapshot>` with `.cloned()` / `insert(..., snapshot.clone())` duplicates entire arenas.

**Change:**

```rust
pub scan_cache: HashMap<PathBuf, Arc<Snapshot>>,
```

Share arcs across rebuild / resolve_scan_tree. Clone is `Arc::clone` only.

**Care:** Any mutation of snapshot contents must copy-on-write (`Arc::make_mut`) or be forbidden — snapshots should remain immutable after insert.

**Tests:** Cache hit uses same `Arc::ptr_eq`; tree rebuild still correct.

---

### P9 — Parallel walk + metadata reuse

**Problem:** Serial `WalkBuilder::build()`; entries may be re-stated with `symlink_metadata` after walk already typed them.

**Change:**

1. Use `build_parallel` **or** channel-fed workers carefully: arena inserts are not sync — prefer collect path list in parallel then sequential insert, **or** shard by top-level child (harder).
2. Prefer `ignore::DirEntry` metadata when present; skip redundant `symlink_metadata` when safe for hardlink / type detection.

**Risk:** Parallelism + cancel + hardlink global `seen_inodes` needs mutex; order of children may change → sort children for deterministic tests/snapshots.

**Tests:** Deterministic child order (sort by name); cancel still works; hardlink size semantics unchanged.

**Docs:** Mention non-deterministic walk order if any user-facing listing depends on discovery order (UI already sorts).

---

### P10 — Delta SQL prefix queries

**Problem:** `LIKE path||'%'` plus correlated `NOT EXISTS` anti-join over agg rows is expensive as `delta_events` grows.

**Change options (pick after EXPLAIN QUERY PLAN):**

1. Maintain coverage/agg markers so detail queries don’t need correlated anti-join.
2. Narrow indexes: `(is_agg, path, timestamp)` or covering index for common filter.
3. Simplify semantics documented in requirements if anti-join is over-precise for product needs.

**Do not** “optimize” by loading all rows into Rust unless consolidating (see P11).

**Tests:** Query correctness for nested agg vs leaf events unchanged.

---

### P11 — `consolidate_events` streaming

**Problem:** Loads all non-agg rows into a `Vec` then parent `HashMap` — RAM spike on large DBs.

**Change:** Prefer SQL `GROUP BY path` (or batched keyed cursor) to compute aggregates; delete/replace in a transaction (pattern already exists for some deletes with `tx`).

**Tests:** Consolidation idempotence / totals preserved (existing tests in `db.rs`).

---

### P12 — Incremental `refresh_filtered_lines`

**Problem:** Full O(visible tree_lines) scan on every delta filter tweak.

**Change:**

- Cache per-line last filter verdict + last delta value used.
- On filter value change only: re-evaluate lines (still O(n) of lines, but skip hash/path work if delta unchanged — limited win), **or**
- Keep O(n) over **visible** lines only (already) and ensure expand doesn’t duplicate filter+rematch (P2 is higher leverage).

Treat P12 as polish after P2; avoid complex dirty bit machinery unless profiling still shows filter path hot.

---

### P13 — Match collection path clones

**Problem:** `path_is_visible` does `path[..len].to_vec()`; match maps insert cloned paths for all nodes.

**Change:**

- Check visibility with borrowed prefixes (`expanded.contains` needs owned keys today — consider `HashSet` of `Arc<[String]>` or intern ids).
- Or store `expanded` as `HashSet<u64>` path hashes with care for collisions (prefer intern).

**Minimal fix:** Avoid allocating in the ancestor loop when `expanded` can answer via iterative push/pop on a reusable `Vec` and HashSet keyed the same way as expand (reuse one buffer).

---

### P14 — Watcher cache bounds & accounting

**Problem:** `WatcherState` `size_cache` / `hardlink_cache` unbounded; create/modify/delete are file-centric; deletes of never-seen files under-count; no directory subtree rollup in watcher (daemon is event → delta_events, not live tree).

**Change:**

1. Cap caches (LRU / random drop / clear on consolidate) with configurable max entries.
2. On delete: if size unknown, skip or emit 0 with metric — document behavior.
3. Optional: on directory remove, emit a single aggregated delta if OS provides it (platform-dependent) — mark `FUTURE` if incomplete across OSes.

**Tests:** Cache eviction doesn’t panic; hardlink still dedups while entries remain.

---

### P15 — Match index architecture (from older plan #9)

**Problem:** `SearchMatch.tree_idx` semantics coupled to DFS / filter index spaces historically.

**Change:** Rename to `tree_line_idx: Option<usize>` meaning index into `tree_lines`. Set only from current `tree_lines` via `path_to_tree_idx`. Filtered cursor uses `filtered_tree_lines` mapping separately.

**Depends on:** P2 remap logic. Do after P2 stabilizes.

**Tests:** Filter + jump + expand integration cases from `file-tree-search-opt-plan.md` Phase 1.

---

## 5. Phased rollout

### Phase A — Quick wins (1–2 PRs)

| Order | ID | PR focus |
|-------|-----|----------|
| 1 | P1 | DB insert transaction |
| 2 | P2 | Expand/jump without full rematch |
| 3 | P7 | Zero-alloc match (small, isolated) |

### Phase B — Scan & IPC (1–2 PRs)

| Order | ID | PR focus |
|-------|-----|----------|
| 4 | P3 | Skip-dir single walk |
| 5 | P5 | GetDelta totals / bounded detail |
| 6 | P8 | `Arc<Snapshot>` |

### Phase C — Allocation & structure

| Order | ID | PR focus |
|-------|-----|----------|
| 7 | P6 | Flatten path reuse |
| 8 | P4a | Scan child HashMap |
| 9 | P13 | Match path clone cuts |
| 10 | P12 | Filter refresh polish (if needed) |

### Phase D — Scale & longevity

| Order | ID | PR focus |
|-------|-----|----------|
| 11 | P9 | Parallel walk (careful determinism) |
| 12 | P10–P11 | SQL query / consolidate |
| 13 | P14 | Watcher bounds |
| 14 | P15 | tree_line_idx cleanup |
| — | P4b | Model children-without-name (only with architect OK) |

---

## 6. Testing strategy

| Layer | What |
|-------|------|
| Unit | Each P# with `test_<fn>_<scenario>` in-module |
| Integration | TUI search/expand; daemon insert+GetDelta; scanner skip-dir |
| Manual | 90K tree: search Enter, spam `n`, expand huge dirs, toggle delta filter, long debounce under FS churn |
| Optional | `criterion` for `insert_events` batch, skip-dir walk, `recompute_matches` |

---

## 7. Docs sync checklist (per PR)

| If you change… | Update… |
|----------------|---------|
| IPC request/response | `docs/requirements` UX / architecture as applicable; `argus-core` ipc module docs |
| `FileNode` / children layout (P4b) | `docs/requirements/08-data-model.md` |
| Skip-dir / scan behavior user-visible | `docs/requirements/03-core-features.md`, README if needed |
| Stable TUI behavior | `docs/notes/tui-current-behavior.md` |
| This plan’s “Already done” | Check off IDs here when merged |

After Phase A lands, mark obsolete sections in `docs/notes/file-tree-search-opt-plan.md` with a pointer to this file (avoid conflicting “next steps”).

---

## 8. Risks & tradeoffs

| Risk | Mitigation |
|------|------------|
| P2 index remap bugs → wrong jump highlight | Path HashMap as source of truth; heavy tests |
| P5 totals-only breaks delta sort | Don’t switch TUI sort path until aggregate API ready |
| P9 parallel scan flaky order | Explicit sort by name before snapshot finalize |
| P4b data model break | Architect review; separate PR |
| Over-engineering P12 dirty flags | Profile after P2; skip if cold |

---

## 9. Open questions (need confirmation before coding)

1. **P5:** Is a breaking IPC version bump acceptable, or must old clients keep working via additive fields only?
2. **P7:** ASCII-only case fold OK, or full Unicode required?
3. **P4b:** Pursue children-without-duplicated-name in this performance track, or defer indefinitely?
4. **P9:** Is deterministic child order a hard requirement for CLI JSON output?

---

## 10. Agent / implementer checklist

```text
[x] P1  insert_events transaction + call sites &mut
[x] P2  tree version / remap matches on expand
[x] P3  unified skip-dir walk
[x] P4a scan-time child HashMap
[x] P5  GetDelta totals-only (+ optional aggregate)
[x] P6  flatten path stack reuse
[x] P7  zero-alloc case-insensitive match (ASCII)
[x] P8  Arc<Snapshot> scan_cache
[x] P9  metadata reuse + sorted children (no build_parallel)
[x] P10 delta SQL / index tune
[x] P11 consolidate streaming aggregation
[ ] P12 filtered lines incremental (deferred — P2 covers main expand cost)
[x] P13 match path clone reduction (superseded by remap; path_is_visible removed)
[x] P14 watcher cache bounds
[x] P15 tree_line_idx rename / decoupling
[x] Update this plan checkboxes + stale note pointers
[x] cargo test && cargo clippy && cargo fmt --check
```

### Implementation notes (2026-07-15)

- **P2:** `expand_path_in_tree` calls `remap_match_tree_indices()` instead of full `recompute_matches`.
- **P5:** `GetDelta { include_entries }` — TUI refresh still uses `true`; totals-only path tested in daemon.
- **P9:** Conservative — reuse walk metadata + sort children by name; no parallel walk.
- **P12:** Deferred; profile again if delta-filter keystroke path is still hot after P2.
- Cross-ref: older `docs/notes/file-tree-search-opt-plan.md` Phase 2 #5/#7 are done; prefer this plan for remaining work.
