# SQLite Storage Backend 实施清单

> 用途：指导 AI agent 按 [`sqlite-storage-backend.md`](./sqlite-storage-backend.md) 继续实现后续工作。
>
> 范围：当前实施轨道只考虑 SQLite 作为主存储，不做旧 JSON 迁移。

## 0. 开工前确认

- [ ] 先阅读 [`docs/plans/sqlite-storage-backend.md`](./sqlite-storage-backend.md)。
- [ ] 明确当前目标是 SQLite-first，不再沿用旧的 JSON 文件方案。
- [ ] 不实现 JSON 数据迁移，也不为迁移预留额外复杂度。
- [ ] 只做当前需求需要的抽象，不提前设计未来未用到的框架。
- [ ] 遵守仓库约束：`cargo fmt`、`cargo clippy`、`cargo test` 通过。

## 1. `argus-core`：SQLite 数据层

### 1.1 依赖与模块

- [ ] 在 `argus-core/Cargo.toml` 中加入 `rusqlite`，优先使用 bundled sqlite。
- [ ] 新建 `argus-core/src/db.rs`。
- [ ] 在 `argus-core/src/lib.rs` 导出 `db` 模块和公开 API。

### 1.2 数据表与初始化

- [ ] 实现数据库初始化逻辑。
- [ ] 建立 `scan_events` 表。
- [ ] 建立 `path_records` 表。
- [ ] 建立必要索引。
- [ ] 保证数据库可重复初始化，且不会破坏已有数据。

### 1.3 写入流程

- [ ] 从 `Snapshot` 写入一条 `scan_events` 记录。
- [ ] 将 `FileNode` 树拍平成 `PathRecord`。
- [ ] 写入根节点与所有子节点。
- [ ] 使用事务保证单次扫描写入原子性。
- [ ] 处理绝对路径、文件类型、修改时间、inode、device 等字段。

### 1.4 查询流程

- [ ] 实现按时间点定位最近扫描的逻辑。
- [ ] 实现 `query_delta`，支持 `from` / `to` 时间范围。
- [ ] 实现 `query_scan_timestamps`，用于列出某个 root 的扫描历史。
- [ ] 实现 `query_root_summaries`，用于列出所有 root 的扫描概览。
- [ ] 实现 `rebuild_snapshot`，可从最新扫描重建内存快照。
- [ ] 实现 `build_diff_tree`，将扁平 delta 还原为树形差分结果。

### 1.5 Core 测试

- [ ] 写 `write_scan` 和 `rebuild_snapshot` 的 round-trip 测试。
- [ ] 写 delta 查询的新增 / 删除 / 修改测试。
- [ ] 写空树和单文件树测试。
- [ ] 写时间点解析的 fallback 测试。
- [ ] 写 root summary 查询测试。
- [ ] 确保 core 测试覆盖 root 节点、目录节点和路径前缀边界。

## 2. CLI：新命令形态

### 2.1 命令结构

- [ ] 保持 CLI 入口为新形态，不恢复旧的文件快照命令。
- [ ] `scan --path <PATH>` 写入 SQLite。
- [ ] `diff --path <PATH> --from <RFC3339> --to <RFC3339>` 做时间范围对比。
- [ ] `list-scans [--path <PATH>]` 列出 root 的扫描历史或 root 概览。

### 2.2 交互与输出

- [ ] 支持 `text`、`json`、`markdown` 输出格式。
- [ ] `--threshold` 继续支持人类可读大小字符串。
- [ ] CLI 帮助文案反映 SQLite-first 的实际行为。
- [ ] 退出码与当前 requirements 保持一致或在文档中明确更新。

### 2.3 CLI 测试

- [ ] 为 `scan` / `diff` / `list-scans` 增加集成测试。
- [ ] 验证 `scan -> diff -> list-scans` 的最小闭环。
- [ ] 验证时间范围、阈值和输出格式参数。

## 3. TUI：SQLite 读写与视图刷新

### 3.1 启动流程

- [ ] 启动时从 SQLite 加载 root 概览。
- [ ] 当前工作目录有历史记录时，直接恢复最新快照。
- [ ] 没有历史记录时，回退到实时文件系统浏览。

### 3.2 连接模型

- [ ] 不在 `App` 内持久持有 `rusqlite::Connection`。
- [ ] 查询和写入都通过短生命周期连接执行。
- [ ] 后台任务负责阻塞型 SQLite 操作。

### 3.3 视图刷新

- [ ] 时间范围筛选触发 `query_delta`。
- [ ] 将 delta 结果交给 `build_diff_tree` 渲染。
- [ ] 扫描完成后刷新当前 root 的缓存与视图。

### 3.4 TUI 测试

- [ ] 验证启动后 root 概览可加载。
- [ ] 验证 active root 可从 SQLite 恢复。
- [ ] 验证 diff 模式在时间范围变化时会刷新。

## 4. 文档同步

- [ ] 如果改了数据结构，回同步 [`docs/requirements/08-data-model.md`](../requirements/08-data-model.md)。
- [ ] 如果改了 CLI 交互，回同步 [`docs/requirements/05-ux-interaction.md`](../requirements/05-ux-interaction.md)。
- [ ] 如果改了配置或默认路径，回同步对应 requirements 和 README。
- [ ] 如果命令帮助文案变了，保持 `--help`、README 和计划文档一致。

## 5. 完成标准

- [ ] `cargo test` 通过。
- [ ] `cargo clippy` 无警告。
- [ ] `cargo fmt --check` 通过。
- [ ] CLI 命令形态与文档一致。
- [ ] SQLite 成为当前轨道的唯一持久化实现。
- [ ] 不存在为 JSON 迁移保留的未完成实现。

## 6. 不要做的事

- [ ] 不要恢复旧的 JSON 快照主方案。
- [ ] 不要引入当前阶段不需要的抽象层。
- [ ] 不要把 `Connection` 长期塞进 UI 状态里。
- [ ] 不要跳过测试直接扩功能。
- [ ] 不要在 core 里引入 GUI/TUI 依赖。
