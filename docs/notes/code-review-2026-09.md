# 代码审查记录 — 2026-09

全库检视（argus-core / argus-cli / argus-tui / argusd）的结果归档。
已修复的问题有对应 commit；"存疑/记录" 类问题保持现状，等出现实际症状或专门排期再处理。## 已修复（第一轮）

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

## 已修复（第二轮）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| 两级目录各自 consolidate 后，子孙 agg 行被祖先 agg 行在查询反连接中抑制；而 consolidation 是父级局部的（祖先 agg 只汇总直接子级，从不吸收子孙 agg）→ 子树增量**永久丢失** | `argus-core/db.rs` `query_delta_total/detail` | delta 记账错误（嵌套聚合场景） | 5fd47ec |
| debounce 兜底分支对 `delete→create`（文件替换/重建）直接覆盖旧 entry → 删除的负增量被丢弃，100B 删+50B 重建记成 +50 而非 -50 | `argusd/debounce.rs` `merge` | daemon 记账错误 | afc91c8 |
| 默认 watch_dirs 硬编码 `/Users/lex/...` → 换机器守护进程什么都不监控；config 解析失败仅发 warn（此时 tracing 未初始化）→ **静默**回退默认 | `argusd/config.rs` `main.rs` | 可移植性 + 可诊断性 | d08c4a1 |
| 点文件过滤豁免 `.DS_Store` —— 恰是 Finder 频繁重写的噪音源（疑似写反，按噪音处理）；目录参与尺寸记账户，目录 modify 事件把 ~4KiB 的目录 stat 大小灌进 delta | `argusd/watcher.rs` | delta 噪音 | a124e33 |
| `analyze_batch` 对网络错误 `?` 直接传播，完全不重试（与"带重试"的注释不符）；超时/连接失败/429/5xx 现在走退避重试 | `argus-core/ai.rs` | 健壮性 | ab8fa01 |
| `parse_human_size` f64→u64 饱和转换，超大输入静默返回 u64::MAX、负数返回 0；`Overflow` 变体从未被构造 | `argus-core/model.rs` | 输入校验 | 34d2e25 |
| `request_delta_refresh`、`:R` 重连、`:Consolidate` 从 `TuiConfig::default()` 取 socket 路径（`:Consolidate` 在无 client 时甚至取空串）→ 自定义 `[daemon].uds_path` 时连错地方 | `argus-tui/delta.rs` `handler/browsing.rs` `handler/command.rs` | server 模式连接 bug | c190c0e |
| TUI 自带一份受保护路径黑名单但比 core 弱：macOS `/etc` canonicalize 后是 `/private/etc`，不在 TUI 清单里 → **TUI 删除保护对 /etc、/var/db 失效**。改为委托 `argus_core::is_protected` 单一权威清单 | `argus-tui/util.rs` | 删除安全 | b628f23 |
| `App::new` 加载 AI 缓存是 N+1 查询（先列路径再逐条取 blob）→ 新增 `load_ai_cache_entries` 单查询 | `argus-tui/app.rs` `argus-core/db.rs` | 启动性能 | 630e911 |
| `compute_path_size`/`compute_pending_total_size` 依赖 HashMap 遍历顺序取"第一个前缀命中" → `/a`、`/a/b` 同时缓存时结果不确定。合并为最长前缀优先的 `lookup_scan_size` | `argus-tui/app.rs` | 正确性（边缘场景） | 630e911 |
| AI 关闭时的启发式判定与真 AI 结果写同一缓存键 → 配好 AI 后这些路径**永远**不会再被真分析。verdict 增加 `source` 字段，启发式条目在 AI 可用时自动升级重分析 | `argus-tui/types.rs` `app.rs` | AI 体验 | 630e911 |
| 单文件删除成功走 `set_error`（红色错误样式），批量删除却走 success 样式 | `argus-tui/handler/prompt.rs` | 体验一致性 | 049e4b8 |
| `:Time` 时长解析 `n * mult` 无溢出检查，debug 构建可 panic | `argus-tui/time_utils.rs` | 输入校验 | 049e4b8 |
| `[keybindings]`、`[labels]`、`[theme].colors` 被解析进 TuiConfig 但**从未被任何代码读取** → 用户配置 `move_up = "w"` 毫无效果。解析器只保留已实现的节（serde 忽略未知节，旧配置文件兼容） | `argus-tui/config.rs` + README + `04-configuration.md` | 幽灵配置 | ecbc79b |
| CLI `truncate_ellipsis` 按字节切片 → 中文 brew 描述超 40 字节时**panic**（非字符边界） | `argus-cli/main.rs` | 崩溃 | 054f8fa |
| `free_space` 用 `std::process::Command` 调 `df -k /`，违反 AGENTS.md 硬约束 → 改 libc statfs/statvfs（新增 libc 依赖） | `argus-cli/main.rs` | 硬约束合规 | 054f8fa |
| consolidate/status/clear 三份相同的 IPC 帧收发代码 → 提取 `daemon_request()` | `argus-cli/main.rs` | DRY | 054f8fa |

另：argusd `main.rs` 去掉 `open_db` 后多余的 `init_db`（随 d08c4a1）；clippy `--all-targets` 清零（ee94ad2）。

## 已修复（第三轮）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| SQLite `LIKE` 未转义通配符：查询 `my_projects` 会把 `my-projects`/`myXprojects` 的 delta 事件算进来；含 `_` 的目录被 consolidate 后，agg 反连接还会**抑制兄弟目录的原始事件**；`consolidate_events` 的 DELETE 会误删兄弟目录的行 | `argus-core/db.rs` 三处查询 + consolidate | delta 记账错误 + 误删数据 | 560c53f |
| TUI 清理面板 `d` 键显示 `[DRY-RUN]`，但确认后 `exec_clean(items, dry_run)` 把标志传给被忽略的 `_force` 参数 → **dry-run 下真实删除** | `argus-tui/handler/cleanup.rs` `argus-core/cleaner.rs` | 数据安全 | 523d299 |
| CLI `argus clean` 在 "Proceed with cleanup?" 确认**之前**执行 `brew cleanup`、`docker builder prune -f` 等不可逆命令 | `argus-cli/main.rs` | 破坏性操作未经确认 | 523d299 |
| daemon 重启/缓存淘汰后首次 modify 无基线，旧代码以 0 为基线把整个文件大小记为幽灵正增量（touch 一个 5GB 文件记 +5GB） | `argusd/watcher.rs` | delta 记账错误 | 6318726 |
| 无单实例保护：起第二个 argusd 会移走运行中 daemon 的 socket 文件，且两个 watcher 对同一事件各插一条 → delta 双倍记账 | `argusd/daemonize.rs` `main.rs` | delta 记账错误 | 2d179ee |
| `plan_clean` 把 target 总尺寸赋给该 target 的每个路径：多项 target 时每项 label 虚高，exec_clean 的 freed_bytes 按项数翻倍 | `argus-core/cleaner.rs` | 清理统计错误 | 508b67b |
| `bundle_id_for_app` 硬编码 `start + 30` 偏移多跳 1 字节：压缩格式 plist（`</key><string>` 无空白）解析失败 → bundle id 落到 `unknown.<name>` → leftover 扫描为空 | `argus-core/uninstaller.rs` | 卸载残留清理失效 | 569a5ba |
| `classify_risk` 用裸 `starts_with(home)`：`/Users/lexx/...` 被当成 `/Users/lex` 家目录内 → 误判 Safe | `argus-core/safety.rs` | 删除风险分级 | 91aada8 |
| AI 缓存按 64 位 `path_hash` 单键读写：碰撞时静默返回/覆盖别的路径的分析结果。读写删均加 `path` 列校验 | `argus-core/db.rs` | 健壮性 | 91aada8 |
| `apply_deletion_to_state` 对 view 树和每个命中的 scan_cache 条目各计一次 freed：删除的路径同时存在于根扫描和子目录缓存时 freed 双倍 | `argus-tui/tree_ops.rs` | 统计错误 | fae8995 |
| `AppMessage::Info` 走 `set_error`：后台任务的成功消息（"consolidated N events"）显示为红色错误 | `argus-tui/app.rs` | 体验一致性 | 同上 |
| `TIME_PRESET_COUNT = 7` 但预设只有 0..5：`t` 键循环中多出一个重复的 1h 档 | `argus-tui/types.rs` | 交互小 bug | 同上 |
| `ShellCmdTarget.timeout_secs` 声明但从未生效：卡死的 docker/brew 命令永久阻塞清理流程。改为 spawn + `try_wait` 轮询 + 超时 kill | `argus-core/shell_cmd.rs` | 可靠性 | 09593c0 |
| CLI 连 daemon 硬编码默认 socket，`[daemon].uds_path` 自定义时 status/consolidate/clear 全部连错地方（TUI 同类问题已在 c190c0e 修复）。新增全局 `--uds-path` | `argus-cli/main.rs` | server 模式可用性 | 同上 |
| `dir_size` 四份私有拷贝（purge/categories/brew/uninstaller）合并为 `cleaner::dir_size`，统一跳过符号链接 | `argus-core/cleaner/*` | DRY | 508b67b |
| 文案 `mo clean --dry-run`（其它项目的残留） | `argus-cli/main.rs` | 文案 | 523d299 |
| brew 卸载前从不展示依赖者警告：`brew_dependents_of` 是死 API，而卸载带 `--force`（brew 自己也不会拦）。TUI 确认框后台查询并列出依赖者，CLI 在确认提示前打印 | `argus-tui/handler/brew.rs` `components/brew.rs` `argus-cli/main.rs` | 删除安全（存疑 #13 已落地） | d88d131 |

另：`mock_ai_verdict` 的 Phase 1/Phase 2 过期注释更新；AGENTS.md 中从未存在的 `diff` 模块引用改为真实模块；05-ux-interaction.md 命令清单同步到实际 CLI（含 `--uds-path`）。

## 存疑 / 记录在案（未改动）

### 1. 聚合行覆盖下，新事件的可见性滞后
consolidate 之后某子树已有 agg 覆盖行，新到达的子路径事件在查询层被 agg 反连接抑制，要等下一次 consolidation（默认间隔 60 分钟）才折入 agg。期间 TUI delta 面板看不到这部分变化。嵌套 agg 场景的**永久性**丢失已由 5fd47ec 修复，本条只剩窗口期滞后。可选修法：新事件插入时若祖先存在 agg 行，直接以小粒度行写入并在查询层做"agg 截止到 agg.ts，其后事件单独计"的语义；或缩短 consolidate 间隔。
**已解决（第五轮，c9c33ea）**：反连接增加 `事件 ts <= agg.ts` 条件——consolidation 之后到达的事件（ts 超出 agg 的 max 子事件 ts）立即可见，无需等下一次聚合。仅剩"迟到写入且事件 ts 早于 agg.ts"的极端场景（去抖落库晚于聚合）仍会被抑制，与原行为一致。

### 2. SQLite `LIKE` 对 ASCII 不区分大小写
`query_delta_total/detail` 的路径前缀匹配 `path LIKE '/users/%'` 会命中 `/Users/...`。macOS 默认文件系统大小写不敏感，跨大小写路径冲突的场景罕见，但理论上会造成串数据。若要严格，可改 `GLOB` 或 `PRAGMA case_sensitive_like`。
**已解决（第三轮，560c53f）**：前缀匹配改为 `substr` 精确等值，天然大小写敏感且无通配符语义。

### 3. 非 ASCII 搜索高亮可能偏移 1 个字符位
`argus-tui/search.rs` `fuzzy_match_indices` 非 ASCII 分支用 `to_lowercase()` 后的字符数换算高亮区间；个别字符（如 `İ`）小写化后字符数会变，高亮宽度随之偏移。文件名场景极少见，暂不处理。

### 4. 多选删除静默丢弃与根同名的选中项
`handle_multi_delete_action` 按 `file_name() != root_name` 过滤，与根目录同名的普通文件/目录被无声剔除。**已部分改善**（8f45519）：状态栏现在会提示跳过了多少项；但"与根同名就剔除"这条规则本身仍偏保守，单选删除同路径会显式报错，规则上仍不一致。

### 5. `build_current_tree` 失败即清空视图
view_root 无 scan_cache 且 `list_dir` 失败（权限/被删）时置 `tree_root = None` 并报错，当前浏览内容全部清空。行为可以接受，但"保留旧树 + 报错"体验更好。

### 6. argusd PID 文件复用风险
`stop()` 按 PID 文件 kill，若守护进程异常退出后 PID 被其他进程复用，会误杀。个人工具风险可接受；正规做法是 pidfd / kill 前校验进程名。

### 7. `find_artifacts` 只扫每 root 一级
`~/Projects/2024/app/node_modules` 这类两层嵌套项目扫不到（一层：root 的子目录下的 artifact 目录）。默认 roots（Projects/GitHub/dev…）下再嵌一层项目目录的场景会漏。可考虑限深 2–3 层的目录遍历。

### 8. 架构坏味道：`argus-tui/app.rs` 约 1900 行
超出 AGENTS.md 500 行约定数倍。合理的拆分线已经存在：flat-mode 导航（enter/go_to_parent/go_up_fs/nav_*）、消息处理（handle_message）、AI review 状态机、清理扫描。建议在下次功能性改动顺手拆，避免专门一次大重构。

### 9. UDS socket 位于全局 /tmp
`/tmp/argusd.sock` 对本机所有用户可连接：任何本地进程都能查询 delta（路径泄露信息）、ClearDb、触发 consolidation。个人工具可接受；加固方向是按用户隔离的 `$TMPDIR`（`getconf DARWIN_USER_TEMP_DIR`）或 socket 权限位。跨端改动需同步 TUI/CLI/daemon 三处默认值。
**权限位已加固（第四轮，4d349c7）**：daemon 绑定后立即 `chmod 0600`，非 daemon 属主用户已无法连接；路径迁移到 `$TMPDIR` 仍未做（需三端同步默认值）。

### 10. brew 依赖错误靠字符串反解析
`AppMessage::Error` 处理里用 `split("required by ")` 从错误文案中提取依赖者列表回填 brew 包元数据。文案一改即静默失效；重构 brew 错误通道（结构化错误）时顺手处理。

### 11. CLI uninstall/brew 选择串匹配
`inquire::Select` 的选中项靠重新格式化字符串再全串比对反查下标。当前格式化函数一致所以正确，但格式串是单一事实点的两份拷贝；将来改列格式时需两处同步（或直接用 index 选择）。

### 12. `uninstaller.rs` 通过 `mdls` 查询应用元数据
属于对外部二进制的调用，但 macOS Spotlight 元数据没有纯 Rust 原生接口（需 CoreServices FFI/objc 绑定），作为查询型例外保留；与 AGENTS.md 约束针对的"用 shell 做文件操作"性质不同。若将来引入 `objc2-core-services` 绑定可移除。

## 性能观察（评估过，暂不动）

- `has_ai_analysis_batch` 逐 path 一条 SQL；批量为个位数～几十条，开销可忽略。若将来批量上千，改临时表 JOIN。
- `prune_file_node` 删除后对每个祖先做整子树尺寸重算，O(深度 × 子树)。交互式删除频率低，正确性优先，暂保留。
- purge 发现阶段对每个命中 artifact 做完整 `dir_size` 递归（可能很大），但在后台线程执行且结果要展示大小，属必要成本。
- `Snapshot::child_idx` 为 CSR 区间线性扫描；单目录子项数千以内无感。如遇超 fan-out 目录再考虑加索引。
- TUI 渲染层 `right_width` 逐帧重算等微开销，远低于渲染阈值，不值得优化。
- `SeenInodes::positions` 每次插入做 9 次完整 SipHash（device+inode+轮次）。可换 Kirsch–Mitzenmacher 双重哈希（1 次哈希派生 9 个位置），约省扫描热路径上 ~5% CPU；布隆误判率特性会略变，改动前先跑误判率测试。
- `load_current_children` 的 graft 路径（进入树中无子节点的目录时 `list_dir` 后 `Arc::make_mut`）会整体克隆快照：`current_children` 持有同一 Arc，refcount>1 触发克隆。大树上每次进入此类目录多付一次 O(树) 拷贝。出现频率低（仅"树里为空但磁盘上非空"的目录），若实际可感可改为独立 overlay 列表。
- `query_delta_total/detail` 前缀匹配由 `LIKE 'prefix%'` 改为 `substr` 等值后，理论上放弃了 LIKE 前缀索引优化；但原查询从未设 `PRAGMA case_sensitive_like=ON`，大小写不敏感的 LIKE 本就不满足该优化的前提，且 OR 条件也阻碍索引路径。个人库数据量（万级事件）下无回归。

## 存疑 / 记录在案（第三轮新增，未改动）

### 13. brew 依赖信息链路断裂：`brew_dependents_of` 从未被调用
CLI/TUI 的 brew 面板都展示 `dependents` 字段，但列表构建（`list_brew_packages`）恒填 0，core 里的 `brew_dependents_of()`（`brew uses --installed`）没有任何调用方——用户在卸载前看不到"有 N 个包依赖它"的警告，而 `uninstall_brew_package` 又带 `--force`（绕过 brew 的依赖检查）。合理接法：卸载前对该包调一次 `brew_dependents_of`，非空则展示并要求额外确认。属于 brew 错误通道重构（存疑 #10）同域，留待一起做。
**已落地（第三轮，d88d131）**：TUI 确认框打开时后台查询并列出依赖者（最多展示 5 个），CLI 在确认提示前打印。剩余的"brew 错误靠字符串反解析"（存疑 #10）仍留待结构化错误通道重构。

### 14. 死公共 API：`exec_all_shell_cmds`、`has_ai_analysis(_batch)`
三者只在 `lib.rs` re-export，全仓无调用方（`has_ai_analysis_batch` 已被 `load_ai_cache_entries` 取代）。保留待真实需求出现或下次清理时删除；`#[allow(dead_code)]` 对 pub 项不生效，故暂无噪音。

### 15. watcher size_cache 淘汰后，删除事件无法记账
`MAX_CACHE_ENTRIES`（10 万）触发减半淘汰时，被逐出的条目后续删除时 `state.remove()` 返回 None → 负增量丢失，delta 只会偏高不会偏低（保守方向）。这是有界缓存的固有代价，当前上限对个人机器绰绰有余；若将来要彻底解决，需把基线尺寸落库。

### 16. 启动时不存在的 watch 目录永远不被监控
`start_watcher` 对 `!dir.exists()` 的目录只打 warn 跳过，之后即使用户创建了该目录也不会补挂 watcher，直到重启 daemon。可选：后台线程周期性重试挂载，或监听父目录的 create 事件。

### 17. 审计日志只追加、无轮转；`read_audit_log` 返回最旧的 limit 条
`~/.config/argus/audit.log` 无限增长（每行一条 JSON，个人使用增长缓慢）；且 `read_audit_log(limit)` 从文件头读，返回的是**最旧**记录——审计场景通常想要"最近 N 条"。轻量修法：读取时用固定环形缓冲/从尾部读。
**limit 语义已修复（第四轮，294f412）**：`read_audit_from` 读完整个文件后保留最近 N 条（按时间正序返回）。日志轮转仍未做，见第四轮存疑 #21。

### 18. `find_orphaned_data` 的已知性判断双向 contains，容易漏报孤儿
`fc.contains(kc) || kc.contains(fc)`：名为 "Go" 的应用会把 "golang"、"google-cloud-sdk" 全部判为已知，孤儿数据漏报。方向保守（宁可漏删不可误删），可接受；若要更准可改成词边界匹配或仅精确等值 + bundle id 前缀。

### 19. IPC 客户端读响应无长度上限
daemon 端对请求设了 `MAX_PAYLOAD_LEN`，但 TUI/CLI 的 `send_request` 按 daemon 报头分配响应缓冲。对端是本机同用户的可信进程，风险低；若要对称加固，读响应时套同样的上限。

### 20. 单实例守卫依赖 PID 文件，崩溃后可能误报"已在运行"
`DaemonGuard::acquire` 以 `kill(pid, 0)` 判断存活；崩溃残留的 PID 文件若被回收复用会拒绝启动（错误信息已提示 `argusd stop` 恢复）。与 stop() 的既有 PID 复用问题（存疑 #6）同源，一并留待 pidfd/进程名校验方案。

## 已修复（第四轮）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| `run_with_timeout` 轮询期间不读管道：子进程输出超过 64KiB 管道缓冲后阻塞在 write，永远退不出 → 多输出的 `brew cleanup` 每次都按超时被杀（上轮修的超时机制自身引入的缺陷）。管道改为后台线程并行排空 | `argus-core/shell_cmd.rs` | 清理流程误报超时 | a34ada1 |
| zsh 扩展历史 `: <ts>:0;<cmd>` 解析取到 `"<ts>:0"`，`parse::<i64>` 必然失败 → **带时间戳的 zsh 历史分支从未生效**，所有包退化为 mtime 粗粒度。先按 `;` 切再按 `:` 取首段 | `argus-core/brew.rs` | brew 排序依据错误（新测试暴露） | abf2228 |
| `list_brew_packages` 对每个包重读两份完整 shell 历史（长历史 × 百级包 = 数百 MB 重复读），`last_access_from_opt` 每包重新 spawn `brew --prefix`；且其"100 文件上限"实际在第一个有条目的文件处提前返回，注释与行为不符。历史/prefix 每次扫描只加载一次，atime 取前 100 项最大值；非 UTF-8 历史文件跳过而非中断整个查找（原 `?` 会连带丢弃后续文件） | `argus-core/brew.rs` | brew 扫描性能 + 正确性 | abf2228 |
| `read_audit_from(limit)` 从文件头截断，返回**最旧** N 条（存疑 #17 后半）。改为保留最近 N 条并补测试 | `argus-core/audit.rs` | 审计查询语义 | 294f412 |
| UDS socket 绑定后无权限收紧（存疑 #9 前半）：/tmp 下默认全员可连，任意本地用户可 ClearDb。绑定后 chmod 0600 | `argusd/ipc_server.rs` | IPC 暴露面 | 4d349c7 |
| `find_orphaned_data` 内部调 `find_installed_apps`（逐 app spawn `mdls` + 全量 bundle 尺寸遍历），但它只需要名字和 bundle id；CLI `cmd_clean` 又额外跑第二次全量扫描仅为打印 app 数。拆出免详情的 `collect_app_bundles`，`OrphanedData` 新增 `installed_app_count` | `argus-core/uninstaller.rs` `argus-cli/main.rs` | clean 命令耗时（秒级节省） | 7c759e3 |
| Clean 面板 `j`/`k` 移动光标时静默把 `dry_run` 重置为 false：用户显式开启的预览模式一移动就变回真实删除 | `argus-tui/handler/cleanup.rs` | 删除预览可预期性 | 93c1097 |
| CLI `argus clean` 的 "First time? Run --dry-run first" 提示只在 dry-run 模式下显示（逻辑写反），真实删除前反而看不到 | `argus-cli/main.rs` | 提示时机 | 7c759e3 |
| `.git` 内部文件（pack 可达数百 MB）全额记账：`is_ignored` 只看路径最后一段，与注释声称的".git internals 是噪音"不符。隐藏目录检查改为遍历 watch root 之下的所有路径段；watch dir 配了显式 include glob 时跳过该检查（用户显式选择这些路径） | `argusd/watcher.rs` | delta 噪音 | 19bd263 |
| `.` 切换隐藏文件可见性走 `set_error`（红色错误样式）；同类中性消息样式统一 | `argus-tui/handler/browsing.rs` | 体验一致性 | 677b6a3 |
| `remove_artifacts`/`uninstall_app` 与 `exec_clean` 三份"保护检查+trash+报告+审计"循环合并为 `exec_items(op)`；uninstaller 的 `app_size` 私有遍历与 TUI 明细面板的 `dir_total_size` 均改为复用共享 `dir_size`（已导出为公共 API） | `argus-core/cleaner/*` `argus-tui/handler/cleanup.rs` | DRY | 0e1829a |

## 存疑 / 记录在案（第四轮新增，未改动）

### 21. 审计日志无轮转
`read_audit_log` 语义已修（见 #17），但文件本身仍无限追加。个人使用增长缓慢（每次删除一行 JSON），暂不做轮转；若做，按大小或按月切段 + 读取时合并。

### 22. 隐藏目录过滤遮蔽 `.npm`/`.cargo` 类缓存的真实增长
第四轮把隐藏目录**内部**的变更也过滤了（与注释声称的意图一致，.git 噪音确实严重）。副作用：`~/.npm/_cacache`、`~/.cargo/registry` 等隐藏目录下真实的大体积增长也不再进 delta。用户可通过 watch dir 的显式 `include` glob 重新纳入（此路径不受隐藏过滤影响）。若默认行为要反转，改动点在 `has_hidden_ancestor` 的调用条件。

### 23. `classify_risk` 对 home 下未知目录判 Safe
`~/Documents`、`~/src` 等不在 Library/.Trash 规则内的路径返回 `Safe`（无需键入式确认）。当前所有删除入口都有 `.max(target.risk)` / `.max(Low)` 兜底 + 保护路径硬闸 + 废纸篓默认，未构成实际风险；但语义上"未知"更接近 Medium。改动会影响现有 target 的确认流，需连同 UI 文案一起评估。

### 24. watcher 对 `RenameMode::Both/Any` 不产生事件
rename 只处理 `From`（记删）与 `To`（记增）。macOS FSEvents 与 inotify 对 rename 的事件形态不同（有的拆成 remove+create，有的发 `Both`），`_ => None` 分支可能让某些平台的 rename 漏记。实测未观察到缺账（多数后端拆发事件），留观。

### 25. `spotlight_last_used` 的 app 名匹配过宽
`pkg_name.contains(&stem.to_lowercase())` 单向包含：短包名可能匹配到无关 app（如包 `vim` 匹配 `Vi Media.app`?）。仅影响 last_used 展示的准确性，不影响删除决策；mdls 调用有 app 目录遍历上限。可加双向词边界收紧。

## 性能观察（第四轮新增）

- `find_installed_apps` 保留逐 app `mdls` + bundle 尺寸遍历——这是"按最近使用排序/展示大小"功能的必要成本（Uninstall 面板真正需要这些数据），orphan 扫描侧已豁免。
- `consolidate_events` 每周期全表扫描原始事件行（`WHERE is_agg = 0`）。事件表在保留期（默认 30 天）+ consolidation 下有界，个人量级无感；若将来默认保留期变长，考虑按 `timestamp` 分段扫描。

## 已修复（第五轮）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| agg 反连接按「任意深度后代」抑制 raw 事件，但 consolidation 是 parent-local 的（agg 只汇总直接子级）：已聚合目录下的**孙辈 raw 事件**被祖先 agg 抑制——若其直接父目录从不聚合（低于阈值），这部分 delta **永久丢失** | `argus-core/db.rs` `query_delta_total/detail` | delta 记账错误（永久少算） | c9c33ea |
| 同一反连接不比较事件 ts 与 agg ts：consolidate 之后到达的子路径事件要等下一轮聚合才可见（旧存疑 #1 的窗口期滞后，默认最长 60 分钟） | 同上 | delta 可见性滞后 | 同上 |
| `classify_risk` 用裸 `starts_with("/tmp")`：`/tmpbackup` 被当成临时目录（Medium 而非 Low）；与第三轮修掉的 home 边界同型。`/var/tmp`、`/Library` 同步改为带分隔符的前缀判断 | `argus-core/safety.rs` | 风险分级 | 67f6a46 |
| `parse_human_size("GB")` 返回 `Ok(0)`：split 助手对空数字部分回退 "0"，纯单位字符串是唯一能走到该回退并成功解析的输入。删除回退后纯单位按错别字拒绝；`.5KB` 等合法前导点输入不受影响 | `argus-core/model.rs` | 输入校验 | 同上 |
| `list_brew_packages` 对每个包全量重扫两份 shell 历史：O(包数 × 历史行数)，重 zsh 历史（数十万行）× 百级包 = 数亿次行检查。改为加载时单遍建 token 索引（精确 + 去尾数字两张表，镜像 `command_matches_pkg` 三段匹配语义），每包查询降为常数次 HashMap 探测；新增暴力扫描等价性测试钉住匹配语义 | `argus-core/brew.rs` | brew 扫描性能 | fdf14cc |
| `spotlight_last_used` 中 mdls spawn 失败经 `?` 直接放弃整个 app 目录查找（其余 app 全部跳过）。改为 continue 到下一个 app | `argus-core/brew.rs` | 健壮性 | 同上 |
| `:scan docs`、`:consolidate now` 等带参数形式绕过裸命令拦截后落到 `App::cmd_scan`/`cmd_consolidate`，返回"scan started"/"consolidation requested"但**什么都不做**。cmd_scan 真正调用 start_scan；cmd_consolidate 委托新提取的 `App::request_consolidation`（顺带去重裸 `:Consolidate` 分支的内联 spawn） | `argus-tui/command.rs` `handler/command.rs` | 静默 no-op | d8687f6 |

## 存疑 / 记录在案（第五轮新增，未改动）

### 26. watcher 的硬链接去重是"装饰性"的
`WatcherState::file_size` 在 `nlink > 1` 且命中 hardlink_cache 时返回已缓存尺寸——但同一 inode 的 `stat.len()` 恒等，dedup 命中与直读 metadata 返回值完全一样，create 事件照样全额记账。净效果：硬链接组每次 create 都 +size、每次 delete 都 -size，**全生命周期净额正确**，但存在窗口期内瞬时虚高（如 pnpm 全局 store + 项目 node_modules 同时计入）。真正按 inode 记账需要存 nlink 并做"末链接删除才计负增量"的引用计数模型，涉及记账语义重设计，且当前净额方向正确，暂不动。
**create 侧已修复（第六轮，229711f）**：hardlink_cache 改为记录所有观察过的文件（不再只记 nlink>1，首链接因此可被命中），Create 事件若命中已存在且 inode 仍指向同一文件的映射则记账 0（基线照常播种）。`cp -l` 等新建链接不再虚增 delta，生命周期净额与瞬时值都正确；rename 以 remove+create 形式到达时靠「映射路径存在且 inode 相同」守卫避免误吞 create 的一半。**delete 侧未动**：删除硬链接组中的一个链接仍计全额 -size（组内还有其他链接时磁盘占用并未减少），引用计数模型仍然缺位。另：hardlink_cache 与 size_cache 同为每文件一项，watcher 内存占用约翻倍（同样受 10 万条上限约束）。

### 27. 命令栏中 j/k 被导航抢占
`:Command` 输入态下 `j`/`k` 优先用于上下选择补全项（matches 几乎对任何输入都非空），因此命令参数里实际上打不出 j/k 字符。现有命令集（scan/sort/time/delta/…）参数不含 j/k，未构成实际问题；若将来命令参数需要这两个字母，需改为仅上/下箭头导航。
**已解决（第六轮，随 #29 键位重划）**：补全导航收敛到 Tab/BackTab，`j`/`k` 恢复为字面字符输入（有测试钉住）。

### 28. AI 分块发送时单个超限 context 无法再切分
`find_chunk_boundary` 的二分下界是 1：单个目录指纹本身就超过 `max_tokens_per_request` 时，chunk=1 依然发送并按 API 错误重试后失败。目录指纹只有两个短字段，触发条件不现实（路径长达数十 KB），留观。

## 已修复（第六轮）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| 命令栏 Enter 总是用高亮补全项**替换**已输入文本（matches 非空即触发）：`sn`（按名称排序）恰是 `Scan` 的子序列 → 回车全量重扫；`sd` → 执行 `Consolidate`；空输入回车直接打开 Brew。改为仅当输入是高亮项的（大小写不敏感）前缀时才展开补全，否则执行输入原文 | `argus-tui/handler/command.rs` | 交互 bug（可触发意外重扫） | c11dd25 |
| retention 周期计算 `retention_days * 86_400_000`、`interval_minutes * 60` 用裸乘法：config 填入超大值时 release 下回绕使 `prune_before` 落到近 past——**一次保留任务即可清空整个 delta 库**（debug 构建 panic）。改 saturating，interval 钳到最小 60s；提取纯函数并补测试 | `argusd/retention.rs` | 数据丢失（极端 config） | 3d8fe93 |
| watcher Create 事件对重复硬链接全额记账（与 scanner 的 `SeenInodes` 去重语义不一致），且旧 hardlink_cache 只记 nlink>1 的观察、首链接永不入表、去重分支几乎不命中（见 #26，create 侧已修）。符号链接同样修正：`stat` 跟随链接，创建/删除 symlink 曾按 target 尺寸记账幽灵 delta（Homebrew、`node_modules/.bin` 会创建大量链接），改为与 scanner 一致按 0 处理 | `argusd/watcher.rs` | delta 记账虚增 | 229711f |
| `datetime_to_millis` 对不存在的日期（2-30、13-01、24:00、12:99）静默返回 0=epoch：`:time` 过滤器实际变成"从 1970 起"（显示全部事件），状态栏却显示着错误日期。改为返回 `Result`，`:time` 及 `from to HH:MM` 右侧解析把错误报给用户（右侧 `unwrap_or(0)` 一并消除） | `argus-tui/time_utils.rs` `command.rs` | 时间过滤静默失效 | cb0cfc7 |
| TUI/CLI 客户端读 daemon 响应帧不校验长度上限（daemon 侧请求上限在 0a63ee2 已加，客户端漏了）：损坏/恶意流可用 4 字节长度让客户端分配最多 4 GiB。两端补齐与 daemon 相同的 64 MiB 上限 | `argus-tui/ipc_client.rs` `argus-cli/main.rs` | 健壮性 | b4ba818 |
| `brew_prefix()` 每次调用同步 spawn `brew --prefix`（Ruby 脚本约 100ms），且在 TUI 消息处理路径（BrewScanComplete / enter_brew_ai_review）调用——每次 brew 扫描完成卡一次 UI。前缀进程内不变，改 `OnceLock` 缓存 | `argus-core/brew.rs` | TUI 卡顿 | 995f15e |
| CLI `count_files` 手写递归与 `Snapshot.total_files` 重复（builder 已算好）。删递归用字段 | `argus-cli/main.rs` | DRY | 9277b7e |

## 存疑 / 记录在案（第六轮新增，未改动）

### 29. 命令栏历史导航几乎不可达
Up/Down（及 j/k）在 matches 非空时优先导航补全列表，而 matches 仅在输入匹配不到任何命令时为空——即历史上下翻只在"输入的是无效命令"时可达。vim 风格的"空输入 + Up = 历史"会更顺手；当前选择器行为自洽，暂不改（行为变更收益低）。
**已解决（第六轮）**：键位重划为 Tab/Shift+Tab 上下循环补全列表、Up/Down 专职翻命令历史（空历史时无操作）；j/k 不再参与导航（见 #27）。帮助浮层补 Command Bar 键位节，README 同步。

### 30. watcher 硬链接 delete 侧引用计数缺位
见 #26 第六轮更新：create 侧已去重，删除硬链接组中的一个链接仍计全额 -size（组内其他链接的数据仍在盘上）。完整修复需要 nlink 引用计数模型。

## 已修复（第七轮，2026-09-29）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| 三个清理 target 的路径与其它 target 完全重复：`system-diagnostic-logs`（= system-crash-reports 的用户 DiagnosticReports 路径）、`user-app-logs`（= system-logs 的 `~/Library/Logs`）、`cloud-icloud`（= system-icloud-session 的 CloudKit 缓存）→ 同时选中时 plan 总量双计、同一目录 trash 两次（第二次必 ENOENT）。删除三个冗余 target，并在 `plan_clean` 增加组件级去重：路径相等或嵌套于更早条目（组件级 `Path::starts_with`，`/logs` 与 `/logs-extra` 互不影响）时只保留外层一条，剩余的伞形/子 target 重叠（`~/Library/Logs` ⊃ DiagnosticReports/PowerManagement）也因此只计一次 | `argus-core/cleaner/categories.rs` `cleaner.rs` | 清理总量虚报 + 必现删除报错 | d4e05b5 |
| `classify_risk` 用 `contains("/Library/")` 判定 Library 归属：`~/Library` 本体（Library 后无组件）不匹配 → 整个用户 Library 目录被判为 **Safe**（最低风险）。补 `ends_with("/Library")`；系统 `/Library` 维持 Medium | `argus-core/cleaner/safety.rs` | 风险分级（最敏感路径反而最低风险） | cf98b0c |
| DeltaData / AiAnalysisComplete / DeleteComplete 到达时重载 children 会清空搜索状态：daemon 推送增量或 AI 批次完成的一瞬，用户正在输入（Input）或 n/N 导航（Active）的查询静默消失。新增 `reload_children_preserving_search` 保留查询与模式并按新 children 重算匹配 | `argus-tui/app.rs` | 交互 bug（搜索被后台刷新吞掉） | 041b468 |
| 被删路径恰好是某个 scan_cache 根时缓存项残留：`remove_path_from_snapshot` 对根自身无组件可删、返回 false，陈旧快照留在缓存里；`enter_directory` 优先命中缓存 → 删除后再进入该目录显示已删内容。改为直接丢弃该缓存项，下次访问回退 `list_dir` | `argus-tui/tree_ops.rs` | 浏览陈旧数据 | da063bb |
| `search::fuzzy_match`（子序列匹配，命令补全用）与 `fuzzy_match_indices`（子串匹配，树搜索高亮用）名字几乎相同而语义不同。改名 `fuzzy_subsequence_match` | `argus-tui/search.rs` | 命名歧义 | 4025a13 |

## 存疑 / 记录在案（第七轮新增，未改动）

### 31. debounce 落库重试无上限
`flush`/`flush_expired` 失败后以 3s 过期时间重插 pending，若 DB 永久不可写（磁盘满、文件权限、库损坏），pending 无限累积且只有 error 日志，无放弃阈值、无用户可见告警。进程退出时最后的 `flush()` 同样可能静默丢事件。概率低（daemon 单写者 + WAL），真正修复需要重试上限 + 丢弃策略决策，出现实际症状再排期。

### 32. brew `spotlight_last_used` 匹配过松
`pkg_name.contains(&stem.to_lowercase())` 双向包含：cask `iterm2` 会命中 `iTerm.app`（读错 last-used），反之短名包可能命中大量无关 app。仅在 shell 历史、opt atime 两级都无命中时作为 cask 回退，影响面是显示值；收紧匹配会让部分 cask 从"可能有误的值"变成"无值（never）"，属产品取舍，未动。

### 33. `find_orphaned_data` 双向 contains 归类过松
`fc.contains(&kc) || kc.contains(&fc)`：极短目录名（如 `go`）会被任何 bundle id 含该串的已装应用"认领"，孤儿判定漏报；同样机制也可能误报。启发式的固有模糊性，修需要更精细的名称相似度策略。

### 34. `dir_size` 顶层符号链接跟随目标
函数对遍历中的条目跳过 symlink，但传入的 `path` 本身若是 symlink→dir，`path.is_dir()` 跟随链接并统计 target 全树——与函数注释"symlinks are skipped entirely"矛盾。当前所有调用方（keg、app bundle、清理 target）都传真实目录，未构成实际问题；修复应在入口处用 `symlink_metadata` 短路。
**已解决（第九轮，b4472bd）**：入口 `symlink_metadata` 短路，link→dir 与 link→file 均计 0，有测试钉住。

### 35. watcher RenameMode::To 缺 From 半账时虚增
rename 的 To 事件按 create 全额记账；若 From 半账丢失（监控启动窗口、事件洪峰丢弃、跨文件系统 rename 只报 create），净效果虚增 +size 且无对冲负账。事件流固有窗口，修复需配对缓冲，复杂度高于收益，留观。

### 36. `fuzzy_match_indices` 非 ASCII 分支的字符索引可能偏移
`to_lowercase()` 可能改变字符数（如 `İ` → `i̇` 两字符），按小写串计算出的字符区间套回原串时高亮错位。仅影响显示层高亮，CJK（1:1 映射）不受影响。

### 37. TUI `i`/`x` 按键在 UI 线程同步 open_db
`handle_info_popup` / `handle_delete_ai_analysis` 每次按键同步 `open_db`（建目录、WAL pragma、CREATE TABLE IF NOT EXISTS），毫秒级阻塞。可把 `Connection` 缓存在 App 生命周期内（UI 单线程，无 Sync 问题），属小优化，当前无可感知卡顿。

### 38. `freed_bytes` 语义与"释放"不符
`exec_items` 把进废纸篓的尺寸计入 freed_bytes，但废纸未清空前磁盘占用不变。UI 文案是 "freed"。改文案（"removed"/"移入废纸篓"）或区分两种统计是产品决策。

### 39. 两套 `RiskLevel` 枚举并存
`argus_core::cleaner::safety::RiskLevel`（带 Ord，安全分级）与 `argus_tui::types::RiskLevel`（带 serde，AI verdict 用）字段同名、转换靠手写 match。可下沉到 core 统一，但牵动 AI 缓存序列化格式（已落库的 verdict JSON），迁移成本大于当前收益。

### 40. TUI 配置解析失败静默回退默认值
`argus-tui/config.rs load_config` 对 TOML 解析错误与 daemon 侧不同——直接返回默认值且无任何输出。用户 typo（如 `[daemons]`）后所有自定义项悄悄失效。可在进入 TUI 前 `eprintln!` 一行警告。

## 已修复（第八轮，2026-09-30）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| `test_find_orphaned_data_returns_ok` 直接扫真实 `$HOME/Library`：对每个未知条目做完整 `dir_size` 递归，开发机上 Application Support 动辄几十 GB，测试二进制挂起数十分钟、`cargo test --workspace` 永远完不成。抽出 `find_orphaned_data_in(home, apps)` 可注入核心，测试改用临时目录 | `argus-core/cleaner/uninstaller.rs` | 测试套件挂起（违反"单测不碰真实文件系统"约定） | e90e16e |
| 孤儿扫描已知名单只收录**去点** bundle id：沙盒容器目录（`~/Library/Containers/com.vendor.app`）和大量 Application Support 目录都按带点 id 命名，等值必不匹配 → 凡应用名不是 id 子串的已装应用，其容器被误报为孤儿数据。已知名单同时收录原始带点 id | 同上 | 孤儿数据误报 | 同上 |
| `delta_retention_days = 0`（或漏配）使 `prune_before = now`：首个 retention tick 把整个 delta 库清空。钳到最小 1 天（"清空"是 `argus clear` 的职责） | `argusd/retention.rs` | 配置笔误 → 数据丢失 | 同上 |
| cleanup / uninstall 面板扫描期间丢弃所有按键，而这两处恰是全应用最慢的扫描（全目标 dir_size、逐 app mdls）；brew 面板早有 Esc/q 逃生通道。补齐 | `argus-tui/handler/cleanup.rs` | 用户被困在转圈界面 | 5cccea9 |
| TUI 配置解析失败静默回退默认值（第七轮存疑 #40）：typo 后所有自定义项悄悄失效。解析失败 `eprintln!` 一行警告（进 TUI 前打印，退出后仍可见） | `argus-tui/config.rs` | 可诊断性 | b8cbfbc |
| header 版本号硬编码 `v0.1.0`，与 `--version` 脱节。改 `CARGO_PKG_VERSION` | `argus-tui/render.rs` | 版本漂移 | 同上 |
| 列宽/截断/光标全部按**字节**计量：`⚡`/`⎋`/`⏎` 等 3 字节符号使 info 列多预留 2 列、右对齐块偏移，中文文件名被过早截断，命令栏输入中文时光标飞出词尾。列宽、截断、光标改用 `unicode-width` 显示宽度 | `flat_tree.rs` `render.rs` `status_bar.rs` `command_bar.rs` | CJK/符号场景排版 | 32ad4e8 及后续 |
| 删除进行中每 tick 无条件整屏重绘（10 次/秒），而进度数字只在收到消息时变化。删除冗余 dirty | `argus-tui/event.rs` | 微性能 | 同上 |
| **AI review 删除确认绕过保护路径闸**：浏览模式删除在弹窗前检查 `is_protected_path`，AI review 面板 d/D + y 直接 `remove_dir_all`，无此检查——/etc 等路径可经此删除。同时删除失败也照样剪视图、把全部标记尺寸计入 freed。现在逐路径跳过受保护项并报错，仅成功者更新状态与计数，补回归测试 | `argus-tui/handler/ai_review.rs` | 删除安全 + 统计正确性 | f54a97e |
| `load_delta_detail` 仍从 `TuiConfig::default()` 取 socket 路径——c190c0e 修的同类幽灵配置的漏网点，自定义 `[daemon].uds_path` 时弹窗连错 socket。独立模式下按 K 现在给提示而不是弹无解释的空窗口 | `argus-tui/components/delta_detail.rs` | server 模式连接 bug + 体验 | 5d51237 |
| 死公共 API 清理（第三轮存疑 #14 约定"下次清理删除"）：`scan_target_size`（被 plan_clean 取代）、`exec_all_shell_cmds`、`has_ai_analysis`/`_batch`（被 load_ai_cache_entries 取代）、`load_all_ai_analyzed_paths`、`clean_brew_cache`、`brew_info_json`/`parse_brew_info_json`/`BrewDetailedInfo` 死链、`Snapshot::from_builder`（零引用） | `argus-core` 多处 | -173 行死代码 | 58e9cbe |

## 存疑 / 记录在案（第八轮新增，未改动）

### 41. 永久删除大目录时全量 materialize 条目 + 递归收集
`delete_dir_progressive` 的 `collect_items` 递归收集目录下所有路径再排序删除：数百万条目的大树会占用 O(条目数) 内存且递归深度受栈限制（数千层才见险）。交互式永久删除频率低，迭代器化收益有限，留观。

### 42. `classify_risk` 的 `contains("/Caches")` 类子串匹配过宽
`~/Library/MyCachesStuff` 也会命中 Caches 分支（Low）。方向保守（比真实风险低一级的分支都是 Low/Safe 间的保守侧），且删除入口均有 `.max(target.risk)` 兜底；与 #23 同域，改动需连 UI 文案一起评估。

### 43. `datetime_to_millis` 未来时刻回退到去年
`:time from 12-25`（今年 12-25 尚未到）返回**去年** 12-25 的时间戳，标签仍显示 `12-25`——窗口起点比标签早一年。语义上"今年还没到的时刻"作为 from 无意义，回退去年是可辩护的选择，但标签未反映年份，记录备查。

### 44. watcher `RemoveKind::Folder` 未显式处理
remove 分支只匹配 `File | Any`；某些后端目录删除报 `Folder` 落入 `_ => None`。目录本身不参与记账户（dir 不入 size_cache），漏掉的只是"清缓存"机会，文件级删除事件各平台仍独立到达，净额不受影响。与 #24 同域，实测未见缺账。

### 45. `request_consolidation` / delta detail 每次新建 UDS 连接
低频路径（用户显式命令），不复用 `daemon_client` 的代价是一次额外 connect（本机 UDS 亚毫秒级）。若将来加入轮询类调用再考虑复用。

## 性能观察（第八轮）

- `find_orphaned_data` 对每个未知条目 `dir_size` 是孤儿体积展示的必要成本；真正的问题是此前单测直接跑真实 HOME（已修）。生产路径在 CLI/TUI 均为后台/一次性调用，保持现状。
- `plan_clean` 的嵌套去重 O(n²)、`load_current_children` 的 graft 克隆等既有观察维持不变（见前几轮）。
- brew `list_brew_packages` 每包 `keg_size` 全树遍历是尺寸展示的必要成本；历史索引（第五轮）已消除主要热点。

## 性能观察（第七轮）

- `has_ai_analysis_batch` 仍是每路径一次查询（预编译语句复用），批量 AI 面板路径多时是 N 次探测。SQLite 本地点查微秒级，未构成瓶颈；若将来路径上千，可换临时表 join。
- `plan_clean` 的嵌套去重是 O(n²)（n = 现存 target 路径数，几十量级），可忽略。
- `query_delta_total/detail` 的反连接子查询依赖 `idx_delta_agg_path_time`，EXPLAIN 过；量大时的替代方案（预计算覆盖表）在 `scan-memory-optimization-csr.md` 的思路之外，暂无必要。

## 已修复（第九轮，2026-10-01）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| `pid_alive`/`stop()` 在 `kill(pid, 0)` **成功**后仍读 errno：成功调用不清 errno，读到的是任意早前失败的陈旧值——恰好为 ESRCH 时活着的 daemon 被判死，单实例守卫被静默绕过。改为先看返回码，失败才解读 errno | `argusd/daemonize.rs` | 单实例守卫失效（双重记账风险） | 9461d8a |
| `dir_size` 顶层符号链接跟随目标：入口用 `is_file`/`is_dir`（stat 穿透链接），link→大目录会把整个目标子树计入，与函数自身"symlinks are skipped"契约矛盾（第七轮存疑 #34 落地）。入口改 `symlink_metadata` 短路 | `argus-core/cleaner/mod.rs` | 尺寸统计错误 | b4472bd |
| `:time A to B` 用 `to_lowercase()` 定位分隔符再按字节下标切原串：`İ`→`i̇` 等字符变长时下标错位，可 panic 在非字符边界。分隔符是 ASCII，改用保长的 `to_ascii_lowercase()` | `argus-tui/command.rs` | 输入崩溃（低概率） | 590f300 |
| **uninstall 确认面板的逐项残留勾选从未生效**：确认时调用 `uninstall_app(app, remove_leftovers)`，core 内部重跑 `find_leftovers` 并删除**全部**残留——用户取消勾选的项照样进废纸篓，且每棵子树被重复 `dir_size` 一遍。core 新增 `uninstall_app_with_leftovers(app, paths)`（`uninstall_app` 保留为 CLI 用的薄封装），TUI 按 `remove_leftovers` + `selected_leftovers` 解析出确切路径传入 | `argus-core/cleaner/uninstaller.rs` `argus-tui/handler/cleanup.rs` | 误删用户明确保留的数据 | 6f60208 |
| uninstall 应用列表过滤态 Enter 被吞（brew 面板 Enter 退出过滤），必须 Esc 再 Enter | `argus-tui/handler/cleanup.rs` | 交互不一致 | 同上 |
| **视图根切换后多选残留**：选择键是相对视图根的 `Vec<String>`；finder 换根、进入缓存扫描子目录、`u` 上跳、子目录扫描完成切换根时旧选择仍在，多选删除会按**新根**解析旧键 → 删错路径。四处根变更点统一退出多选 | `argus-tui/app.rs` `handler/finder.rs` | 删除落点错误 | a6b4368 |
| finder 确认的新根不进导航历史：`b` 跳过 finder 位置、`f` 回不去。补 `push_nav_history` | `argus-tui/handler/finder.rs` | 导航历史缺口 | 同上 |

## 存疑 / 记录在案（第九轮新增，未改动）

### 46. `AiConfig::max_tokens_per_request` 一语两义
该值既用作**提示词**分块预算（估算 prompt token 超限就切块），又原样作为 API 请求的 `max_tokens`（**响应**上限）。批量分析的响应 JSON 随路径数线性增长，几十条路径的批次可能被响应截断触发整批重试。语义上应是两个独立配置；当前默认批次小，未观察到症状，留观。

### 47. 删除入口按「名字 == 根名」挡根目录，误伤同名子目录
`handle_delete_action` 与批量删除都用 `file_name == view_root 的根名` 阻止删根；浏览 `/tmp/test` 时其下恰好有 `/tmp/test/test/` 子目录会被误判为根而拒绝删除。改为路径等值比较需要把 entry 相对键还原成绝对路径再比（现有 `selected_node_full_path` 可复用），改动小但牵动两处提示文案，等实际撞上再修。

### 48. AI 审阅 `lookup_scan_size` 对每个路径线性扫整个 scan_cache
`compute_pending_total_size` × `lookup_scan_size` 是 O(路径数 × 缓存根数 × 深度)。AI 审阅通常几十条路径、缓存根个位数，微秒级；路径上千才值得按公共前缀建索引。

### 49. `alloc_name_into` 对 >64 KiB 的名字静默截断
`name_len` 钳到 `u16::MAX`，blob 仍写入完整字节，读回得到前 64 KiB。真实文件系统的单组件名上限是 255 字节，触发不了；记录以免将来 `INLINE_NAME_MAX`/编码格式变动时踩回去。

### 50. `DaemonGuard::acquire` 的检查-写入竞态（TOCTOU）
`running_daemon_pid()` 探测与 PID 文件写入之间没有原子性：两个 `argusd` 同时启动都能通过探测（PID 文件原子创建可缓解写入侧，但探测侧无锁）。当前用「先探测后写」+ 既有 PID 即拒绝把窗口压到毫秒级；彻底修复需要 flock 锁文件，收益低。

### 51. CLI `clean` 的孤儿应用数据只展示不清理
`argus clean` 的 "Uninstalled App Data" 节扫描并展示孤儿路径与体积，但后续 `exec_clean` 只删计划内 target——孤儿数据在 CLI 侧没有任何清理路径（TUI 卸载面板走的是逐应用 leftovers）。要么把孤儿纳入可选清理集，要么在文案里写明"仅提示"，现状两头不靠。

## 第九轮性能观察

- `dir_size` 入口短路消除了「link→大目录被整树遍历」的最坏路径（b4472bd），遍历主体不变。
- 第八轮的 `plan_clean` O(n²) 去重、`load_current_children` graft 克隆等既有观察维持不变。
- TUI `i`/`x` 每次按键同步 `open_db`（第八轮 #37）维持原判：毫秒级，缓存 Connection 的收益要等出现可感知卡顿。

## 已修复（第十轮，2026-10-02）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| **导航历史跨视图根切换后多选残留**：第九轮统一了 4 处根变更点退出多选，但 `b`/`f` 历史步进落到不同根的条目时漏了——选择键按新根解析旧值，多选删除落点错误。`nav_back`/`nav_forward` 收敛到 `apply_nav_position`，根变化即退出多选；同根步进保留选择 | `argus-tui/app.rs` | 删除落点错误（第九轮同类第 5 处） | fe100b9 |
| **根目录拦截按名字相等误伤同名子目录**（第九轮存疑 #47 落地）：浏览 `/tmp/test` 时其子目录 `/tmp/test/test` 因名字与根相同被拒绝删除。单删守卫与批删过滤都改为完整路径与 `view_root_path` 等值；选择集只含子项，守卫保持兜底语义不再过宽 | `argus-tui/handler/browsing.rs` | 合法子目录不可删 | 63f8fcc |
| **重复硬链接的删除记幽灵负账**：`create_size` 对 dup link 不入账 +size（防 `cp -l` 双算），但把全额 baseline 塞进 size_cache；`remove` 却按 baseline 全额入账 -size——`cp -l big.bin link.bin && rm link.bin` 凭空记一笔释放。新增 `dup_link_seeds` 集合，seed 路径的删除不入账；seed 被真实 modify 过则清除标记（此后按普通文件对冲）。`RenameMode::To` 同步从 `file_size` 改为 `create_size`：rename 落点是 dup link 时不再把共享数据重复入账；普通 rename 的源路径已消失、陈旧映射守卫放行，From/To 两半仍净额为零 | `argusd/watcher.rs` | daemon 记账错误（双向） | 01c27be |
| **二进制 Info.plist 丢失 bundle id**：`bundle_id_for_app` 按 UTF-8 读 plist，二进制格式（实机 229 个应用里 Xcode、Pages、Tunnelblick 等）读取失败退回 `unknown.<name>`——恰是容器按带点 bundle id 命名的大厂应用，其残留数据对卸载面板与孤儿扫描全部不可见。明文扫描抽为 `bundle_id_from_plist_xml`，macOS 上读取失败回退 `plutil -convert xml1`（OS 转换器，与既有 `mdls` 同类每应用一次 spawn，且仅对非 UTF-8 plist 触发） | `argus-core/cleaner/uninstaller.rs` | 残留检测失效（大厂应用） | 62a0099 |
| **AI `max_tokens_per_request` 一语两义**（第九轮存疑 #46 落地）：既是提示词分块预算又是 API 响应上限。批量响应带 5 个文本字段/路径、随路径数线性增长，沿用小的提示词预算会截断大批次 → JSON 解析失败 → 烧完 3 次重试返回残缺结果。响应上限拆为独立配置 `max_response_tokens`（默认 8192），经 `TuiConfig [ai]` 透传，文档同步 | `argus-core/ai.rs` `argus-tui/config.rs` + `04-configuration.md` + README | 大批次 AI 分析截断重试 | b2c3ee1 |
| **CLI `clean` 孤儿数据段"两头不靠"**（第九轮存疑 #51 落地）：扫描展示孤儿路径与体积，但 `exec_clean` 只删计划内 target。批量删除在 `--yes` 下可能误删误判目录里的用户文档，维持"逐应用复核走 TUI 卸载面板"为唯一删除路径；标题与条目行明确标注 display-only，`05-ux-interaction.md` 同步 | `argus-cli/main.rs` + `05-ux-interaction.md` | 文案误导 | da24766 |

## 存疑 / 记录在案（第十轮新增，未改动）

### 52. 已修改过的 dup-link seed 再删除时按全额对冲
`cp -l a b` 后通过 b 写入（modify 入账 +delta），再删除 b：seed 标记已被 modify 清除，删除按当前 size 全额 -size 记账，净额 `delta - size`。共享 inode 语义下"b 的修改"与"b 的删除"本就难以按路径精确归因（写穿 b 改变的是 a/b 共享的数据，删除 b 不释放数据）；当前处理把 b 从 modify 起当作普通文件对待，简单且不会静默丢账，极端场景留观。

### 53. `Snapshot::find_node` 递归深度受路径深度约束
与 #41（`collect_items` 递归）同类：按路径组件递归下降，数千层才见栈险，真实文件系统单组件 255 字节、深度远小于此。TUI 的 `find_node` 调用来自用户浏览路径，深度即用户所在层级，无实际风险。

## 第十轮验证

- `cargo test --workspace --all-features`：391 通过（新增 8 个回归测试：nav 跨根/同根多选、同名子目录可删、dup-link 删除/修改/重命名三态、AI 响应 token 独立配置、二进制 plist）
- `cargo clippy --all-targets --all-features`：0 警告；`cargo fmt --check` 干净

## 已修复（第十一轮，2026-10-03）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| AI 错误信息按裸字节切片：`raw[..raw.len().min(500)]` 在多字节内容（代理返回含 CJK 的错误页）跨越 500 字节边界时 **panic**（非字符边界）。新增 `truncate_utf8` 回退到最近字符边界 | `argus-core/ai.rs` | 崩溃（错误路径 + 低概率） | eea4017 |
| **清理详情扫描 O(n²)**：`scan_dir_details` 对每个目录条目调一次 `dir_size`，每个祖先都要重走整棵子树——`~/Library/Caches` 这类深层目录让详情弹窗转圈数分钟。改为迭代后序遍历一次累加，产出等价的逐目录累计大小，恰好走一遍树 | `argus-tui/handler/cleanup.rs` | 详情弹窗性能（分钟级 → 秒级） | 6564b2b |
| AI 审阅滚动计算在四行终端下 `visible = 0`，`visible - 1` usize 下溢（debug 构建 panic）。抽出 `review_visible_rows` 钳到最少 1 行并补测试 | `argus-tui/handler/ai_review.rs` | 崩溃（极小终端） | 4483848 |

## 存疑 / 记录在案（第十一轮新增，未改动）

### 54. uninstall 残留匹配的模糊 `contains` 有过匹配风险
`find_leftovers` 的 Application Support 扫描用 `fname.contains(&app_name.to_lowercase())` 模糊匹配：应用名短（如 `Go`、`X`）时会命中大量无关目录（`Go` 命中 `Google Drive`）。默认 8 个 LEFTOVER 目录走精确名拼接不受影响；仅 Application Support 一处模糊。删除前有 TUI 确认面板逐项复核兜底，未观察到误删；若要收紧可要求词边界匹配。

### 55. `classify_risk` 的 `contains("/Caches")` 边界宽松
`~/Library/CachesExtra` 之类名字包含 `/Caches` 的路径会被归为 Low（缓存级风险）而非 Medium。启发式分类的方向性偏差（偏宽松）只在风险标签显示与输入确认要求上体现，不拦删除；等真实路径撞上再收紧。

### 56. `fuzzy_match_indices` 非 ASCII 分支高亮下标可能错位
`target.to_lowercase()` 可能改变字符数（`İ` → `i̇`），按小写串算出的高亮区间映射回原串会偏移。只影响搜索高亮位置（无 panic、无匹配错误）；`İ` 出现在文件名首匹配段之外的几率极低，与第九轮 `:time` 修复同类但后果轻得多。

## 第十一轮性能观察

- cleanup 详情扫描的 O(n²) 是本轮唯一新发现的实际性能问题（已修复，6564b2b）。
- 第八轮的 `plan_clean` 去重 O(n²)、`lookup_scan_size` 线性扫（原 #48）等既有观察维持不变。
- daemon 侧（watcher/debounce/retention/IPC）经十轮打磨未发现新的记账或性能问题。

## 第十一轮其他改进

- daemonize 补 `chdir("/")`（4d57633）：后台进程原本终身持有启动 shell 的工作目录，把所在卷钉在 busy 状态；此后打开的 config/DB/socket/PID 文件全是绝对路径，手动 `argusd --daemon` 场景行为不变（launchd/systemd 本就以 / 为工作目录）。

## 第十一轮验证

- `cargo test --workspace --all-features`：396 通过（新增 5 个：UTF-8 截断、详情扫描单文件/嵌套累计/符号链接跳过、可见行数下限）
- `cargo clippy --all-targets --all-features`：0 警告；`cargo fmt --check` 干净

## 已修复（第十二轮，2026-10-04）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| `:Consolidate` 的失败路径（daemon 连不上、请求出错）发 `AppMessage::Info` → 状态栏按成功色渲染，失败读成成功。改走 Error 通道（与 `R` 重连一致） | `argus-tui/command.rs` | 误导性反馈 | e229cf9 |
| 卸载最终确认弹窗无条件写 "Uninstall X and remove leftovers?"，`t` 关掉残留开关后文案与实际行为相反；按开关条件显示 "(keep leftovers)" | `argus-tui/components/cleanup.rs` | 误导性确认文案 | 9b61716 |
| `render_cleanup`/`render_uninstall`/`render_brew` 每帧整状态 `clone()`（数百应用/包/清理项的字符串全量复制），渲染路径无任何变更，改借用 | 同上 + `components/brew.rs` | 每次按键的多余堆复制 | 9b61716 |
| **存疑 #56 落地**：`fuzzy_match_indices` 非 ASCII 分支按小写串数字符定位高亮，`İ`→`i̇` 这类展开让下标映射到原串不存在的位置（匹配在展开字符之后时高亮错位/越界）。改为按每字符小写展开长度回溯原串字符下标；Final_Sigma 全大小写映射不按字符复现，该罕见场景高亮可能偏 1 字符（匹配本身不受影响） | `argus-tui/search.rs` | 搜索高亮错位（显示层） | c293b12 |
| **存疑 #54 落地**：`find_leftovers` 的 Application Support 扫描对 app name 用模糊 `contains`，短名应用（`Go`）命中无关目录（`Google Drive`）。收紧为名字精确等值（小写）或 bundle id 子串（带点/去点两种形态），谓词抽出为纯函数可测。召回让位于准确：`AppName Helpers` 类变体目录不再命中，TUI 确认面板兜底 | `argus-core/cleaner/uninstaller.rs` | 残留误报（删除前最后一道人工复核被噪音稀释） | 320ac67 |
| `find_artifacts` 只看 `root/<project>/<kind>` 一层，嵌套工作区（`~/Projects/work/my-app/target`）对 purge 不可见。改为每根下深度 ≤ 4 的目录遍历：命中产物目录不再向下遍历（`node_modules` 内层副本不重复上报）、符号链接不匹配也不跟随（与 `dir_size` 纪律一致）、`project_name` 取父目录名；平面布局行为不变 | `argus-core/cleaner/purge.rs` | purge 漏报嵌套项目 | 3448cdf |
| `cargo test -p argus-core`（默认 feature）编译失败：`truncate_utf8` 的测试没挂 `#[cfg(feature = "ai")]`（基线只跑 `--all-features`，从未暴露） | `argus-core/ai.rs` | 默认 feature 测试不可编译 | 489b78c |

## 存疑关闭（第十二轮）

### 55. `classify_risk` 的 `contains("/Caches")` 边界宽松 — 关闭，维持现状（有据 won't-fix）

逐组件等值改造对该路径**行为无差**：`~/Library/CachesExtra` 即使不再命中 `/Caches` 分支，也会落进 Library 分支末尾的兜底 `return Low`，结果仍是 Low。而反向案例（`~/Library/Application SupportExtra`）现状按 contains 判 Medium，逐组件等值后反而降为 Low——**收紧边界会让分级更不保守**。contains 式匹配的方向性偏差全部朝"要求更多确认"一侧，拦不住删除但多一道键入确认，这是安全的方向。等真实路径撞上再议。

## 存疑 / 记录在案（第十二轮新增，未改动）

### 57. `App::new` 在单测里打开真实用户 DB
`ai_analyzed` 启动加载直连 `default_db_path()`：每个构造 `App` 的单测/集成测试都会 open 开发者真实的 `~/.config/argus/argus.db`（只读 + `CREATE TABLE IF NOT EXISTS`，无写路径）。当前没有测试断言 `ai_analyzed` 为空所以不炸，但测试结果隐式依赖机器状态；注入构造函数要动 `App::new` 的全部调用点，等出现真实 flaky 再做。

### 58. CLI 交互选择按格式化串反查下标
`argus uninstall` / `argus brew` 用 `inquire::Select` 的选项字符串 `position()` 反查原对象——两行格式化后完全相同（同名同版本同尺寸）时会选到第一个。真实 brew 里同名包不共存，触发面极窄；改为携带索引需自定义 Select 项类型，收益不匹配改动面。

### 59. 多选尺寸摘要只统计当前视图条目
状态栏 `MULTI(n) size` 对 `current_children` 求和：选中后进入子目录，不在视图里的选中项不计入显示（删除本身仍按完整选择集执行，不受影响）。显示口径问题，修正需要在 DirEntry 之外维护一份选择集尺寸缓存，等有人被它误导再说。

### 60. delta 详情弹窗页脚百分比与可见行口径不一致
页脚 `visible_rows = popup.height - 4`，实际行渲染用 `inner.height - 2`，滚动到底前的百分比略有偏差。纯显示层，随下次触碰该弹窗一并修正。

## 第十二轮性能观察

- 三个清理面板的每帧整状态 clone 是本轮唯一的实际性能修复（9b61716）；渲染频率受事件驱动，按键期间每次重绘都在复制数百条 AppInfo/BrewPackage。
- `find_artifacts` 深度 1 → 4 的代价是每根多几层 `read_dir`（只列目录名、不进产物子树），毫秒级；换来嵌套工作区可见性。
- 第八轮的 `plan_clean` 去重 O(n²)、`lookup_scan_size` 线性扫（原 #48）等既有观察维持不变。

## 第十二轮验证

- `cargo test --workspace --all-features`：401 通过（新增 5 个：高亮下标映射、残留匹配规则、purge 嵌套/防自嵌套/符号链接）；`cargo test --workspace`（默认 feature）同步绿
- `cargo clippy --workspace --all-targets --all-features`：0 警告；`cargo fmt --check` 干净

## 已修复（第十三轮，2026-10-05）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| `consolidate_events` 对 parent 为 `/` 的原始事件照常聚合：DELETE 的子前缀是 `//`（一行都删不掉，原始行原地不动），INSERT 却照写一条 `/` 聚合行——一条任何查询都命不中的垃圾行（`/` 查询的前缀本身是 `//`），且 `total_consolidated` 把没删掉的行计入。根级事件现在与相对路径空 parent 一样跳过 | `argus-core/db.rs` | DB 垃圾行 + 统计虚报 | 17de760 |
| **存疑 #60 落地**：delta 详情弹窗行循环按 `inner.height - 2` 取行，而表头下的实际行区是 `inner.height - 1`——表格底部永远空一行；页脚百分比又按 `popup.height - 4`（同一个少一行的口径）算。两处统一到 `visible_row_count`（弹窗高 − 2 边框 − 1 表头），弹窗用满行区、百分比与实际一致 | `argus-tui/components/delta_detail.rs` | 显示层（空行 + 百分比偏差） | 0946bc0 |
| **面板扫描失败永久卡死**：`AppMessage::Error` 从不清 `cleanup_state`/`uninstall_state` 的 `scanning`——清理目标扫描或残留扫描失败时，本该清旗标的完成消息不会到来，面板停在扫描屏，除"退出整个面板"的 Esc 外全部按键失效。Error 路径现在解除两个面板的 scanning；配套地，Uninstall 确认页等待残留扫描期间 Esc 改为返回应用列表（原来直接丢掉整个面板），迟到的扫描结果由新增的「阶段 + 所选应用」校验丢弃——否则用户退出后换了应用重选，旧应用的残留可能冒充新应用的残留出现在确认页 | `argus-tui/app.rs` `handler/cleanup.rs` | 面板不可恢复 + 确认页数据张冠李戴（竞态） | 6b142bf |
| **存疑 #57 落地**：`App::new` 直连 `default_db_path()` 加载 `ai_analyzed`，全部约 110 处测试构造都在读（并创建）开发者真实的 `~/.config/argus/argus.db`。加载抽为 `load_ai_levels_from_db_file(db_path)` 并在单测构建中跳过；加载器本身补了对临时 DB 的测试（含 DB 缺失返回空表） | `argus-tui/app.rs` | 测试隐式依赖机器状态 | 2acf194 |
| **存疑 #58 落地**：`argus uninstall`/`argus brew` 的 Select 用重新格式化的字符串反查下标，两行格式化后完全相同时永远选到第一个。改为携带源下标的 `SelectItem`（Display 输出同样的标签） | `argus-cli/main.rs` | 同名同版本同尺寸包选错 | a89ce59 |
| `query_db_size` 只看主文件：daemon 常驻 WAL 模式连接，未 checkpoint 的页常规性落在 `-wal` 侧车文件里，status 面板的 db size 长期偏小。改为主文件 + WAL 求和；AI 审阅删除确认的路径列表排序（`mark_for_delete` 是 HashSet，确认框行序与删除顺序都是随机序） | `argus-core/db.rs` `argus-tui/handler/ai_review.rs` | 显示口径 + 确定性 | d49ca06 |

## 存疑关闭（第十三轮）

### 57. `App::new` 在单测里打开真实用户 DB — 关闭（已修复，2acf194）
### 58. CLI 交互选择按格式化串反查下标 — 关闭（已修复，a89ce59）
### 60. delta 详情弹窗页脚百分比与可见行口径不一致 — 关闭（已修复，0946bc0；`K` 的滚动开关仍用 `(h*0.65)-4` 启发式，与精确值差一行，只影响边界处是否还能再滚一格，无碍）

## 存疑 / 记录在案（第十三轮新增，未改动）

### 61. 以 `/` 为根的 delta 查询永远匹配不到顶层事件
`query_delta_total/detail` 对 path `/` 构造的前缀是 `//`，而顶层事件形如 `/a.bin`——`substr(path,1,2) = "/t"` 不等。查询文件系统根实际不可用（返回 0），第十三轮的 consolidate 修复只是让根级事件不再聚合。watch dir 配成 `/` 本就不是受支持场景（配置默认 `~/Downloads`、`~/Desktop`），记录以免将来有人把「根查询返回 0」当成没有数据。

### 62. 孤儿扫描的 known 匹配仍是双向模糊 contains
第十二轮把 `find_leftovers` 收紧为「名字精确等值或 bundle id 子串」，但 `find_orphaned_data_in` 的 known 判定仍是 `fc == kc || fc.contains(kc) || kc.contains(fc)`：短名应用（`Go`）会把大量无关目录认成 known 而不进孤儿报告，目录名恰好是某个 known 名的子串时同样被吞。方向是保守的（少报孤儿），该节在 CLI 里也只是 display-only，与 #54 不同没有删除路径兜底诉求；两个表面的精度口径不一致，等孤儿报告有人当真用时再对齐。

### 63. daemon 配置里 watch_dirs 非法时整份配置回退默认值
`load_config_from_path` 只在 `[daemon] watch_dirs` 的 glob 编译失败时，把 debounce_seconds、uds_path、retention 等同文件里合法的配置一并丢掉、整份回到默认（有 eprintln + warn 提示）。按字段合并默认值是更对的行为，但改动面涉及 RawDaemonConfig 的字段级 fallback，等真实踩到再修。

## 第十三轮性能观察

- 未发现新的实际性能问题。第八轮的 `plan_clean` 去重 O(n²)、`lookup_scan_size` 线性扫（原 #48）、多选尺寸摘要只统计当前视图（#59）等既有观察维持不变。
- #59 维持记录在案：状态栏 `MULTI(n) size` 只累计当前视图条目，删除本身按完整选择集执行，显示口径问题。

## 第十三轮验证

- `cargo test --workspace --all-features`：408 通过（新增 7 个：consolidate 根路径、弹窗行数、面板失败恢复、Confirm Esc 返回、迟到残留丢弃、AI 等级加载器、WAL 尺寸）；`cargo test --workspace`（默认 feature）同步绿
- `cargo clippy --workspace --all-targets --all-features`：0 警告；`cargo fmt --check` 干净

## 已修复（第十四轮，2026-10-06）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| 审计日志路径硬编码 `~/.config/argus`，是全仓唯一不尊重 `XDG_CONFIG_HOME` 的状态路径：设置了 XDG 时 DB 在 `$XDG_CONFIG_HOME/argus/`、审计轨迹在 `~/.config/argus/`，分家。config 目录解析同时散落 5 处（core/db、tui/util ×2、argusd/config、argusd/daemonize），各处 fallback 还不一致（TUI 日志回退 `/tmp`，其余回退 `.`）。收敛为 `argus_core::config_dir()` 单一权威 + 可测纯函数 `config_dir_from`（XDG > HOME/.config > `.`；空 XDG 视为未设置） | `argus-core/db.rs` `cleaner/audit.rs` `argus-tui/util.rs` `argusd/config.rs` `argusd/daemonize.rs` | 状态文件路径分裂 + DRY | 35f3d51 6d14774 |
| **macOS rename 完全不记账**：notify 的 FSEvents 后端对 rename 每侧各发一条 `Modify(Name(RenameMode::Any))`、从无 From/To 配对（kqueue 对源侧同样发 Any），watcher 只处理 From/To——`mv` 后旧路径的 size 永久留在账上，新路径没有基线（后续 modify 全部不记账）。现在 Any 侧按当前存在性判定：仍在 → `create_size`（dup-link 感知），已消失 → `remove`。inotify 的配对 rename 发 From/To/Both、从不含 Any，Linux 时序不受影响 | `argusd/watcher.rs` | delta 记账错误（macOS 主平台） | a0df5ea |
| 目录级删除不记账：FSEvents/inotify 都会把删除报成 `Remove(Folder)`，之前落入 catch-all。新增 `WatcherState::remove_tree`：把该前缀下仍缓存的尺寸求和、清除（含 dup-link 种子与硬链接映射）后记一条负 delta。逐文件事件正常到达时缓存已排空、计 0 不重复记账；FSEvents 合并吞掉逐文件事件时这是唯一入账机会 | `argusd/watcher.rs` | delta 记账缺口（事件合并场景） | a0df5ea |
| 多选批删不去重嵌套选择：先选子项、回退再选其祖先（合法操作序列，多选跨目录保留）后，HashSet 迭代顺序决定谁先进废纸篓——祖先先删则嵌套项必报一条 ENOENT「删除失败」。现在外层选择胜出（与 `plan_clean` 同规则），跳过数进提示消息 | `argus-tui/handler/browsing.rs` | 批删误报失败 | a6e554f |
| K 弹窗（delta detail）取数失败把用户困在看不见的弹窗里：模式先切到 DeltaDetail 再异步取数，失败时 `delta_detail` 为 None、渲染层什么都不画，只有 Esc 能脱身。Error 到达时若弹窗仍空则自动退回浏览列表；已加载的弹窗不受无关错误影响 | `argus-tui/app.rs` | 交互死区 | f010366 |
| brew 展示排序（never 最前、组内按尺寸降序、再按 LRU 升序）在 `list_brew_packages` 与 CLI 过滤后重排各有一份相同闭包，规则漂移即静默不一致。收敛为 `argus_core::sort_oldest_first`，测试钉住顺序 | `argus-core/cleaner/brew.rs` `argus-cli/main.rs` | DRY | 43486d4 |
| `App::set_error` 内联了一份日志写入（无毫秒时间戳），与 `log_msg` 漂移。改走 `log_msg` | `argus-tui/app.rs` | DRY | c05779a |

文档同步：`07-safety.md`/`10-cleaner.md`/`11-logging.md` 的审计日志路径补 XDG 说明；`phase3-daemon-design.md` §4.3 事件映射表补 Any/Folder 两行；`tui-current-behavior.md` 补批删去重与 K 弹窗失败回退。

## 存疑 / 记录在案（第十四轮新增，未改动）

### 64. cask 的 Spotlight last_used 匹配过宽且大小写不对称
`spotlight_last_used` 判定 `stem.eq_ignore_ascii_case(pkg_name) || stem.replace(' ', "-").eq_ignore_ascii_case(pkg_name) || pkg_name.contains(&stem.to_lowercase())`：第三个条件里 stem 被小写而 `pkg_name` 没有（`pkg_name` 含大写时永不命中），且 `contains` 过宽——包名 `google-chrome` 会把任何名字含 "chrome" 的 .app（含非 brew 安装的和 Chromium 衍生品）当作匹配对象并 spawn mdls。影响仅限 last_used 显示（结果偏向「最近用过」，保守方向：推迟而非催促卸载建议），等有人对 cask 的 last_used 当真时再收紧为与 uninstaller 相同的精确/子串规则。

### 65. brew 依赖错误解析依赖英文文案
`AppMessage::Error` 的 brew 分支用 `e.split("required by ")` 从 brew 的报错里抠依赖列表。brew 本地化或改版时解析静默失效——后果只是确认弹窗缺 dependents 提示（错误本身仍完整显示），无数据风险。要稳就得让 core 的 `uninstall_brew_package` 结构化返回依赖信息，等真实踩到再做。

### 66. FSEvents 事件丢弃（MUST_SCAN_SUBDIRS）无补偿
notify 把 FSEvents 的 kernel-dropped/user-dropped 转成 `EventKind::Other` + `Flag::Rescan`，watcher 目前忽略。事件被内核丢弃后该段搅动永久缺失，第十四轮的 folder 兜底只覆盖「目录级删除仍被送达」的场景，不覆盖整段事件流丢失。正确修法是 daemon 收到 Rescan 旗标时对受影响 watch dir 做一次增量重扫（与 0..基线的 scan_cache 对账），属于设计级改动，先记录。

## 第十四轮性能观察

- 未发现新的实际性能问题。既有观察维持：`plan_clean` 去重 O(n²)、`lookup_scan_size` 线性扫（原 #48）、多选尺寸摘要只统计当前视图（#59）。`remove_tree` 对 size_cache 的一次 O(n) 遍历只发生在目录级删除事件时，量级与单文件事件相同缓存下的查询相当，不构成热点。

## 第十四轮验证

- `cargo test --workspace --all-features`：416 通过（新增 8 个：config_dir 回退链、rename Any 双侧记账、folder 删除一次性入账 + 二次为空、目录 rename 不记账、嵌套选择去重、delta-detail 失败回退 + 已加载不受扰、brew 排序）；`cargo test --workspace`（默认 feature）同步绿
- `cargo clippy --workspace --all-targets --all-features`：0 警告；`cargo fmt --check` 干净

## 已修复（第十五轮，2026-10-07）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| **存疑 #59 落地**：多选尺寸摘要只对 `current_children` 求和——多选跨目录保留后，不在当前视图里的选中项从状态栏总数中消失（删除本身按完整选择集执行，显示口径与实际不符）。改为从树快照按选择键解析每个条目的尺寸，口径与可见行同源 | `argus-tui/app.rs` | 状态栏少报多选总量 | f566acb |
| Clean/Purge 面板列表按 `CleanupState::scroll_offset` 滚动，但没有任何 handler 写过这个字段——条目多于终端行数时光标移出冻结的视口后消失（`g`/`G` 之外所有移动都看不见）。改为与 brew/uninstall 列表一致的光标锚定滚动，删除死字段 | `argus-tui/components/cleanup.rs` `types.rs` | 面板不可见光标 | 7214d34 |
| **搜索 Active 模式按键泄漏**：`n`/`N`/`/`/`Esc` 里只有 `Esc` 上报「已消费」，其余落到浏览层——`N`（上一个匹配）同时是浏览层的永久删除快捷键，翻匹配会连带弹出永久删除确认；列表头承诺的 "Enter edit" 也不存在（Enter 实际进入光标目录并丢弃搜索）。四个键现在全部消费；`j`/`k` 与其余键继续放行，搜索态下浏览照常 | `argus-tui/handler/search.rs` | 误触删除确认 + 交互与提示不符 | 842ea0a |
| `:Connect` 失败走 `AppMessage::Info`（成功色渲染），失败读成成功——与第十二轮 `:Consolidate` 同类问题，当时漏掉了这一处。改走 Error 通道 | `argus-tui/handler/command.rs` | 误导性反馈 | 48f8868 |
| **存疑 #64 落地**：cask 的 Spotlight 匹配第三个条件只小写 stem（cask 名带大写时永不命中），且裸 `contains` 让 `google-chrome` 命中 `Chromium.app` 这类跨产品。新匹配器按 token 对齐：app 名的每个词都必须被 cask token 覆盖，允许数字后缀扩展（`iTerm` ↔ `iterm2`，与 shell-history 规则同源）。漏匹配会落到 keg atime 再到「从未使用」，方向偏催促卸载，因此收紧只丢跨产品误报、保留大小写/连字符/版本数字变体的召回 | `argus-core/cleaner/brew.rs` | last_used 误报 + 大小写不对称 | 3717f4e |
| **存疑 #63 落地**：daemon 配置 `watch_dirs` 里一个 glob 拼写错误把同文件全部字段一起丢掉（debounce_seconds、uds_path、retention 静默回默认）。现在只有监控列表回退默认，其余字段重新解析保留 | `argusd/config.rs` | 一处拼写重置全部配置 | 11f0cab |
| brew 面板 `o` 键的 Time 排序仍持有一份最旧优先比较器的私有副本（第十四轮统一了扫描序与 CLI，漏了这第三份）；CLI 与 TUI 还各有一份相同的 never/today/yesterday 相对时间格式化。收敛为 `argus_core::compare_oldest_first` 与 `argus_core::format_last_used_relative` | `argus-core/cleaner/brew.rs` `argus-tui/handler/brew.rs` `argus-cli/main.rs` | DRY | 48f8868 |
| brew 扫描进行中退出面板再按 `B` 重入，会整体替换状态并起第二个全量扫描（history 重读 + 逐包 keg 遍历 + mdls spawn）；先完成的旧结果随后覆盖新状态。扫描在途时重入只显示面板的扫描屏 | `argus-tui/app.rs` | 重复扫描 + 迟到结果覆盖 | bed6502 |

文档同步：`04-configuration.md` 补 argusd 配置按字段回退的说明；`tui-current-behavior.md` 补搜索态按键语义、Clean/Purge 滚动与多选摘要口径。doc-poste 的 argus 概览核验后无需改动（本轮修复都在交互细节层，低于概览的陈述粒度，无陈述失效）。

## 存疑关闭（第十五轮）

### 59. 多选尺寸摘要只统计当前视图条目 — 关闭（已修复，f566acb）
### 63. daemon 配置里 watch_dirs 非法时整份配置回退默认值 — 关闭（已修复，11f0cab）
### 64. cask 的 Spotlight last_used 匹配过宽且大小写不对称 — 关闭（已修复，3717f4e）

## 存疑 / 记录在案（第十五轮新增，未改动）

### 67. delta 详情弹窗可滚过最后满页
`handle_delta_detail_key` 的 `j` 允许 scroll 到 `entries.len() - 1`：超过「最后一满页」后继续按 `j`，底部条目逐渐升到弹窗顶部、下方留白，页脚百分比提前到 100%。多数 TUI 把偏移钳在 `len - visible`。修这个需要 handler 复现渲染层 `centered_rect` 的百分比弹窗高度（第十三轮已记录过两侧各一行的启发式差异），两处口径要一起收敛，等下次触碰该弹窗再做。

### 68. watcher 不处理 `RenameMode::Both`
Windows 后端把 rename 报成携带双路径的 `RenameMode::Both`，watcher 落入 catch-all 不记账。argusd 的目标平台是 macOS/Linux（inotify 发 From/To、FSEvents 发 Any，均已处理），macOS 上不会出现 Both；记录以免将来移植 Windows 时把 rename 缺账当成新 bug。

## 第十五轮性能观察

- 未发现新的实际性能问题。`spotlight_last_used` 的 token 匹配同时减少了无关 .app 的 mdls spawn 次数（原 contains 规则命中的每个错误候选都要 spawn 一次）。既有观察维持：`plan_clean` 去重 O(n²)、`lookup_scan_size` 线性扫（原 #48）。
- `selected_total_size` 改为对选择集逐键 `find_node`（每层子节点线性扫）：选择集是人为规模（数十条），状态栏每帧的开销可忽略。

## 第十五轮验证

- `cargo test --workspace --all-features`：423 通过（新增 7 个：搜索态按键消费 ×2、brew 比较器一致性、相对时间格式化、cask 匹配规则、watch_dirs 按字段回退、多选摘要含视图外条目）；`cargo test --workspace`（默认 feature）419 同步绿
- `cargo clippy --workspace --all-targets --all-features`：0 警告；`cargo fmt --check` 干净

## 已修复（第十六轮，2026-10-08）

| 问题 | 位置 | 影响 | Commit |
|------|------|------|--------|
| **目录删除以 `RemoveKind::Any` 面目出现时不记账**：kqueue 把所有删除（含目录）都报成 Any，降级的 FSEvents 事件丢失 `IS_DIR` 标志后同样落到 Any。Any 分支只查路径本身的 size_cache，而目录从不进缓存——被删目录下已缓存的子树 churn 静默丢失，缓存条目泄漏。普通 remove 返回 None 时回退到前缀扫（`remove_tree`），对普通文件是 no-op（`starts_with` 按组件比较）；每文件删除事件先耗尽缓存的常规路径不会二次入账 | `argusd/src/watcher.rs` | 目录删除漏账 + 缓存泄漏 | 653586e |
| **存疑 #67 落地**：delta 详情弹窗 `j` 可滚到最后一行升顶、底部留白。handler 的可视行数来自 `(h*0.65)-4` 启发式，与渲染层 `centered_rect` 实际布局差一行；两侧收敛为同一几何函数（零宽探针 Rect 复用 `centered_rect`），滚动钳制在 `len - visible`（最后满页），页脚百分比恰在满页时到 100% | `argus-tui/handler/delta_detail.rs` `components/delta_detail.rs` | 弹窗滚动越界 + 两侧口径漂移 | c5619ed |
| **Clean/Uninstall 面板重入叠加扫描**：扫描在途退出再进入会整体替换状态并起第二份全量扫描（Purge 深扫所有项目树、Uninstall 逐应用 spawn mdls），旧完成消息还会落进新状态。与第十五轮 brew 同一修法：在途状态在退出时保留（重入只显示扫描屏），完成的结果同模式重入时复用（Uninstall 重置回应用列表）；完成的面板退出仍丢弃状态，下次进入重扫保证新鲜 | `argus-tui/app.rs` | 重复扫描 + 旧结果竞态 | 495c7fa |
| **AI 审阅删除在 UI 线程同步执行**：`D`（永久删除）大目录时 `remove_dir_all` 冻结界面数十秒（浏览模式删除早已后台化 + 进度条）。确认后由后台线程执行（新增 `AiStatus::Deleting`：删除/标记键失效、标题显示 deleting…），新增 `AiDeleteComplete` 消息负责树剪枝、释放字节记账与结果清理；用户中途退出面板后完成消息仍会补齐文件系统侧的状态维护。受保护路径闸门不变 | `argus-tui/handler/ai_review.rs` `app.rs` `types.rs` | 大目录删除冻结 UI | 1423f2d |
| `pub mod bloom` 无任何公开项（`SeenInodes` 是 `pub(crate)`），收敛为私有模块 | `argus-core/src/lib.rs` | API 面噪声 | 1423f2d |

## 存疑关闭（第十六轮）

### 67. delta 详情弹窗可滚过最后满页 — 关闭（已修复，c5619ed；handler 与渲染层共用 `centered_rect` 几何，第十三轮记录的两侧启发式差一行的口径分叉一并消除）

## 存疑 / 记录在案（第十六轮新增，未改动）

### 69. `consolidate_events` 的事务外 SELECT
聚合计数在事务开窗前流式读全表，DELETE + INSERT 在其后的事务里执行；两步之间到来的事件会被 DELETE 删掉却不进聚合和（丢账）。当前不可触发：`insert_events` 只有 argusd 调用，daemon 内部全部 DB 访问共享一把 `Arc<Mutex<Connection>>`，单实例守卫挡住第二个 daemon；CLI/TUI 只读 delta 表。把 SELECT 挪进事务（快照隔离，冲突时报 BUSY 而非静默丢账）是正确修法，等出现第二个写方（如独立的 ingest 工具）再做。

### 70. `uninstall_app_with_leftovers` 对每个残留重跑 `dir_size`
`find_leftovers` 刚为每个残留算过 `dir_size`，确认卸载时 `uninstall_app_with_leftovers` 按路径再算一遍（每残留一次全树遍历）。CLI 路径（`uninstall_app`）因此每个残留走两遍；TUI 路径同样付两次。要把尺寸穿进 API（`AppLeftovers` 增加逐路径尺寸）才消得掉，删除本身已是秒级操作，收益有限，记录备查。

### 71. 关停时 debounce 通道内未合并事件丢弃
`SHOULD_QUIT` 后 debounce 引擎的 select 循环随即退出，只 flush 已合并的 pending；此刻仍在 `event_rx` 里排队（容量 1024）的原始事件不合并直接丢弃。关停前最后两秒内的 churn 可能缺账。要修需在退出前 drain 通道（ watcher 线程还有 ≤1s 的 `recv_timeout` 尾巴，要一起等），收益是关停时刻附近几百毫秒的完整性。

## 第十六轮性能观察

- AI 审阅删除后台化同时消除了一个隐性 UI 卡顿源（原同步路径）；`delete_marked` 对失败路径只报错不更新状态，与原语义一致。
- 未发现新的实际性能问题。既有观察维持：`plan_clean` 去重 O(n²)、`lookup_scan_size` 线性扫（原 #48）、残留尺寸双算（新 #70）。

## 第十六轮验证

- `cargo test --workspace --all-features`：437 通过（新增 14 个：watcher Any-remove 目录删除 ×2、delta-detail 滚动钳制 ×3 + 几何一致性、面板重入 ×5、`delete_marked` 永久删除、AI 删除完成消息 ×1、既有 uninstall-Esc 测试按新语义更新）；`cargo test --workspace`（默认 feature）同步绿
- `cargo clippy --workspace --all-targets --all-features`：0 警告；`cargo fmt --check` 干净
- 文档同步：`tui-current-behavior.md`（面板重入语义、AI 删除后台化、K 弹窗滚动钳制）
