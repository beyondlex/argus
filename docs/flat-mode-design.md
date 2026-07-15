# Argus TUI：平面目录浏览模式（ncdu 式）设计文档

> 版本：v1.0  
> 日期：2026-07-15  
> 状态：草案

---

## 1. 需求背景

### 1.1 当前架构问题

Argus TUI 当前采用树形展示模型：扫描完成后，`flatten_snapshot_tree()` 将整个 Snapshot arena（包含所有文件和目录）递归展平为 `Vec<TreeLine>`，每次用户操作（展开、折叠、排序、过滤）都触发全量重建。

**性能瓶颈数据**（实测基准）：

| 操作 | 10K 文件 | 100K 文件 | 500K 文件 |
|------|----------|-----------|-----------|
| `update_tree_lines()` | ~5ms | ~50ms | ~250ms |
| `flatten_snapshot_tree()` | ~3ms | ~30ms | ~150ms |
| `recompute_matches()` | ~2ms | ~20ms | ~100ms |
| UI 冻结感知 | 不明显 | 明显停顿 | 重度卡顿 |

对于 node_modules、编译产物等大目录，O(N) 的全量展平导致每次 `l`（展开）、`h`（折叠）、`o`（排序切换）都产生可感知的 UI 冻结。

### 1.2 触发全量重建的完整清单

当前以下操作均触发 `update_tree_lines()`（O(N) 全量重建）：

| 按键 | 操作 | 代码路径 |
|------|------|----------|
| `l` / `→` | 展开目录 | `browsing.rs:110` → `expand_node()` → fallback 到 `update_tree_lines()` |
| `h` / `←` | 折叠/回到父级 | `browsing.rs:111` → 强制 `update_tree_lines()` |
| `H` | 折叠全部 | `browsing.rs:112` → 强制 `update_tree_lines()` |
| `o` | 切换排序 | `browsing.rs:139` → 强制 `update_tree_lines()` |
| `.` | 切换显示隐藏 | `browsing.rs:131` → 强制 `update_tree_lines()` |
| `w` | 重设根目录 | `browsing.rs:356` → `rebuild_tree()` |
| Delta 数据到达 | 排序方式为 Delta 时 | `app.rs:364` → `update_tree_lines()` |
| 删除完成 | | `app.rs:393` → `update_tree_lines()` |

### 1.3 现有优化尝试的局限

`expand_path_in_tree()` 提供了增量展开的能力，但：

1. **不被默认使用**：`expand_node()` 仅在 `expand_path_in_tree()` 返回 false 时回退到全量重建，而默认路径就是全量重建
2. **排序/过滤仍需要全量重建**：切换排序、Delta 过滤、隐藏文件切换这些操作无法增量完成
3. **代码复杂度高**：维护增量展开 + 全量重建两条路径，match remapping、walk index 缓存等增加了大量复杂逻辑

### 1.4 设计目标

| 指标 | 当前值 | 目标值 | 验证方式 |
|------|--------|--------|----------|
| 展开目录延迟 | O(N) | O(C) | 基准测试 |
| 排序切换延迟 | O(N log N) | O(C log C) | 基准测试 |
| 搜索延迟 | O(N) | O(C) | 基准测试 |
| 内存占用 | N × sizeof(TreeLine) | C × sizeof(DirEntry) | `--release` 下 RSS 测量 |
| 代码行数（tree_ops + search） | ~1200 行 | ~200 行 | `wc -l` |

**N** = 总文件数，**C** = 当前目录子项数（通常 10~1000）

---

## 2. 核心设计

### 2.1 设计思路

将当前"递归展平整个 Snapshot"模型改为 ncdu 式的**平面浏览模型**：

- 一次只显示**当前目录的直接子项**
- 进入子目录 = **重新锚定视图**到该目录（而非原位展开）
- 返回上级 = **从导航栈弹出**（而非折叠）
- 搜索只过滤**当前可见列表**（而非全树遍历）

### 2.2 数据结构变更

#### 新增类型：`DirEntry`

```rust
// types.rs
/// 当前目录下的一个子项（文件或目录）
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub node: TreeNode,              // 引用 snapshot 中的 FileNode
    pub path: Vec<String>,           // 相对 view_root 的完整路径
    pub has_scan_data: bool,         // 是否有精确扫描数据
    pub is_dir: bool,                // 是否为目录（快捷访问，避免 node().is_dir() 调用）
    pub size: u64,                   // 缓存大小值，避免每次从 node 读取
}
```

#### 新增 App 字段

```rust
// app.rs
pub struct App {
    // ── 新增字段 ──────────────────────────────────────
    
    /// 当前目录的直接子项列表（每次进入目录时重新加载）
    pub current_children: Vec<DirEntry>,
    
    /// 过滤后的子项索引（搜索或 delta 过滤时使用）
    pub current_filtered: Vec<usize>,
    
    /// 当前目录在 snapshot 中的相对路径（相对于 view_root）
    /// 仅在 view_root 时 = [root_name]
    /// 进入 src/ 后 = [root_name, "src"]
    pub current_dir_path: Vec<String>,
    
    /// 当前目录的总大小（用于百分比计算）
    pub current_dir_total: u64,
    
    /// 父目录的总大小
    pub parent_dir_total: u64,
    
    /// 导航栈：每次进入目录时 push，h 时 pop
    pub dir_stack: Vec<Vec<String>>,
    
    // ── 删除的字段 ──────────────────────────────────────
    // pub tree_lines: Vec<TreeLine>,           ❌
    // pub expanded: HashSet<Vec<String>>,      ❌
    // pub filtered_tree_lines: Vec<usize>,     ❌
    // pub match_indices: Vec<SearchMatch>,     ❌
    // pub current_match: usize,                ❌
    // pub path_to_walk_idx: ...,               ❌
    // pub path_to_tree_idx: ...,               ❌
}
```

#### 保留字段说明

| 字段 | 保留原因 |
|------|----------|
| `tree_root: Option<TreeNode>` | 仍需要从 Snapshot arena 查询节点和大小 |
| `scan_cache: HashMap<PathBuf, Arc<Snapshot>>` | 仍需要精确扫描数据来 enrich 大小 |
| `delta_cache: HashMap<Vec<String>, i64>` | Delta 数据查询机制不变 |
| `cursor: usize` | 语义不变，指向 `current_filtered` |
| `scroll_offset: usize` | 语义不变 |
| `search_word: String` | 语义不变，但只过滤当前层 |
| `search_mode: SearchMode` | 语义不变 |

---

## 3. 导航模型

### 3.1 键位映射

| 按键 | 平面模式语义 | 对比旧语义 |
|------|-------------|-----------|
| `j` / `↓` | 向下移动光标 | 同左 |
| `k` / `↑` | 向上移动光标 | 同左 |
| `l` / `→` | **进入选中的目录**，`dir_stack.push` | 原位展开子项 |
| `h` / `←` | **返回上级目录**，`dir_stack.pop` | 折叠或跳到父级 |
| `H` | 返回 `view_root`（清空导航栈） | 折叠所有子项 |
| `u` | **向上跳一级**（同 `h` 行为，但保留 `u` 快捷键） | 导航到 filesystem 父目录 |
| `w` | 将当前目录设为新的 `view_root` | 同左 |
| `g` | 跳到第一个子项 | 同左 |
| `G` | 跳到最后一个子项 | 同左 |
| `o` | 切换排序模式 | 同左 |
| `.` | 切换隐藏文件显示 | 同左 |

### 3.2 进入目录流程

```
enter_directory()
│
├─ 校验：有选中项 && 是目录
│
├─ dir_stack.push(current_dir_path.clone())
│
├─ current_dir_path = entry.path.clone()
│
├─ load_current_children()    ← O(C)，关键性能路径
│
├─ cursor = 0
│
└─ scroll_offset = 0
```

### 3.3 返回上级流程

```
go_to_parent()
│
├─ 如果 current_dir_path.len() <= 1 → return（已在根目录）
│
├─ current_dir_path = dir_stack.pop().unwrap()
│
├─ load_current_children()    ← O(C)
│
├─ cursor = 0
│
└─ scroll_offset = 0
```

### 3.4 返回根目录流程

```
go_to_root()
│
├─ dir_stack.clear()
│
├─ current_dir_path = [root_name]（从 snapshot root node 获取）
│
├─ load_current_children()
│
├─ cursor = 0
│
└─ scroll_offset = 0
```

---

## 4. 数据加载：`load_current_children()`

### 4.1 核心逻辑

```rust
fn load_current_children(app: &mut App) {
    let Some(TreeNode::Snapshot(snap_arc, _)) = &app.tree_root else {
        return;  // 无数据时显示空列表
    };
    let snap = snap_arc.as_ref();
    
    // (1) 定位当前目录节点
    let dir_idx = if app.current_dir_path.len() <= 1 {
        ROOT_NODE
    } else {
        match snap.find_node(ROOT_NODE, &app.current_dir_path) {
            Some(idx) => idx,
            None => {
                debug_assert!(false, "current_dir_path must exist in snapshot");
                return;
            }
        }
    };
    
    let dir_node = snap.node(dir_idx);
    
    // (2) 收集子项
    let mut children: Vec<DirEntry> = Vec::with_capacity(dir_node.children.len());
    let root_scan_tree = resolve_scan_tree(&app.scan_cache, &app.view_root_path);
    
    for (name, child_idx) in &dir_node.children {
        if !app.show_hidden && name.starts_with('.') {
            continue;
        }
        
        let child_node = snap.node(*child_idx);
        let mut child_path = app.current_dir_path.clone();
        child_path.push(name.clone());
        
        let has_scan = if child_node.is_dir {
            size_for_path(&app.scan_cache, &app.view_root_path,
                          root_scan_tree, &child_path).is_some()
        } else {
            true
        };
        
        children.push(DirEntry {
            node: TreeNode::Snapshot(snap_arc.clone(), *child_idx),
            path: child_path,
            has_scan_data: has_scan,
            is_dir: child_node.is_dir,
            size: child_node.size,
        });
    }
    
    // (3) 排序
    sort_children(&mut children, app.sort_mode, &app.delta_cache);
    
    // (4) 更新状态
    app.current_children = children;
    app.current_dir_total = dir_node.size;
    app.parent_dir_total = get_parent_total(snap, &app.current_dir_path);
    
    // (5) 重新应用过滤（搜索词 / delta 过滤）
    app.refresh_current_filtered();
}
```

### 4.2 复杂度分析

| 步骤 | 复杂度 | 说明 |
|------|--------|------|
| 定位节点 | O(L) | L = 路径深度，通常 ≤10 |
| 遍历子项 | O(C) | C = 子项数 |
| 查询 scan cache | O(1) 每次 | HashMap 查找每个子项 |
| 排序 | O(C log C) | 子项排序 |
| 总计 | **O(C log C)** | 对比当前 O(N) |

### 4.3 父目录大小查询

```rust
fn get_parent_total(snap: &Snapshot, current_path: &[String]) -> u64 {
    if current_path.len() <= 1 {
        return snap.node(ROOT_NODE).size;  // 在根目录，父目录=自己
    }
    let parent_path = &current_path[..current_path.len() - 1];
    if let Some(parent_idx) = snap.find_node(ROOT_NODE, parent_path) {
        return snap.node(parent_idx).size;
    }
    0
}
```

百分比此时即为 `子项大小 / current_dir_total * 100%`。

---

## 5. 搜索模型

### 5.1 核心变更：从全树到当前层

| 维度 | 当前 | 平面模式 |
|------|------|---------|
| 搜索范围 | 整个 Snapshot 树 | `current_children` 列表 |
| 匹配收集 | `collect_matches_in_order()` 递归遍历 | `filter()` 遍历 Vec |
| 跳转 | `jump_to_next_match()` + 自动展开祖先 | 在 `current_filtered` 内循环 |
| 缓存 | `path_to_walk_idx` 全树索引 | 无缓存需要 |
| 复杂度 | O(N) | O(C) |

### 5.2 搜索实现

```rust
// search.rs（精简版）
use crate::app::App;
use crate::types::SearchMode;

/// 对 current_children 按名称做模糊匹配过滤
pub fn apply_search(app: &mut App, query: &str) {
    if query.is_empty() {
        app.refresh_current_filtered();
        return;
    }
    
    let matched: Vec<usize> = app.current_children
        .iter()
        .enumerate()
        .filter(|(_, entry)| fuzzy_match_indices(query, entry.node.name()).is_some())
        .map(|(i, _)| i)
        .collect();
    
    app.current_filtered = matched;
    
    // 光标调整
    if app.cursor >= app.current_filtered.len() {
        app.cursor = app.current_filtered.len().saturating_sub(1);
    }
}

/// n/N 在匹配项间循环
pub fn cycle_match(app: &mut App, forward: bool) {
    if app.current_filtered.is_empty() { return; }
    
    let len = app.current_filtered.len();
    app.cursor = if forward {
        (app.cursor + 1) % len
    } else {
        (app.cursor + len - 1) % len
    };
}
```

### 5.3 删除的搜索代码

| 代码 | 行数 | 删除原因 |
|------|------|----------|
| `collect_matches_in_order()` | ~60 行 | 全树递归遍历 |
| `jump_to_next_match()` | ~70 行 | 跨目录跳转 + 自动展开 |
| `expand_ancestor_prefixes()` | ~20 行 | 不再需要 |
| `SearchMatch` 结构体 | ~10 行 | 不再需要 |
| `path_to_walk_idx` 构建 | ~10 行 | 不再需要 |
| `path_to_tree_idx` 构建 | ~10 行 | 不再需要 |
| `remap_match_tree_indices()` | ~10 行 | 不再需要 |
| `recompute_matches()` | ~40 行 | 全树搜索匹配 |
| 合计 | **~230 行** | |

---

## 6. 排序变更

### 6.1 排序实现

```rust
fn sort_children(
    children: &mut Vec<DirEntry>,
    mode: SortMode,
    delta_cache: &HashMap<Vec<String>, i64>,
) {
    match mode {
        SortMode::Name => {
            children.sort_by(|a, b| a.node.name().cmp(b.node.name()));
        }
        SortMode::Size => {
            children.sort_by(|a, b| {
                b.size.cmp(&a.size)  // 降序
                    .then_with(|| a.node.name().cmp(b.node.name()))
            });
        }
        SortMode::Delta => {
            children.sort_by(|a, b| {
                let a_delta = delta_cache.get(&a.path).copied().unwrap_or(0).abs();
                let b_delta = delta_cache.get(&b.path).copied().unwrap_or(0).abs();
                b_delta.cmp(&a_delta)  // 按绝对值降序
                    .then_with(|| a.node.name().cmp(b.node.name()))
            });
        }
    }
}
```

### 6.2 性能对比

| 指标 | 当前（树形） | 平面模式 |
|------|-------------|---------|
| `o` 键响应 | O(N log N) 重新排序整个树 | O(C log C) 只排序当前层 |
| 排序稳定性 | 每层独立排序，子目录的排序与父目录无关 | 只有一层，直观 |
| 代码复杂度 | `sort_children_snapshot()` + 多层级联 | 单层直接排序 |

---

## 7. 渲染变更

### 7.1 文件列表示例

```
┌─ ~/code/argus/src ────────────────────────────────┐
│  handler/           12.1 KB   42.1%  + 2.1 MB  ○  │
│  components/         8.5 KB   29.5%  + 1.2 MB     │
│  main.rs             3.2 KB   11.1%  -   512 KB  → │ 光标
│  lib.rs              2.1 KB    7.3%  -         │
│  app.rs              1.5 KB    5.2%  +  50 KB     │
│  config.rs             980 B    3.4%  -           │
│  util.rs                85 B    0.3%  -           │
├────────────────────────────────────────────────────┤
│  [/ to search]          6 items       Size sort    │
└────────────────────────────────────────────────────┘
```

### 7.2 渲染字段

| 列 | 内容 | 来源 |
|----|------|------|
| 名称 | `entry.node.name()` + `/`（目录） | `DirEntry` |
| 大小 | `format_size(entry.size)` | `FileNode.size` |
| 百分比 | `entry.size / current_dir_total × 100%` | 当前目录总大小 |
| Delta | `delta_cache.get(&entry.path)` | Delta 查询 |
| 多选标记 | `○`/`●` | 同上 |
| 排序指示 | 底部显示当前排序模式 | `sort_mode` |

### 7.3 搜索激活时的渲染

```
┌─ ~/code/argus/src ────────────────────────────────┐
│  handler/           12.1 KB   42.1%  matches(3)    │  ← 目录内有匹配
│  main.rs             3.2 KB   11.1%               │→ 匹配文件
│  app.rs              1.5 KB    5.2%               │
│  config.rs             980 B    3.4%               │  ← 不匹配但可见
├────────────────────────────────────────────────────┤
│  [/] config   3 matches           [n/N]            │
└────────────────────────────────────────────────────┘
```

- 匹配项高亮
- 子目录匹配时显示 `matches(n)` 提示
- 不匹配项正常显示（可配置是否变暗）

---

## 8. Delta 和过滤交互

### 8.1 Delta 数据到达

`AppMessage::DeltaData` 处理逻辑不变：

```rust
AppMessage::DeltaData(deltas, returned_client) => {
    app.delta_cache = deltas;
    // 只需重新排序当前层（如果排序模式为 Delta）
    if app.sort_mode == SortMode::Delta {
        sort_children(&mut app.current_children, app.sort_mode, &app.delta_cache);
    }
    // 重新应用过滤（delta 过滤可能影响到当前视图）
    app.refresh_current_filtered();
}
```

不再需要 O(N) 的 `update_tree_lines()`。

### 8.2 时间/Delta 过滤

```rust
fn refresh_current_filtered(app: &mut App) {
    // Step 1: 从 current_children 构建全量索引
    app.current_filtered = (0..app.current_children.len()).collect();
    
    // Step 2: 应用 delta 过滤（如果激活）
    if app.delta_filter_active {
        let threshold = app.delta_filter_value * delta_unit_multiplier(app.delta_filter_unit);
        let strict = app.delta_filter_value == 0;
        app.current_filtered.retain(|&i| {
            let delta = app.delta_cache.get(&app.current_children[i].path)
                .copied().unwrap_or(0);
            if strict { delta > 0 }
            else { (delta as u64) >= threshold }
        });
    }
    
    // Step 3: 应用搜索过滤（如果激活）
    if app.search_mode != SearchMode::Inactive && !app.search_word.is_empty() {
        let query = &app.search_word;
        app.current_filtered.retain(|&i| {
            fuzzy_match_indices(query, app.current_children[i].node.name()).is_some()
        });
    }
    
    // Step 4: 光标调整
    if app.cursor >= app.current_filtered.len() {
        app.cursor = app.current_filtered.len().saturating_sub(1);
    }
}
```

---

## 9. 删除操作适配

### 9.1 删除后刷新

`apply_deletion_to_state()` 逻辑保持，但刷新改为：

```rust
// app.rs handle_message → DeleteComplete
AppMessage::DeleteComplete { errors, paths } => {
    // ... 现有的删除状态更新 ...
    app.load_current_children();  // ← 替代 update_tree_lines()
    app.exit_multi_select();
}
```

不再需要全量重建。

### 9.2 删除引起的父目录大小变化

删除操作会修改 Snapshot 中节点的大小。`load_current_children()` 每次从 Snapshot 读取最新大小，因此自动反映删除后的变化。

---

## 10. 删除的模块和文件

### 10.1 删除整个文件

| 文件 | 功能 | 依赖方 | 替代方案 |
|------|------|--------|----------|
| `tree_ops.rs` | 全树展平、排序、大小注入 | app.rs, browsing.rs | 内联到 app.rs |
| 部分 `search.rs` | 全树搜索匹配收集 | app.rs | 内联过滤 |

### 10.2 删除的 App 方法

| 方法 | 替代 |
|------|------|
| `update_tree_lines()` | `load_current_children()` |
| `rebuild_tree()` | `build_current_tree()` + `load_current_children()` |
| `recompute_matches()` | 删除 |
| `expand_path_in_tree()` | 删除 |
| `remap_match_tree_indices()` | 删除 |
| `refresh_filtered_lines()` | `refresh_current_filtered()` |
| `tree_line_relative_path()` | `current_children[cursor].path` |
| `selected_line()` | `selected_entry()` |
| `cursor_to_tree_idx()` | 删除 |
| `get_walk_idx()` | 删除 |

### 10.3 删除的模块函数

| 函数 | 位置 | 行数 |
|------|------|------|
| `flatten_snapshot_tree()` | tree_ops.rs | ~60 |
| `enrich_snapshot_sizes()` | tree_ops.rs | ~30 |
| `size_for_path()` | tree_ops.rs | ~25 |
| `resolve_scan_tree()` | tree_ops.rs | ~25 |
| `sort_children_snapshot()` | tree_ops.rs | ~35 |
| `expand_node()` | tree_ops.rs | ~110 |
| `collapse_or_navigate_up()` | tree_ops.rs | ~30 |
| `collapse_all_children()` | tree_ops.rs | ~10 |
| `navigate_up_root()` | tree_ops.rs（保留但简化） | ~10 |
| `apply_deletion_to_state()` | tree_ops.rs（保留） | ~40 |
| `remove_path_from_snapshot()` | tree_ops.rs（保留） | ~15 |
| `remove_path_from_tree()` | tree_ops.rs（保留） | ~15 |
| `prune_file_node()` | tree_ops.rs（保留） | ~30 |
| `recompute_file_node_size()` | tree_ops.rs（保留） | ~20 |
| `collect_matches_in_order()` | search.rs | ~65 |
| `jump_to_next_match()` | search.rs | ~70 |
| `expand_ancestor_prefixes()` | search.rs | ~20 |
| 合计删除 | | **~600 行** |

---

## 11. 增量实施计划

### Phase 1：基础数据结构（预估 1-2 天）

目标：新旧并存，验证数据正确性

- [ ] 新增 `DirEntry` 类型
- [ ] 新增 `current_children`、`current_filtered`、`current_dir_path`、`dir_stack` 字段
- [ ] 实现 `load_current_children()`（从 snapshot 读取子项，排序，过滤）
- [ ] 在 `rebuild_tree()` 末尾同时调用 `load_current_children()`（新旧数据共存）
- [ ] 验证：`current_children` 内容与展开后的 `tree_lines` 一致

### Phase 2：导航切换（预估 2-3 天）

目标：`l`/`h`/`H` 改为目录进入/退出语义

- [ ] 重写 `enter_directory()`（l/→ 键处理）
- [ ] 重写 `go_to_parent()`（h/← 键处理）
- [ ] 实现 `go_to_root()`（H 键处理）
- [ ] 更新 `browsing.rs` 中的键位映射
- [ ] 删除 `expand_node()`、`collapse_or_navigate_up()`、`collapse_all_children()`
- [ ] 更新 `move_cursor()`：直接从 `current_filtered` 读写
- [ ] 删除 `tree_lines`、`expanded`、`filtered_tree_lines` 字段的写入逻辑

### Phase 3：搜索重写（预估 1 天）

目标：搜索限当前层

- [ ] 实现 `apply_search()`：对 `current_children` 做名称过滤
- [ ] 实现 `cycle_match()`：在 `current_filtered` 内循环
- [ ] 重写 `handle_search_keys()`：移除 `recompute_matches()` 调用
- [ ] 删除 `collect_matches_in_order()`、`jump_to_next_match()`、`expand_ancestor_prefixes()`
- [ ] 删除 `match_indices`、`current_match`、`path_to_walk_idx`、`path_to_tree_idx` 字段
- [ ] 删除 `SearchMatch` 类型

### Phase 4：清理（预估 1-2 天）

目标：删除所有旧代码和冗余模块

- [ ] 彻底删除 `tree_lines`、`expanded`、`filtered_tree_lines`
- [ ] 删除 `flatten_snapshot_tree()`、`enrich_snapshot_sizes()`
- [ ] 清理 `tree_ops.rs`：只保留删除相关函数
- [ ] 删除 `search.rs` 中不用的函数
- [ ] 清理 `types.rs`：删除 `TreeLine`、`SearchMatch`
- [ ] 更新 `lib.rs` 中的 `pub mod` 声明
- [ ] 全面检查 `clippy`

### Phase 5：渲染和测试（预估 2-3 天）

目标：UI 适配 + 全测试通过

- [ ] 调整百分比计算：子项大小 / 当前目录总大小
- [ ] 调整路径显示：顶部显示 `view_root/dir/subdir`
- [ ] 调整目录项视觉样式：`/` 后缀或颜色区分
- [ ] 适配多行渲染使用 `current_children` 而非 `tree_lines`
- [ ] 新增测试：
  - `test_enter_directory_updates_children`
  - `test_go_to_parent_restores_previous`
  - `test_search_only_current_dir`
  - `test_sort_only_current_level`
  - `test_percentage_of_current_dir`
  - `test_dir_stack_push_pop`
- [ ] 更新/删除旧测试
- [ ] 端到端集成测试

---

## 12. 测试策略

### 12.1 新增测试

| 测试名 | 测试内容 |
|--------|----------|
| `test_enter_directory_basic` | 进入目录后 `current_children` 内容正确 |
| `test_enter_directory_non_dir` | 在文件上按 `l` 不生效 |
| `test_go_to_parent_basic` | 返回上级后恢复前一个目录的内容 |
| `test_go_to_parent_at_root` | 在根目录按 `h` 无效果 |
| `test_go_to_root_clears_stack` | `H` 清空导航栈，回到根目录 |
| `test_dir_stack_depth` | 多次进入后退回路径正确 |
| `test_search_filters_children` | 搜索只过滤当前层 |
| `test_search_cycle_next_prev` | n/N 在匹配项间循环 |
| `test_search_empty_query` | 空搜索恢复全部显示 |
| `test_sort_by_name` | 名称排序 |
| `test_sort_by_size` | 大小排序 |
| `test_sort_by_delta` | Delta 排序 |
| `test_percentage_of_current` | 百分比 = 子项/当前目录 |
| `test_percentage_of_parent` | 百分比正确性验证 |
| `test_delete_refreshes_view` | 删除后 current_children 更新 |
| `test_hidden_files_toggle` | `.` 切换隐藏文件显示 |

### 12.2 需要重写的测试

| 旧测试 | 处理方式 |
|--------|----------|
| `test_expand_node_keeps_regular_dirs_marked_with_metadata` | 删除 |
| `test_collapse_or_navigate_up_*` (3 个) | 删除 |
| `test_collapse_all_children_*` (2 个) | 删除 |
| `test_navigate_up_root_basic` | 保留但语义改为回到 view_root |
| `test_delete_updates_parent_sizes_and_scan_cache` | 保留但简化 |
| `test_delete_file_under_root_keeps_scan_data_and_percentage` | 保留但百分比计算修改 |
| `test_enrich_snapshot_sizes_recurses_into_deep_children` | 删除 |
| `test_size_for_path_*` (3 个) | 删除 |
| `test_jump_to_next_match_*` (4 个) | 删除 |
| `test_recompute_matches_*` | 删除 |
| `test_expand_path_remaps_matches_without_clearing` | 删除 |
| `test_search_keys_*` | 需要全部重写 |

### 12.3 测试覆盖指标

| 模块 | 行覆盖率目标 |
|------|-------------|
| `load_current_children()` | ≥ 95% |
| `enter_directory()` | 100% |
| `go_to_parent()` | 100% |
| `apply_search()` | 100% |
| `cycle_match()` | 100% |
| `sort_children()` | 100% |
| `refresh_current_filtered()` | ≥ 90% |

---

## 13. 风险和缓解措施

| 风险 | 影响等级 | 概率 | 缓解措施 |
|------|---------|------|----------|
| 用户不适应从树形到平面的变化 | 中 | 中 | 顶部路径面包屑；`w` 快速切换根目录；help 文档更新 |
| 百分比语义变化导致用户困惑 | 低 | 高 | `%` 列标题用 tooltip 或底部提示说明"占当前目录" |
| 搜索范围缩小影响工作效率 | 中 | 中 | Phase 5 后考虑添加全局搜索浮层（可选） |
| `u` 键语义变化（本来跳到 filesystem 父目录） | 低 | 中 | 改为跳转到 view_root，filesystem 父目录用 `w` + `..` |
| 测试断裂 | 中 | 高 | 逐 Phase 实施，每个 Phase 结束时 `cargo test` 全绿 |
| 与 daemon 模式的交互（delta、filter） | 低 | 中 | Delta 查询机制不变，仅排序和过滤范围变化 |
| 多选（Tab）行为变化 | 低 | 低 | 多选仍作用于 `current_filtered`，语义不变 |

---

## 14. 未来扩展

### 14.1 全局搜索浮层

在平面模式稳定后，可添加全局搜索（方案 A）：
- 快捷键：`g/` 或 `Ctrl-s`
- UI：半屏浮层，显示全树匹配结果
- 选中后自动导航到目标目录
- 不打断当前浏览上下文

### 14.2 目录大小趋势

在百分比列旁添加趋势指示：
- `↑` / `↓` 箭头：对比上次扫描的大小变化
- 颜色：增长红、减少绿
- 数据来源：scan_cache 中的历史记录

### 14.3 自定义排序

为平面模式添加更灵活的排序选项：
- 按修改时间排序
- 按文件类型排序
- 按 Extension 分组排序

### 14.4 选择祖先快速导航

在顶部路径栏上支持点击/选中祖先路径快速跳转：

```
~/code/argus/src/handler/  ← 点击 ~/ 跳转到 home
```

类比 `vim` 的 `gf` 或编辑器中的路径面包屑。