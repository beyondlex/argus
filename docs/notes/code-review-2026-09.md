# 代码审查记录 — 2026-09

全库检视（argus-core / argus-cli / argus-tui / argusd）的结果归档。
已修复的问题有对应 commit；"存疑/记录" 类问题保持现状，等出现实际症状或专门排期再处理。

## 已修复（第一轮）

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

### 18. `find_orphaned_data` 的已知性判断双向 contains，容易漏报孤儿
`fc.contains(kc) || kc.contains(fc)`：名为 "Go" 的应用会把 "golang"、"google-cloud-sdk" 全部判为已知，孤儿数据漏报。方向保守（宁可漏删不可误删），可接受；若要更准可改成词边界匹配或仅精确等值 + bundle id 前缀。

### 19. IPC 客户端读响应无长度上限
daemon 端对请求设了 `MAX_PAYLOAD_LEN`，但 TUI/CLI 的 `send_request` 按 daemon 报头分配响应缓冲。对端是本机同用户的可信进程，风险低；若要对称加固，读响应时套同样的上限。

### 20. 单实例守卫依赖 PID 文件，崩溃后可能误报"已在运行"
`DaemonGuard::acquire` 以 `kill(pid, 0)` 判断存活；崩溃残留的 PID 文件若被回收复用会拒绝启动（错误信息已提示 `argusd stop` 恢复）。与 stop() 的既有 PID 复用问题（存疑 #6）同源，一并留待 pidfd/进程名校验方案。
