# 核心功能需求

## 1. 时间差分磁盘分析引擎 (Time Diff Engine)

### 1.1 基准扫描

- 使用 `ignore` 库扫描指定目录，在内存中构建 `FileTree`。Phase 1 以 [10-phase1-guide.md](10-phase1-guide.md) 的同步 `Walk` 实现为准；多线程 `WalkParallel` 是后续性能优化目标。
- 自动尊重 `.gitignore` 规则（基于 `ignore` 库）。
- 支持扫描可中断：用户按下 Esc/Ctrl+C 时能立刻停止，释放内存。
- 支持自定义忽略规则（参见 [04-configuration.md](04-configuration.md)）。
- **符号链接**：默认不跟随（`follow_symlinks = false`），避免循环链接导致无限递归或重复统计。可选开启跟随。
- **硬链接**：同一 inode 被多路径引用时，仅统计一次体积，避免重复计算。通过对比 `(device, inode)` 元组实现 dedup。
- **特殊文件**：管道（FIFO）、socket、设备文件等计入 `is_dir = false`，`size = 0`，并标记 `file_type` 字段供筛选。

### 1.2 快照系统

- 将单次扫描结果序列化为 JSON 快照文件。
- 快照存储位置：`~/.config/argus/snapshots/`。
- 快照包含：时间戳、根路径、总大小、完整目录树。
- MVP 阶段使用 JSON（便于 Debug），后期可迁移至二进制格式以提升性能。

### 1.3 时间差分查询

Delta 计算有两种实现路径：

| 实现 | 适用阶段 | 原理 |
|------|---------|------|
| **快照对比（Snapshot Diff）** | Phase 1 CLI / Phase 2 TUI 独立模式 | 加载两个 JSON 快照，Tree Merge 算法在内存中计算 diff。仅支持"固定两个时间点"对比，且必须属于同一树根路径 |
| **时序查询（Time-Series Query）** | Phase 3+ Daemon 模式 | 数据库存储 `(path, size, timestamp)` 记录。支持任意时间范围聚合查询，如 `-2h`、`上周一到这周一` |

两种实现共享相同的 Tree Merge 核心算法。时序查询是快照对比的增强版——将"一次性对比两个全量快照"替换为"按需从数据库聚合子树 delta"。

**Phase 2 独立模式使用快照对比**：TUI 启动时先根据最近一次扫描结果确定当前树根路径，再在该树根路径对应的快照中做 diff。用户扫描两次后，时间选择器列举该树根路径下的可用快照时间戳，用户手动选择 from/to 两个时间点。

### 1.4 Delta 作为筛选器覆盖层

Delta 不是独立的"模式"，而是文件树上的一个**可视化筛选层**：

```
┌────────────────────────────────────────────┐
│  时间: [2026-06-01 → 2026-07-01]  Δ≥ [50 MB]  [清除] │
├────────────────────────────────────────────┤
│  name              size        delta       │
│  ├── Desktop/     2.1 GB     +500 MB      │
│  ├── Downloads/   4.4 GB     +1.2 GB      │
│  │  ├── big.iso   2.0 GB     +2.0 GB      │
│  │  └── ...                                │
└────────────────────────────────────────────┘
```

- **无筛选条件时**（时间为空、阈值为空）：纯 ncdu 模式，文件树只显示 `name + size`，无 delta 列
- **有时间选择时**：树上的文件名旁显示 delta 值。根树节点从快照/数据库获取 delta，子节点按相同时间范围计算
- **有阈值筛选时**：delta 绝对值小于阈值的节点隐藏（或灰显），只突出显著变动

### 1.5 聚合归纳

- 若父目录下 1000 个日志文件各涨 1MB，应在父目录上显示 `+1GB`，而非列出 1000 行。
- 自动合并同后缀类型文件的变动，提供文件类型维度的聚合视图。

## 2. 守护进程 (argusd) - Phase 3

### 2.1 首次冷启动

- 对配置的用户目录进行全盘极速扫描，建立基准索引。

### 2.2 事件驱动更新

- 利用系统级 API 捕获文件变更：
  - macOS: FSEvents
  - Linux: inotify
- 捕获的事件类型：创建、修改、删除。

### 2.3 漏斗去抖 (Debounce)

- 连续大量文件变动（如 `cargo build` 写入 `target/`）需在内存中合并。
- 延迟 5-10 秒后一次性写入轻量数据库。
- 避免高频磁盘 I/O。

### 2.4 时间轴快照 (Timeline)

- 默认按固定周期归档目录树快照。
- 可配置保留策略：
  - 保留 7 天内的每小时快照。
  - 保留 30 天内的每日快照。
- 允许用户查询任意历史时间点的目录状态。

### 2.5 监控目录变更

当用户修改 `config.toml` 中的 `[daemon].watch_dirs` 时：

| 变更类型 | 行为 |
|---------|------|
| **新增目录** | 对该目录执行冷启动扫描，建立基准索引。新旧目录的快照通过 `root_path_hash` 隔离，互不影响 |
| **删除目录** | 停止监控该目录，已有历史快照保留但不再更新。TUI 仍可加载历史快照查看历史 diff |
| **修改目录路径** | 旧路径的快照与新路径的 `root_path_hash` 不匹配，`compare_trees` 返回 `RootPathMismatch` 错误，提示用户重新扫描。**不允许不同根路径的快照做 diff** |

### 2.6 轻量化原则

- 闲置时 CPU 占用接近 0%。
- 内存占用控制在 30MB 以内。
- 遵循"低优先级 I/O"原则，在用户高负载时自动让出硬件资源。

## 3. 安全删除机制

### 3.1 硬编码黑名单 (System Shield)

以下系统关键目录严禁在任何客户端中显示"删除"动作。受保护目录列表以 [07-safety.md](07-safety.md) §2.1 为唯一权威来源，核心功能文档不重复维护清单，避免实现时出现安全规则分叉。

### 3.2 两阶段确认

- **普通目录**：TUI 弹出二次确认框。
- **高危/系统关联目录**：由 AI 判定为高风险时，强制要求长按快捷键 3 秒确认，或手动输入 `YES`。

### 3.3 废纸篓机制

- 优先调用系统原生 API 将文件移至 Trash（废纸篓），而非硬性 `rm -rf`。
- 为用户留一颗"后悔药"。

## 4. 高性能与可中断扫描

### 4.1 扫描性能

- Phase 1 使用 `ignore` 库的同步 `Walk`，优先保证行为简单可测。
- 后续可切换到 `WalkParallel` 做多线程并行扫描，以提升大目录性能。
- 自动过滤 `.gitignore` 中标记的临时文件。

### 4.2 取消机制

- 使用 `AtomicBool` 共享取消标志（Phase 1 同步阶段适用，Phase 3 Daemon 模式可改用 `tokio::sync::oneshot`）。
- 扫描循环内部定期（如每扫描 1000 个文件）检查 `is_cancelled` 标志位。
- 收到取消信号后立刻退出并释放内存。Phase 1 不返回部分快照，避免持久化不完整目录树；如后续需要渐进式扫描结果，应新增显式的 `ScanOutcome::Cancelled { partial, stats }` 类型。

**与 `ignore::WalkBuilder` 集成的具体方案**：

`ignore::WalkBuilder` 是同步迭代器，没有原生取消接口。实现方式有两种：

**方案 A（Phase 1 推荐）**：使用 `Walk::filter_entry` 回调检查取消标志。

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use ignore::WalkBuilder;

fn scan(path: &Path, cancel: &AtomicBool) -> Result<Snapshot, ScanError> {
    let mut file_count = 0;
    let mut entries = Vec::new();

    let walker = WalkBuilder::new(path)
        .filter_entry(move |_| {
            file_count += 1;
            if file_count % 1000 == 0 {
                !cancel.load(Ordering::Relaxed) // 每 1000 文件检查一次
            } else {
                true
            }
        })
        .build();

    for entry in walker {
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }
        // ... 正常处理 entry
    }
    // ... 构建树
}
```

**方案 B（高性能场景）**：使用 `ignore::WalkParallel` + `ParallelVisitor`，在 `visit` 回调中检查标志。适用于超大规模目录。

> **注意**：`filter_entry` 在目录进入前调用，因此无法在扫描中途取消（需等当前目录遍历完）。如果对取消响应时间有更高要求（如 TUI 追求毫秒级响应），应使用方案 B + 跨线程通知。

### 4.3 全异步操作

- TUI 中所有扫描、Diff 计算、AI 请求均在后台线程/Tokio 任务中进行。
- TUI 界面保持 60 帧流畅，不阻塞用户交互。

## 5. 边界约束与降级策略

### 5.0 文件树与 Delta 覆盖层

TUI 展示一个统一的文件树，delta 是树上的可选的筛选覆盖层，不是独立模式：

```
TUI 启动 → 加载所有快照到 scan_cache → 确定 cwd 作为树根
                │
                ├── scan_cache 有 cwd 的数据 → 渲染完整数据树（size + children）
                │
                ├── scan_cache 无 cwd 的数据
                │       └── 配置 auto_scan_on_start = true
                │               → 后台扫描 cwd → 完成后渲染完整数据树
                │       └── 配置 auto_scan_on_start = false
                │               → list_dir(cwd) → 渲染 FS 树（目录 size="-"，文件有真实 size）
                │               → 用户按 s → scan_path(cwd) → 保存 + 更新缓存 → 渲染完整数据树
                │
                └── 用户导航到其他目录（l/h 切换树根）
                        ├── scan_cache 有该目录 → 渲染完整数据树
                        └── scan_cache 无该目录 → list_dir 惰性读取 → 渲染 FS 树
```

**关键设计决策**：

1. **文件树永远存在**：以 cwd 为根，始终可自由游走。扫描是增强，不是前提。

2. **两层数据驱动**：
   - FS 层（`list_dir`）：懒加载目录结构，文件展示真实 size
   - Scan 层（`scan_cache`）：全量扫描后才有汇总 size / children / delta

3. **没有"路径选择器"**：按 `s` 不再弹输入框，直接扫描当前树根（cwd）。要切换扫描目标，先导航到目标目录再按 s。

4. **Delta 是筛选条件，不是模式**：时间选择器为空 = 无 delta 列。选择了时间范围 = 显示 delta。仅对当前树根路径的历史快照生效。

5. **统一数据模型（Phase 3+）**：数据库以 `(path, size, timestamp)` 三元组存储。TUI 可以查询任意子树在任意时间范围的 delta，无需"两个快照"的概念。Phase 2 通过 JSON 快照对比近似实现这一体验。

6. **独立模式 vs 服务模式**：两种模式都使用相同的 tree + delta filter 界面，只是 delta 数据的来源不同（JSON diff vs 数据库聚合）。

### 5.1 百万级文件目录

当扫描目标包含 100 万+ 文件时：
- 内存预算：每个 `FileNode` 约 200-400 字节，100 万节点约 200-400MB，需在启动时预检剩余内存。
- 扫描进度：每处理 10,000 个文件推送一次进度更新，避免进度更新本身成为性能瓶颈。
- 降级策略：如果总文件数超过 `max_file_limit`（默认 500,000），发出警告并询问用户是否继续。

### 5.2 大文件与溢出保护

- `u64` 可表示最大 18.4 EB（exabytes），个人桌面场景不会溢出。但仍需保护 `total_size` 累加时不触发 panic（使用 `saturating_add`）。
- 单文件 > 4GB 时，`Metadata::len()` 返回 `u64`，无溢出风险。

### 5.3 扫描耗时保护

- 超过 30 秒的扫描自动启用进度指示器。
- 扫描过程中每 5 秒输出一次心跳日志（`scanned: 50000 files, 2.3GB`）。
- 极端情况下（如 NFS 挂载、网络驱动器），`ignore` 库可能超时。此时应跳过该路径并记录错误，而非阻塞整个扫描。
