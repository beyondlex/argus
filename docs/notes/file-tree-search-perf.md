# File Tree: Search Jump Bugs & 90K File Performance

## 1 Jump Bugs

### 1.1 `jump_to_next_match` fallback O(n²)

`search.rs:56-75` — 当 delta filter 隐藏了 target match, fallback 三层循环:

```rust
for offset in 1..total {
    let try_path = &app.match_indices[try_idx].path;
    if let Some(pos) = app.tree_lines.iter().position(|line| line.path == *try_path) {
        if let Some(cursor_pos) = app.filtered_tree_lines.iter().position(|&i| i == pos) {
```

- `tree_lines.iter().position()` = O(n) path 比较 (Vec<String> 逐元素)
- `filtered_tree_lines.iter().position()` = O(m)
- 外层再包 match_indices 循环

90K files + 1K matches → 最坏 ~90M 次 path 比较.

Fix: 引入 `path_to_tree_idx: HashMap<Vec<String>, usize>` O(1) 查找.

### 1.2 `current_match` 在 filter 剪枝后不同步

`app.rs:544-551` `refresh_filtered_lines()` 剪枝 `match_indices`:

```rust
self.match_indices.retain(|m| m.tree_idx.map_or(true, |idx| visible.contains(&idx)));
self.current_match = self.current_match.min(self.match_indices.len().saturating_sub(1));
```

若 `current_match` 指向的 match 被移除, clamp 到一个**不同**的 match. 用户看到的 `current_match` 高亮与实际选中的 match 不一致.

### 1.3 `refresh_filtered_lines` 用过期 tree_idx 剪枝（浪费但安全性无问题）

`expand_path_in_tree` (`app.rs:450-453`) 调用顺序:

```
refresh_filtered_lines()  ← 用老 tree_idx 剪枝
recompute_matches()       ← 覆盖结果
```

最终正确但浪费 O(90K) 遍历.

`DeltaData` handler (`app.rs:307-316`) **只调** `refresh_filtered_lines()` 不调 `recompute_matches()`. 但此处安全：`DeltaData` 不改变 `tree_lines` 结构，旧 `tree_idx` 仍指向正确的 `tree_lines` 索引。只有当 `tree_lines` 被展开/折叠修改后又跳过 `recompute_matches` 时才会出问题，当前代码路径不存在这种情况。

### 1.4 `clear_filter_pane()` 未请求新 delta 数据

`app.rs:592-600`:

```rust
pub fn clear_filter_pane(&mut self) {
    self.delta_filter_active = false;
    self.delta_filter_value = 100;
    self.delta_filter_unit = 1;
    self.set_time_preset(0);      // 重置时间为 1h
    self.focus = Focus::Tree;
    self.refresh_filtered_lines();
    self.recompute_matches();
    // 缺少: self.request_delta_refresh();
}
```

在 server mode 下，`time_preset` 重置为 0 (1h) 后 `delta_cache` 仍保留旧时间范围的数据（如 7d）。`adjust_filter_focus` 切换 TimePreset 时调用了 `request_delta_refresh()`（`filter.rs:108`），但 `clear_filter_pane` 没有。后续 Delta 排序和过滤都基于过期数据。

### 1.6 `filtered_tree_lines` 为空时 cursor 悬空

`refresh_filtered_lines` (`app.rs:540-542`) clamp cursor:
```rust
if self.cursor >= self.filtered_tree_lines.len() && !self.filtered_tree_lines.is_empty() {
    self.cursor = self.filtered_tree_lines.len() - 1;
}
```

当 delta filter 隐藏了所有行，`filtered_tree_lines.is_empty()`，cursor=0 保留但 `filtered_tree_lines[0]` 不存在。`selected_line()` 返回 None，UI 显示空状态但 cursor 内部值不反映实际状态。

### 1.7 Delta 排序在 DeltaData 后不刷新

`app.rs:307-316`:

```rust
AppMessage::DeltaData(deltas) => {
    self.delta_cache = deltas;
    self.refresh_filtered_lines();  // 不重建 tree_lines!
}
```

若 `sort_mode == Delta`, `tree_lines` 保持旧顺序. 显示与实际 delta 值不匹配.

---

## 2 90K 文件性能瓶颈

| 位置 | 问题 | 单次开销 |
|------|------|----------|
| `search.rs:10` 搜索每键 | `recompute_matches()` 全树遍历 | O(90K) + 排序 |
| `tree_ops.rs:274-328` flatten | 每节点 `path.clone()` Vec<String> + `Arc::clone()` + `size_for_path` 构建 PathBuf | O(90K) 堆分配 |
| `app.rs:519-551` refresh_filtered | 每节点 delta_cache hash 查找 | O(90K) |
| `search.rs:132-199` collect_matches | 每节点 `fuzzy_match_indices` + `path_is_visible` | O(90K) |
| `tree_ops.rs:420-454` sort_children_snapshot Delta | 每子节点 build `child_path` Vec<String> | O(children) |
| `search.rs:213-214` fuzzy_match_indices | 每节点 `to_lowercase()` 堆分配 (target + query) | 2× String alloc/节点 |
| `app.rs:544-551` match_indices 剪枝 | `tree_idx` (visible_count) 与 `tree_lines` 索引恰好相等，依赖 DFS 遍历顺序一致性 | 架构脆弱性 |

### 2.1 单次 `n` 按键完整调用链

```
jump_to_next_match()
  ├─ O(log n) binary search for next match
  ├─ expand_ancestor_prefixes() → expand_path_in_tree()
  │     ├─ flatten_snapshot_tree()  O(expanded_subtree)  ← 只展开子目录,非全树
  │     ├─ refresh_filtered_lines() O(90K)
  │     └─ recompute_matches()      O(90K)
  └─ fallback O(n²) if delta filtered
```

一次 jump 中 `refresh_filtered_lines` + `recompute_matches` = 2× O(90K) = 180K 节点遍历.
`flatten_snapshot_tree` 在 `expand_path_in_tree` 内只处理展开目录的子树，不遍历全树.

### 2.2 搜索输入放大

```
SearchMode::Input 每按一字符:
  recompute_matches() → collect_matches_in_order()  O(90K)
```

10 字符 → 900K 遍历. 90K 节点 `big_node.name` string match → CPU 100%.

### 2.3 每按键堆分配放大

`fuzzy_match_indices` (`search.rs:213-214`) 每节点都做:
```rust
let target_lc = target.to_lowercase();   // String 分配
let query_lc = query.to_lowercase();     // String 分配
```

90K 节点 × 10 按键 = 900K 次函数调用 → 1.8M 次 String 堆分配. 每次分配触发行分配器, 90K 文件层次 CPU cache 未命中严重.

函数名 `fuzzy_match_indices` 有误导性：实际实现是 substring `find()`，不是真正的 fuzzy match.

---

## 3 修复方案

### 3.1 必须修复 (影响正确性)

| # | 优先级 | Fix | 涉及文件 |
|---|--------|-----|----------|
| 1 | P0 | `jump_to_next_match` 用 `path_to_tree_idx` HashMap 代替 O(n²) fallback | `search.rs`, `app.rs` |
| 2 | P0 | `DeltaData` 到达时 if sort_mode==Delta 调 `update_tree_lines()` | `app.rs` |
| 3 | P0 | `clear_filter_pane` 末尾加 `request_delta_refresh()`，防止 delta_cache 过期 | `app.rs` |
| 4 | P0 | `refresh_filtered_lines` 不剪枝 `match_indices`, 剪枝逻辑归 `recompute_matches` | `app.rs` |

### 3.2 性能修复 (影响体验)

| # | Fix | 方法 |
|---|-----|------|
| 5 | 搜索输入防抖 | Input 模式只存字符, Enter 才全树遍历 |
| 6 | `flatten_snapshot_tree` 减少分配 | path 复用 Vec, size_for_path 重用 PathBuf |
| 7 | `MAX_DIR_CHILDREN` 降至 500 | 防止单目录展开冻结 |
| 8 | `refresh_filtered_lines` 增量更新 | 只在 delta_cache 变更的行重新过滤, 而非全量 |

### 3.3 架构修复 (长远)

| # | Fix | 方法 |
|---|-----|------|
| 9 | SearchMatch.tree_idx 改为 `tree_line_idx`（tree_lines 的直接索引） | `recompute_matches` 时根据 tree_lines 位置赋值, 消除与 refresh_filtered_lines 的索引空间耦合 |
| 10 | 引入 dirty flag | tree_lines 未变时跳过 recompute_matches |
| 11 | fuzzy_match_indices 避免 per-node 堆分配 | 传入 &str 做 case-insensitive 比较时不分配 String, 用 chars().zip() 逐字符比对 |

---

## 4 关键调用时序

```
User presses 'n'
  → handler/search.rs: handle_search_keys()
    → search.rs: jump_to_next_match(app, 1)
      → binary search next_match_index()
      → expand_ancestor_prefixes()  // 展开祖先目录
      → app.expand_path_in_tree()
        → tree_lines.splice()       // 插入新行
        → refresh_filtered_lines()  // 重建 filtered view
        → recompute_matches()       // 重建 match_indices
      → app.tree_lines.iter().position(|l| l.path == target)  // O(n) [!]
      → app.filtered_tree_lines.iter().position(|&i| i == pos) // O(m)

User presses 'j' (DeltaValue change)
  → handler/filter.rs: adjust_filter_focus()
    → app.delta_filter_inc()
    → app.refresh_filtered_lines()  // 剪枝 match_indices [!]
                                    // 不调 recompute_matches

DeltaData arrives via IPC
  → app.rs: handle_message()
    → app.delta_cache = deltas
    → app.refresh_filtered_lines()  // 不重建 tree_lines [!]
                                    // 不调 recompute_matches [!]
```