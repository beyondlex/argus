# File Tree Size Bug Investigation

## Scope

This note records the recent `file tree` / `filesize` fix chain and the current data-flow around:

- filesystem listing
- scan cache enrichment
- scan history / diff overlay
- render-time size presentation

The goal is to explain why `size` bugs keep reappearing and where the model is still ambiguous.

## Recent Fix Chain

- `b47fcc5`: introduced `has_metadata` and structural placeholder nodes for skipped directories
- `0b840d0`: switched tree rendering to current FS state with diff as an overlay
- `17d05dc`: changed tree root construction to always start from live FS listing and enrich sizes from cache
- `8067621`: expanded lazy-load enrichment so deeper directories can recover scanned size data

The fixes solved specific symptoms, but they also added overlapping sources of truth.

## Current Data Flow

```mermaid
flowchart TD
    A[scan_path / list_dir] --> B[FileNode tree]
    B --> C[scan_cache: HashMap<PathBuf, Snapshot>]
    C --> D[build_current_tree]
    D --> E[tree_root: live FS structure]

    C --> F[flatten_snapshot_tree]
    F --> G[TreeLine.has_scan_data]
    F --> H[TreeLine.delta from diff_lookup]

    I[query_delta / build_diff_tree] --> J[diff_lookup]
    J --> F

    E --> K[expand_node lazy-load]
    K --> L[list_dir(dir_path)]
    K --> M[enrich from root snapshot tree]
    K --> N[mutate tree_root children]

    F --> O[file_tree render]
    O --> P[size display]
    O --> Q[delta display]
    O --> R["..." / "-" / formatted size]

    F --> S[metadata panel]
    S --> T[current_size / delta / file count]
```

## What Each Layer Means Today

### 1. `FileNode.size`

`size` now means different things depending on where the node came from:

- `list_dir`: directory `size = 0`, file `size = metadata.len()`
- `scan_path`: directory `size = recursive aggregate`
- `scan_cache` enrichment: live FS node may inherit a scanned aggregate size
- lazy-load expansion: child nodes may be patched in-place with cached recursive sizes

So `size == 0` is not a stable semantic signal by itself.

### 2. `has_metadata`

`has_metadata` is used as a UI signal for structural completeness:

- `true`: node has real metadata and can show a size
- `false`: node is only a structural placeholder and should render as `...`

This is mostly a rendering concern, not a filesystem-state concern.

### 3. `has_scan_data`

`has_scan_data` is currently used to decide whether a directory should show `-` or a real size.

The problem is that this field is not a clean “node has historical scan data” flag. In practice it also means:

- the root was scanned
- this path exists in scan cache
- or the tree was enriched from a scanned parent

That makes the field easy to misread and easy to misuse.

### 4. `diff_lookup`

Diff data no longer replaces the tree. It is overlaid at flatten time:

- tree structure stays live FS
- delta values come from `diff_lookup`
- size values still come from the tree node itself

This is the correct direction, but it increases the need for a very clear size contract.

## Main Inconsistencies

### A. One field, multiple semantics

`size` is currently used for:

- real file size
- recursive directory size
- placeholder zero
- cached historical directory size

That makes any logic based on `size == 0` inherently fragile.

### B. Two metadata flags, one is structural and one is historical

`has_metadata` and `has_scan_data` both affect size rendering, but they do not describe the same thing.

- `has_metadata` answers: "is this node a real metadata-bearing node?"
- `has_scan_data` answers: "do we have scan-derived size data for this node?"

The UI currently derives display state from both, but the code path that sets them is scattered.

### C. Tree structure and size overlay are still partially coupled

`build_current_tree` says the tree always reflects live FS state, but it also mutates node sizes from scan cache.

That means the structure is live, but the size semantics are hybrid.

### D. Expansion logic depends on stale-state heuristics

`expand_node` uses `children.is_empty()` or `children.values().all(|c| c.size == 0)` to decide whether to reload from disk.

This is a heuristic, not a state machine. It can confuse:

- real empty directories
- unscanned directories
- placeholder structural directories
- old snapshot residue

## Why Bugs Repeat

The repeated bug pattern is:

1. A fix changes one layer’s interpretation of size.
2. Another layer still uses an older interpretation.
3. The UI looks correct for one case and wrong for another.
4. A new special case is added instead of tightening the model.

That creates more branches and more implicit state, which makes the next bug easier to trigger.

## Practical Reading

If the goal is to stabilize this area, the most important missing piece is a single authoritative contract for:

- when a node may show a size
- when a node must show `-`
- when a node must show `...`
- which layer owns live FS structure
- which layer owns scan-derived size

Until that contract is centralized, fixes will likely keep oscillating between the same few edge cases.
