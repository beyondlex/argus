# Argus TUI: 平面目录浏览模式（ncdu 式）实现文档

> 版本：v2.0  
> 日期：2026-07-16  
> 状态：已实现（flat 分支）

---

## 1. 设计目标

### 1.1 核心思路

将树形展示模型替换为 ncdu 式的**平面浏览模型**：

- 一次只显示**当前目录的直接子项**
- 进入子目录 = **重新锚定视图**到该目录（而非原位展开）
- 返回上级 = **从导航栈弹出**（而非折叠）
- 搜索只高亮**当前可见列表**中的匹配项，不隐藏非匹配项

### 1.2 性能提升

| 指标 | 树形模式（旧） | 平面模式（当前） |
|------|-------------|----------------|
| 展开目录 | O(N) 全量展平 | O(C) 加载子项 |
| 排序切换 | O(N log N) | O(C log C) |
| 搜索 | O(N) + 全树匹配 | O(C) + 高亮不过滤 |
| 搜索过滤 | 隐藏非匹配项 | 高亮匹配项，非匹配项保留可见 |
| 删除后刷新 | O(N) 全量重建 | O(C) 重新加载子项 |
| 内存占用 | N × TreeLine | C × DirEntry |

**N** = 总文件数，**C** = 当前目录子项数（通常 10~1000）

---

## 2. 数据结构

### 2.1 `DirEntry`（当前实现）

```rust
// argus-tui/src/types.rs
pub struct DirEntry {
    pub node: TreeNode,
    pub path: Vec<String>,
    pub has_scan_data: bool,
    pub is_dir: bool,
    pub size: u64,          // 逻辑大小（文件实际字节数）
    pub disk_usage: u64,    // 磁盘占用（blocks × block_size）
}
```

`disk_usage` 是新增字段，用于在 status bar 中同时显示 Disk Usage 和 Apparent Size。

### 2.2 App 字段（实际实现）

```rust
// argus-tui/src/app.rs
pub struct App {
    // ── 平面模式字段 ──────────────────────────────────
    pub current_children: Vec<DirEntry>,       // 当前目录子项列表
    pub current_filtered: Vec<usize>,          // 过滤后索引
    pub current_dir_path: Vec<String>,          // 当前目录相对路径
    pub dir_stack: Vec<Vec<String>>,            // 导航栈
    pub current_dir_total: u64,                 // 当前目录逻辑总大小
    pub current_dir_disk_usage: u64,            // 当前目录磁盘占用
    pub current_dir_items: u64,                 // 当前目录可见子项数
    pub parent_dir_total: u64,                  // 父目录总大小
    pub search_match_indices: Vec<usize>,       // 搜索匹配索引（n/N 导航用）

    // ── 已删除的旧字段 ────────────────────────────────
    // tree_lines, expanded, filtered_tree_lines,
    // match_indices, current_match, path_to_walk_idx, path_to_tree_idx
}
```

---

## 3. 导航模型

### 3.1 键位映射

| 按键 | 功能 |
|------|------|
| `j` / `↓` | 向下移动光标 |
| `k` / `↑` | 向上移动光标 |
| `l` / `→` | 进入选中目录，push 导航栈 |
| `h` / `←` | 返回上级目录，pop 导航栈 |
| `H` | 返回 `view_root`，清空导航栈 |
| `u` | 在树内：返回父级；在树根：切换到 filesystem 父目录 |
| `w` | 将当前目录设为新的 `view_root` |
| `g` | 跳到第一项（双击跳回顶部） |
| `G` | 跳到最后一项 |
| `o` | 循环切换排序：Name → Size → Delta |
| `.` | 切换隐藏文件显示 |
| `Tab` | 进入/退出多选模式 |
| `s` | 扫描当前目录 |
| `/` | 搜索（高亮匹配，不隐藏非匹配） |
| `?` | 帮助面板 |
| `d` / `D` | 删除（废纸篓 / 永久） |
| `i` | 显示文件/目录元信息 |
| `K` | 显示 Delta 详情弹窗 |
| `y` | 复制选中路径到剪贴板 |
| `:`  | 命令模式 |
| `t` | 循环切换时间范围（daemon 模式） |
| `c` | 清除 delta filter |
| `q` / `Esc` | 退出 / 取消 |

### 3.2 进入目录流程

```rust
pub fn enter_directory(&mut self) {
    // 校验：选中项存在且为目录
    // 如果在 scan_cache 中有针对该子目录的扫描结果，则切换 view_root 到该子目录
    // 否则：push 导航栈 → 更新 current_dir_path → reload children
}
```

### 3.3 返回上级流程

```rust
pub fn go_to_parent(&mut self) {
    // 已在根目录 → return
    // 从 dir_stack pop 恢复前一个路径 → reload children
}
```

### 3.4 向上导航流程（u 键）

```rust
pub fn go_up_fs(&mut self) {
    // 不在根目录 → 调用 go_to_parent()
    // 在根目录 → view_root_path = parent()，rebuild_tree()
}
```

---

## 4. 数据加载：`load_current_children()`

### 4.1 核心逻辑

```rust
pub fn load_current_children(&mut self) {
    // (0) 惰性填充：如果当前目录在 Snapshot 中为空，调用 list_dir() 填充
    // (1) 定位当前目录节点
    // (2) 解析 scan tree（查找最近的父级扫描结果）
    // (3) 遍历子项，跳过隐藏文件（需显示时除外）
    // (4) 对每个子项查询 has_scan_data
    // (5) 排序（sort_children）
    // (6) 更新 total / disk_usage / items
    // (7) 重新应用 delta filter → refresh_current_filtered()
}
```

### 4.2 惰性目录填充

当进入一个在 Snapshot 中为空（`children.is_empty()`）的目录时，调用 `list_dir()` 从文件系统动态填充子项。这在未扫描的目录中保持导航能力。

### 4.3 复杂度

| 步骤 | 复杂度 |
|------|--------|
| 惰性填充 | O(C) — 仅对空目录 |
| 定位节点 | O(L) — L = 路径深度 |
| 遍历子项 | O(C) |
| 排序 | O(C log C) |
| **总计** | **O(C log C)** |

---

## 5. 搜索模型

### 5.1 搜索不隐藏非匹配项

关键变更：搜索**不高亮**匹配项，但**不隐藏**非匹配项。这是与原始设计的差异：

```rust
pub fn apply_search(&mut self) {
    // 在 current_children 中查找名称匹配 fuzzy_match_indices 的项
    // 结果存入 search_match_indices（供 n/N 导航）
    // refresh_current_filtered() 不应用搜索过滤
    // 光标跳到第一个匹配项
}
```

`n` / `N` 在 `search_match_indices` 间循环跳转，而非在 `current_filtered` 内。

### 5.2 渲染

- 匹配项：名称高亮（绿色 + Bold）
- 非匹配项：正常显示（dimmed）
- 光标所在行的匹配项：使用 `search_match_selected_bg` 背景色 + `match_bg` 区分关键词匹配部分

---

## 6. 排序

```rust
pub fn sort_children(children: &mut Vec<DirEntry>, mode: SortMode, delta_cache: &HashMap<Vec<String>, i64>) {
    match mode {
        SortMode::Name => children.sort_by(|a, b| a.node.name().cmp(b.node.name())),
        SortMode::Size => children.sort_by(|a, b| b.disk_usage.cmp(&a.disk_usage)...),
        SortMode::Delta => children.sort_by(|a, b| delta绝对值降序...),
    }
}
```

Size 模式按 `disk_usage`（磁盘占用）而非 `size`（逻辑大小）排序。

---

## 7. 渲染

### 7.1 布局

```
┌─ ~/code/argus/src ──────────────────────────────┐  ← Title bar（面包屑）
│  handler/     +2.1 MB   42.1%  12.1 KB  ●       │  ← 文件列表
│  main.rs         -       11.1%   3.2 KB          │→ 光标行
│  lib.rs          -        7.3%   2.1 KB          │
├──────────────────────────────────────────────────┤
│  [/ to search]  6 items          Sort: Size      │  ← Status bar
└──────────────────────────────────────────────────┘
```

### 7.2 列顺序（从左到右）

| 列 | 内容 | 说明 |
|----|------|------|
| 名称 | `entry.node.name() + /`（目录） | 隐藏文件用 `theme.hidden` 颜色 |
| Delta | `+1.2 MB` / `-500 KB` / `-` | 仅 daemon 模式显示 |
| 百分比 | `42.1%` | 相对于当前目录的 `disk_usage` |
| 大小 | `12.1 KB` | 显示 `disk_usage`（扫描后有数据）或 `size`（未扫描） |

### 7.3 Status Bar

```
  Disk: 12.1 GB Apparent: 11.5 GB Items: 1,234    Sort: Size
  ├── 当前目录磁盘占用     ├── 当前目录逻辑大小    ├── 子项计数
```

### 7.4 扫描摘要

扫描完成后，摘要信息显示在 Title Bar 中（而非 status bar），包含：
- 路径
- 总大小 / 磁盘占用
- 文件数
- 耗时

---

## 8. Delta 和过滤

### 8.1 Delta 数据到达

```rust
AppMessage::DeltaData(deltas, returned_client) => {
    self.delta_cache = deltas;
    self.load_current_children();  // ← 替代 update_tree_lines()
}
```

### 8.2 Delta 过滤（仅限 daemon 模式）

`refresh_current_filtered()` 根据 `delta_filter_active` 过滤 `current_filtered`：
- `delta_filter_value = 0`（严格模式）：只保留 `delta > 0` 的项
- `delta_filter_value > 0`：保留 `delta >= threshold` 的项

搜索词不再用于过滤 `current_filtered`。

---

## 9. 多选与删除

### 9.1 多选（Tab）

按 `Tab` 进入多选模式：
- `Tab`：选中当前项并跳到下一项
- `d` / `D`：批量删除（废纸篓 / 永久）
- `Esc`：退出多选模式

### 9.2 删除后刷新

```rust
AppMessage::DeleteComplete { errors, paths } => {
    // 调用 apply_deletion_to_state 处理每个路径
    // （从 snapshot 中移除节点，更新父级 size）
    self.load_current_children();  // ← 替代 update_tree_lines()
    self.exit_multi_select();
}
```

---

## 10. 已删除的旧代码

| 模块/函数 | 行数 | 替代 |
|-----------|------|------|
| `file_tree.rs`（整个文件） | 828 | `flat_tree.rs` |
| `handler/filter.rs`（整个文件） | 130 | 内联到 `app.rs` |
| `flatten_snapshot_tree()` | ~60 | `load_current_children()` |
| `update_tree_lines()` | ~110 | 删除 |
| `rebuild_tree()`（旧版） | — | 简化版保留 |
| `collect_matches_in_order()` | ~65 | 删除（搜索不过滤） |
| `jump_to_next_match()` | ~70 | `cycle_match()` |
| `recompute_matches()` | ~40 | 删除 |
| `TreeLine` 类型 | — | `DirEntry` |
| `SearchMatch` 类型 | — | `search_match_indices: Vec<usize>` |
| `path_to_walk_idx` / `path_to_tree_idx` | — | 删除 |

---

## 11. 遗留兼容

### 11.1 `u` 键语义复合

```rust
pub fn go_up_fs(&mut self) {
    if self.current_dir_path.len() > 1 {
        self.go_to_parent();  // 在树内 → 返回父级
        return;
    }
    // 在树根 → 切换到 filesystem 父目录
    let parent = self.view_root_path.parent().map(|p| p.to_path_buf());
    // rebuild_tree() + load_current_children()
}
```

### 11.2 子目录扫描切换

如果用户在子目录上按 `s` 扫描，`enter_directory()` 会检查 `scan_cache`。如果目标子目录已有扫描缓存，直接切换 `view_root` 到该扫描结果。

### 11.3 删除操作的 scan_cache 保护

删除操作会剪枝（prune）而非移除父级 scan_cache 条目，确保 `resolve_scan_tree()` 在子目录视图中仍能找到祖先扫描数据。

---

## 12. 测试覆盖

| 测试文件 | 覆盖内容 |
|---------|---------|
| `tree_ops.rs` 内联测试 | `enrich_snapshot_sizes`, `size_for_path`, 删除操作 |
| `app.rs` 测试 | `load_current_children`, `enter_directory`, `go_to_parent`, `apply_search`, `cycle_match`, 排序, 多选 |
| 集成测试 | 端到端 CLI/TUI 流程 |
