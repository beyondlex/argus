# Delta Event Detail Popup (`K` Key)

## Status: Draft

## 1. Motivation

Currently the TUI shows aggregated delta values per-file/per-dir in the file tree (e.g. `+ 1.2 MB`), but the user cannot see **what events** produced that delta. Pressing `K` on a file or directory should open a scrollable popup listing the individual delta events for its direct children — enabling drill-down from "something changed in `argus/`" to "which child files triggered it and when".

## 2. Core Concept

For a selected path `P`, show a popup with:

- **File children**: each raw `DeltaEntry` row displayed individually
- **Directory children**: if no raw event exists for the dir itself (common case), aggregate descendant `DeltaEntry` rows into one synthetic row per directory child

All rows sorted by timestamp descending.

### Example

Selected path: `~/code/github`

```
┌───────── Delta Events for: ~/code/github ─────────┐
│ Time                 Path              Delta      │
│───────────────────────────────────────────────────│
│ ▼ 2026-07-13 08:00:01  argus          + 100 MB    │  ← aggregated descendant events
│   2026-07-13 06:11:21  README.md      +  143 B    │  ← raw event
│   2026-07-13 05:01:01  README.md      -  10 KB    │  ← raw event
│                                                   │
│          [j/k scroll · Esc close]                 │
└───────────────────────────────────────────────────┘
```

## 3. Data Flow

```
User presses K on selected path P
  │
  ├─ Standalone mode: query_delta_detail(db, P, time_range)
  └─ Server mode:     IpcClient::get_delta_detail(P, time_range)
  │
  └─ Group results by direct child (first component after P/)
      │
      ├─ If entry.path == P/<child>
      │   → individual row (raw DeltaEntry)
      │
      └─ If entry.path is deeper (P/<child>/...)
          → aggregate delta_size by sum, use latest timestamp
          → one synthetic row per <child>, marked as aggregated
      │
      └─ Sort by timestamp descending
```

### Key: Anti-Double-Count

`query_delta_detail()` already handles `is_agg` rows via subquery exclusion — descendant rows covered by an `is_agg = 1` row are excluded. This prevents double-counting when a daemon aggregation worker has written agg rows. The popup reuses the same query and does not need additional dedup logic.

## 4. New Types

### `argus-tui/src/types.rs`

```rust
pub struct DeltaDetailState {
    pub path: PathBuf,              // selected path
    pub entries: Vec<DeltaDetailRow>,
    pub scroll: usize,
}

pub struct DeltaDetailRow {
    pub timestamp: String,          // "2026-07-13 HH:MM:SS"
    pub child_name: String,         // direct child name (e.g. "argus", "README.md")
    pub delta_size: i64,
    pub delta_display: String,      // "+ 100 MB", "-  10 KB"
    pub is_aggregated: bool,        // true = synthetic aggregation of descendants
}
```

## 5. App State Changes

### `argus-tui/src/app.rs`

Add to `App`:

```rust
pub delta_detail: Option<DeltaDetailState>,
```

Same pattern as `info_data: Option<(PathBuf, Metadata)>` — no new `AppMode` variant needed.

## 6. Keybinding

### `argus-tui/src/handler/browsing.rs`

| Key | Action |
|-----|--------|
| `K` | Open delta detail popup for selected node |
| `Esc` | Dismiss (also dismisses info popup — already handled) |

`K` handler:
1. Get selected path from `tree_lines[cursor]`
2. Call `delta_detail::load_delta_detail(app, db_or_ipc, path)`
3. Set `app.delta_detail = Some(state)`

## 7. Rendering

### New file: `argus-tui/src/components/delta_detail.rs`

```rust
pub fn load_delta_detail(app: &mut App, db: &Connection, path: PathBuf) { ... }
pub fn render_delta_detail_popup(f: &mut Frame, area: Rect, state: &DeltaDetailState) { ... }
```

Popup layout:
- `centered_rect(70, 60, area)` from `help_popup`
- `Clear` + `Block::default().borders(Borders::ALL).title(...)`
- Content: `ratatui::widgets::Table` with 3 columns (Time, Path, Delta)
- Header row styled bold
- Aggregated rows prefixed with `▼`
- Scroll state: `state.scroll`, clamped to `entries.len().saturating_sub(visible_rows)`

### `argus-tui/src/event.rs`

Add to `render_overlays()`:

```rust
if let Some(ref state) = app.delta_detail {
    render_delta_detail_popup(f, area, state);
}
```

## 8. IPC Changes

### `argus-tui/src/ipc_client.rs`

Add `get_delta_detail(path, from_ms, to_ms)`:

```rust
pub fn get_delta_detail(&self, path: &Path, from_ms: u64, to_ms: u64) -> Result<Vec<DeltaEntry>>
```

Sends `DaemonRequest::GetDeltaDetail { path, from_ms, to_ms }` — already defined in IPC protocol, just missing client method.

## 9. Implementation Order

```
 1. types.rs:        DeltaDetailState + DeltaDetailRow
 2. app.rs:          delta_detail field
 3. delta_detail.rs: data loading + render (new file)
 4. handler/browsing.rs: K keybinding
 5. event.rs:        render branch in render_overlays()
 6. ipc_client.rs:   get_delta_detail() (server mode)
 7. cargo build + clippy + test
```

## 10. Affected Files

| File | Change |
|------|--------|
| `argus-tui/src/types.rs` | +`DeltaDetailState`, +`DeltaDetailRow` |
| `argus-tui/src/app.rs` | +`delta_detail: Option<DeltaDetailState>` |
| `argus-tui/src/components/delta_detail.rs` | **New** — widget, data loading, render |
| `argus-tui/src/handler/browsing.rs` | +`K` handler, `Esc` dismiss |
| `argus-tui/src/event.rs` | +`render_delta_detail_popup` call |
| `argus-tui/src/ipc_client.rs` | +`get_delta_detail()` method |
