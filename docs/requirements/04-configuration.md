# 配置系统设计

配置文件路径：`~/.config/argus/config.toml`

各客户端（CLI/TUI/GUI/Daemon）启动时自动加载。

> **Phase 1 范围**：仅加载 `[ignore]` 组规则（参见 archived `10-phase1-guide.md` §2.4）。AI/快捷键/主题/守护进程等配置组留待对应 Phase 实现。配置文件不存在时不报错，使用全默认值。

## 1. AI 配置组 `[ai]`

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

# 单次请求 Token 上限保护
max_tokens_per_request = 4096
```

## 2. 交互快捷键组 `[keybindings]`

允许用户完全自定义 Vim-like 键位，防范键位冲突。

```toml
[keybindings]
move_up = "k"
move_down = "j"
enter_dir = "l"
leave_dir = "h"
sort_toggle = "o"         # 在名称排序与体积排序间切换
delete_item = "d"
focus_panel = "tab"       # 在主要面板之间切换焦点
quit = "q"
```

## 3. 色彩与主题组 `[theme]`

TUI 使用 `ColorTheme` 语义颜色系统，内置暗/亮两套主题。
`color_scheme` 控制主题选择逻辑：

- `"light"` — 始终使用亮色主题
- `"dark"` — 始终使用暗色主题
- `"system"` (默认) — 通过 `terminal-light` crate 自动检测终端背景亮度

```toml
[theme]
color_scheme = "system"

# 以下为各项指标的精确颜色控制（十六进制 RGB），留作未来扩展
[theme.colors]
growth_high = "#FF4444"     # 暴涨颜色
growth_medium = "#FF8800"   # 中度增长
shrink_green = "#44FF44"    # 减少颜色
text_primary = "#FFFFFF"    # 主文本
ai_panel_border = "#8888FF" # Phase 4：AI 面板边框
```

## 4. 扫描忽略规则组 `[ignore]`

```toml
[ignore]
# 是否忽略以 "." 开头的隐藏文件/目录
ignore_hidden = true

# 是否跟随符号链接（false 避免循环链接和重复统计）
follow_symlinks = false

# 用户自定义忽略路径（glob 模式）
custom_ignore_paths = [
    "*/.git/*",
    "*/node_modules/*",
    "*/target/*",
    "*/vendor/*",
    "*.pyc",
]
```

## 5. 守护进程组 `[daemon]`

```toml
[daemon]
# 监控的根目录列表（支持纯路径字符串或带过滤规则的结构化格式）
# 冲突规则：路径前缀最长的 watch_dir 的 filter 生效
watch_dirs = [
    "/home/user/docs",
    { path = "/home/user/downloads", include = "*.{pdf,iso,dmg}" },
    { path = "/var/log", include = "*.log", exclude = "*.gz" },
]

# include/exclude 的 glob 语法（基于 globset 库，默认不区分大小写）：
# 每个 watch_dir 可设置 include/exclude 过滤规则，仅匹配的文件事件被记录。
# 冲突规则：路径前缀最长的 watch_dir 的 filter 生效。
# 语法参考: https://docs.rs/globset/latest/globset/#syntax

# 事件去抖延迟（秒）
debounce_seconds = 10

# 快照保留策略
[daemon.snapshot_retention]
hourly_retention_days = 7
daily_retention_days = 30

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

## 6. Token 消耗统计 `[token_usage]`

```toml
[token_usage]
# 是否记录 Token 消耗历史
track_enabled = true

# 每日 Token 上限（0 表示不限制）
daily_limit = 0
```

## 8. 浏览配置组 `[browsing]`

```toml
[browsing]
# 启动时是否自动扫描当前工作目录
# false: 启动后展示纯文件树，按 s 手动扫描
# true:  启动后立即在后台扫描 cwd，完成后刷新 size overlay
auto_scan_on_start = false
```

## 9. 标签配置组 `[labels]`

自定义 path→label 映射，覆盖内置启发式规则。

```toml
[labels]
# 自定义路径到标签的映射（glob 模式，优先于内置启发式）
# 匹配规则：使用 globset 语法，路径匹配时优先使用用户配置的 label
custom_mappings = [
    { pattern = "*/.terraform/*", label = "iac-cache" },
    { pattern = "*.pyc",          label = "python-bytecode" },
    { pattern = ".venv/*",        label = "python-virtualenv" },
]
```

Label 不直接由 AI 决定——AI 只输出 `label_detail`（自由描述），程序根据内置规则 + 用户配置确定 `label`（稳定分类）。

## 10. 配置管理需求

- 配置文件使用 TOML 格式，支持 Rust 的 `serde` 直接反序列化。
- 所有配置项均有合理默认值，用户可增量覆盖。
- 配置文件变更后无需重启守护进程，客户端下次连接时自动检测变更。
- Phase 2+ 支持通过 CLI 命令快速查看和修改配置（`argus config show` / `argus config set <key> <value>`）。Phase 1 仅加载 `[ignore]` 配置，不实现配置修改命令。
