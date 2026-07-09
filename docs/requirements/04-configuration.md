# 配置系统设计

配置文件路径：`~/.config/argus/config.toml`

各客户端（CLI/TUI/GUI/Daemon）启动时自动加载。

> **Phase 1 范围**：仅加载 `[ignore]` 组规则（参见 `10-phase1-guide.md` §2.4）。AI/快捷键/主题/守护进程等配置组留待对应 Phase 实现。配置文件不存在时不报错，使用全默认值。

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
sort_toggle = "o"         # 在体积排序与增量排序间切换
ai_diagnose = "a"         # Phase 4：手动触发 AI 诊断
delete_item = "d"
focus_panel = "tab"       # 在主要面板之间切换焦点（Phase 2 仅文件树 / 筛选栏 / 元数据）
quit = "q"
```

## 3. 色彩与主题组 `[theme]`

支持完全自定义 TUI/GUI 的视觉表现。

```toml
[theme]
# 内置方案: "nord", "dracula", "gruvbox", "system" (跟随系统暗黑模式)
color_scheme = "system"

# 以下为各项指标的精确颜色控制（十六进制 RGB）
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
# 监控的根目录列表
watch_dirs = ["/home/user", "/var/log"]

# 事件去抖延迟（秒）
debounce_seconds = 10

# 快照保留策略
[daemon.snapshot_retention]
hourly_retention_days = 7
daily_retention_days = 30

# UDS 监听地址
uds_path = "/tmp/argusd.sock"
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
# false: 仅展示 FS 文件树（目录 size 显示 "-"），按 s 手动扫描
# true:  启动后立即在后台扫描 cwd，完成后展示完整数据
auto_scan_on_start = false
# 扫描时跳过以下目录（仅记录直接子级的大小，不递归深入）
# 匹配规则：目录名完全匹配即跳过
skip_dirs = ["node_modules", "target", ".git", "__pycache__", ".venv"]
```

## 7. 配置管理需求

- 配置文件使用 TOML 格式，支持 Rust 的 `serde` 直接反序列化。
- 所有配置项均有合理默认值，用户可增量覆盖。
- 配置文件变更后无需重启守护进程，客户端下次连接时自动检测变更。
- Phase 2+ 支持通过 CLI 命令快速查看和修改配置（`argus config show` / `argus config set <key> <value>`）。Phase 1 仅加载 `[ignore]` 配置，不实现配置修改命令。
