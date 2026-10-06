# 配置系统设计

配置文件路径：`~/.config/argus/config.toml`（遵循 `XDG_CONFIG_HOME`）。

各客户端（CLI/TUI/Daemon）启动时自动加载；文件不存在时不报错，使用全默认值；解析失败时回退默认值并向 stderr 输出警告（daemon 的 tracing 此时可能尚未初始化）。argusd 的 `[daemon]` 表内字段按字段校验：仅 `watch_dirs` 含非法 glob 时，只有监控列表回退默认值，同文件其他合法字段（debounce_seconds、uds_path 等）仍然生效。

> 本文档描述**实际实现**的配置项。规划中但未实现的配置（键位重映射、自定义标签、扫描忽略规则、Token 统计）见文末 §7，代码不解析这些节，写了也不生效。

## 1. AI 配置组 `[ai]`（argus-tui）

AI 功能默认关闭，用户无需配置即可使用全部传统功能。

```toml
[ai]
# 总开关：false 时完全禁用 AI 相关功能
enabled = false

# 模型名称，默认支持 gpt-4o, gemini-1.5-flash 等
model = "gpt-4o"

# API 密钥
api_key = ""

# 自定义中转 URL（兼容 OpenAI 格式的任意服务）
api_url = ""

# AI 输出语言（BCP 47 标签）。所有文本字段（label_detail, description, suggestion）
# 将使用此语言返回。默认 en-US，用户可按需配置如 zh-CN、ja-JP 等。
language = "en-US"

# 单次请求的提示词 Token 预算（估算超出时自动分块）
max_tokens_per_request = 4096

# API 请求的响应 Token 上限（独立于提示词预算：批量响应随路径数线性
# 增长，沿用小的提示词预算会截断大批次响应并触发无谓重试）
max_response_tokens = 8192
```

网络抖动、超时、429/5xx 会自动重试（线性退避，最多 3 次）；401/400 等永久性错误立即失败。

## 2. 色彩与主题组 `[theme]`（argus-tui）

TUI 使用 `ColorTheme` 语义颜色系统，内置暗/亮两套主题。
`color_scheme` 控制主题选择逻辑：

- `"light"` — 始终使用亮色主题
- `"dark"` — 始终使用暗色主题
- `"system"` (默认) — 通过 `terminal-light` crate 自动检测终端背景亮度

```toml
[theme]
color_scheme = "system"
```

具体颜色（涨跌色、单位色等）由主题内置决定，暂不支持逐项覆盖。

## 3. 浏览配置组 `[browsing]`（argus-tui）

```toml
[browsing]
# 启动时是否自动扫描当前工作目录
# false: 启动后展示纯文件树，按 s 手动扫描
# true:  启动后立即在后台扫描 cwd，完成后刷新 size overlay
auto_scan_on_start = false
```

## 4. TUI 守护进程访问组 `[daemon]`（argus-tui，与 argusd 的 `[daemon]` 同名但字段不同）

```toml
[daemon]
# TUI 连接守护进程使用的 UDS socket 路径。
# 所有 daemon 调用（自动探测、:R 重连、:Connect、delta 查询、:Consolidate）
# 都使用此路径。
uds_path = "/tmp/argusd.sock"

# 未连接时自动探测守护进程的间隔（秒）；0 关闭周期性探测
detection_interval_secs = 5
```

## 5. 守护进程组 `[daemon]`（argusd）

```toml
[daemon]
# 监控的根目录列表（支持纯路径字符串或带过滤规则的结构化格式）
# 冲突规则：路径前缀最长的 watch_dir 的 filter 生效
watch_dirs = [
    "~/Downloads",   # 注意：TOML 不展开 ~，此处仅为示例；应写绝对路径
    { path = "/home/user/downloads", include = "*.{pdf,iso,dmg}" },
    { path = "/var/log", include = "*.log", exclude = "*.gz" },
]
# 未配置 watch_dirs 时的默认值：$HOME/Downloads 与 $HOME/Desktop（运行时从环境推导）

# include/exclude 的 glob 语法（基于 globset 库，默认不区分大小写）：
# 每个 watch_dir 可设置 include/exclude 过滤规则，仅匹配的文件事件被记录。
# 语法参考: https://docs.rs/globset/latest/globset/#syntax

# 事件去抖延迟（秒）
debounce_seconds = 10

# UDS 监听地址
uds_path = "/tmp/argusd.sock"

# delta 事件保留天数（超过此天数的原始事件会被后台清理）
delta_retention_days = 30

# 目录级事件合并策略
# 当某个目录的直接子级变更数超过阈值时，自动合并为一条汇总记录
[daemon.consolidation]
# 子级变更数阈值，超过则合并（0 表示禁用合并）
sibling_threshold = 500
# 合并任务执行间隔（分钟）
interval_minutes = 60
```

监控行为补充（代码内置，不可配置）：

- 所有点文件/目录（含 `.DS_Store`、`.git`）以及 `~`、`*.swp`/`*.swx`、结尾 `~` 的事件被忽略。
- 目录本身不参与尺寸记账户（目录 stat 大小不是用户数据）；只有文件事件产生 delta。

## 6. 配置管理需求

- 配置文件使用 TOML 格式，支持 Rust 的 `serde` 直接反序列化。
- 所有配置项均有合理默认值，用户可增量覆盖。
- 客户端只解析已实现的配置节；未知/历史遗留的节被 serde 静默忽略（兼容旧配置文件）。
- 配置在启动时读取一次，运行期不热加载。

## 7. 规划中（未实现，解析器不读取）

以下配置在需求阶段设计过，当前代码**不解析、不生效**，列出以避免误用；实现对应功能时应移回正文：

| 配置组 | 状态 |
|--------|------|
| `[keybindings]` 键位重映射 | 未实现，TUI 键位硬编码 |
| `[labels]` 自定义 path→label 映射 | 未实现，标签仅由内置启发式决定 |
| `[theme].colors` 逐项颜色覆盖 | 未实现 |
| `[ignore]` 扫描忽略规则 | 未实现，扫描器当前始终包含隐藏文件 |
| `[token_usage]` Token 消耗统计 | 未实现 |
