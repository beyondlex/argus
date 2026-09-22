# 代码审查记录 — 2026-09

全库检视（argus-core / argus-cli / argus-tui / argusd）的结果归档。
已修复的问题有对应 commit；"存疑/记录" 类问题保持现状，等出现实际症状或专门排期再处理。

## 已修复（本轮）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| 17 个 TUI 测试自 102024f 起失败：测试手工构造 tree_root 但未注册 scan_cache，被 `load_current_children` 根层重建 `list_dir` 回退清空 | `app_tests.rs` `tree_ops.rs` | 测试基建与生产路径脱节 | d10c82f |
| delta 查询反连接子查询不带时间过滤：agg 行在窗口外时其子路径的窗口内事件被抑制而 agg 本身不计入 → **少算** | `argus-core/db.rs` `query_delta_total/detail` | delta 数字不准 | 33069ba |
| 硬链接去重 `continue` 把第二个链接从树里整个删掉 → 文件在浏览视图中**消失** | `argus-core/scanner.rs` | 扫描结果缺文件 | 4ca4a16 |
| TUI purge 扫描硬编码 `[HOME]` 作唯一 root，只扫 HOME 一级 → `~/Projects/x/node_modules` 等扫不到 | `argus-tui/app.rs` | purge 功能形同虚设 | 247c552 |
| 搜索跳转与 delta 过滤坐标系混用（children 索引 vs filtered 索引）→ 过滤激活时 n/N 跳错行 | `argus-tui/app.rs` | 交互 bug | f9fe564 |
| 进废纸篓删除逐文件 collect+trash：大目录慢、废纸篓里碎成大量重名条目 | `argus-tui/handler/prompt.rs` | 性能+体验 | 9a30134 |
| debounce `modify→delete` 合并用 `-abs()` 丢弃 modify 增量 → 长大后删除多计、缩小后删除少计 | `argusd/debounce.rs` | daemon 记账错误 | cb2e4b1 |
| IPC 帧长度不设上限，畸形客户端可触发 4GB 分配；`RequestConsolidation` 硬编码 threshold=500 不读配置 | `argusd/ipc_server.rs` | 健壮性 | 0a63ee2 |
| `Watcher::new().expect()` panic 会静默杀死监控线程 | `argusd/watcher.rs` | 可靠性 | 847350d |
| `[daemon.snapshot_retention]` 配置被解析但从未使用（快照持久化特性本身不存在），已随文档一并移除 | `argusd/config.rs` + docs | 幽灵配置 | 351c961 |

另：clippy 全仓清零（此前约 60 条）、`status_bar::render` 19 个位置参数收敛为 `StatusCtx`、`size_for_path/disk_usage_for_path` 合并为 `metric_for_path`、AI 缓存 path 哈希去重（5 处拷贝 → `path_hash()`）。

## 存疑 / 记录在案（未改动）

### 1. `is_ignored` 对 `.ds_store` 的豁免像是写反了
`argusd/src/watcher.rs`：`name.starts_with('.') && name != ".ds_store"` —— 意味着所有点文件被忽略，**唯独 .DS_Store 被跟踪**。Finder 会频繁重写 .DS_Store，这通常正是想过滤的噪音。如果是有意为之（想看 Finder 行为）请加注释说明，否则应改为 `|| name == ".ds_store"`。

### 2. 聚合行覆盖下，新事件的可见性滞后
consolidate 之后某子树已有 agg 覆盖行，新到达的子路径事件在查询层被 agg 反连接抑制，但要等下一次 consolidation（默认间隔 60 分钟）才会折入 agg。期间 TUI delta 面板看不到这部分变化。可选修法：新事件插入时若祖先存在 agg 行，直接以小粒度行写入并在查询层做"agg 截止到 agg.ts，其后事件单独计"的语义；或缩短 consolidate 间隔。

### 3. SQLite `LIKE` 对 ASCII 不区分大小写
`query_delta_total/detail` 的路径前缀匹配 `path LIKE '/users/%'` 会命中 `/Users/...`。macOS 默认文件系统大小写不敏感，跨大小写路径冲突的场景罕见，但理论上会造成串数据。若要严格，可改 `GLOB` 或 `PRAGMA case_sensitive_like`。

### 4. 非 ASCII 搜索高亮可能偏移 1 个字符位
`argus-tui/search.rs` `fuzzy_match_indices` 非 ASCII 分支用 `to_lowercase()` 后的字符数换算高亮区间；个别字符（如 `İ`）小写化后字符数会变，高亮宽度随之偏移。文件名场景极少见，暂不处理。

### 5. 多选删除静默丢弃与根同名的选中项
`handle_multi_delete_action` 按 `file_name() != root_name` 过滤，与根目录同名的普通文件/目录被无声剔除。**已部分改善**（8f45519）：状态栏现在会提示跳过了多少项；但"与根同名就剔除"这条规则本身仍偏保守，单选删除同路径会显式报错，规则上仍不一致。

### 6. `build_current_tree` 失败即清空视图
view_root 无 scan_cache 且 `list_dir` 失败（权限/被删）时置 `tree_root = None` 并报错，当前浏览内容全部清空。行为可以接受，但"保留旧树 + 报错"体验更好。

### 7. argusd PID 文件复用风险
`stop()` 按 PID 文件 kill，若守护进程异常退出后 PID 被其他进程复用，会误杀。个人工具风险可接受；正规做法是 pidfd / kill 前校验进程名。

### 8. `find_artifacts` 只扫每 root 一级
`~/Projects/2024/app/node_modules` 这类两层嵌套项目扫不到（一层：root 的子目录下的 artifact 目录）。默认 roots（Projects/GitHub/dev…）下再嵌一层项目目录的场景会漏。可考虑限深 2–3 层的目录遍历。

### 9. 架构坏味道：`argus-tui/app.rs` 约 1900 行
超出 AGENTS.md 500 行约定数倍。合理的拆分线已经存在：flat-mode 导航（enter/go_to_parent/go_up_fs/nav_*）、消息处理（handle_message）、AI review 状态机、清理扫描。建议在下次功能性改动顺手拆，避免专门一次大重构。

## 性能观察（评估过，暂不动）

- `has_ai_analysis_batch` 逐 path 一条 SQL；批量为个位数～几十条，开销可忽略。若将来批量上千，改临时表 JOIN。
- `prune_file_node` 删除后对每个祖先做整子树尺寸重算，O(深度 × 子树)。交互式删除频率低，正确性优先，暂保留。
- purge 发现阶段对每个命中 artifact 做完整 `dir_size` 递归（可能很大），但在后台线程执行且结果要展示大小，属必要成本。
- `Snapshot::child_idx` 为 CSR 区间线性扫描；单目录子项数千以内无感。如遇超 fan-out 目录再考虑加索引。
- TUI 渲染层 `right_width` 逐帧重算等微开销，远低于渲染阈值，不值得优化。
