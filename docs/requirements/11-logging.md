# 日志系统设计

## 1. 设计原则

### 1.1 目标

- **AI Agent 可读**：结构化 JSON 日志，无需人工即可定位故障
- **可观测性**：每个关键操作有起止 span，可追踪全链路
- **隐私安全**：绝不记录文件内容、AI Prompt / Response body、API Key
- **轻量**：闲置时零日志写入，不影响磁盘 I/O

### 1.2 技术选型

| 组件 | 选型 | 理由 |
|------|------|------|
| 核心框架 | `tracing` | Rust 生态标准，支持 span + event + 结构化字段 |
| 终端输出 | `tracing-subscriber` | 人类友好彩色格式，开发时实时查看 |
| 文件输出 | `tracing-appender` | 非阻塞写入，支持轮转 |
| 格式化 | `tracing-subscriber` JSON layer | 机器可解析，AI Agent 消费 |

### 1.3 不得记录的内容

- `api_key` 及其他密钥
- AI Prompt 全文（只记录 `prompt_tokens` 和 `target_path`）
- AI Response body（只记录 `completion_tokens` 和解析后的 `risk_level`）
- 文件具体内容

## 2. 日志等级规范

| 等级 | 用途 | 示例 |
|------|------|------|
| `ERROR` | 组件不可恢复故障 | 快照损坏无法反序列化、UDS 连接拒绝 |
| `WARN` | 降级行为 / 可恢复错误 | 权限不足跳过文件、IO 超时、配置文件不完整 |
| `INFO` | 操作边界 | 扫描启动/完成、Diff 计算完成、文件移入废纸篓、AI 诊断完成 |
| `DEBUG` | 详细流程 | 扫描进度（每 10k 文件）、硬链接去重命中、缓存命中/未命中 |
| `TRACE` | 逐条目跟踪 | WalkBuilder 每步回调、AI 请求/响应原始数据（不含 body） |

**默认等级**：
- CLI/TUI 终端输出：`INFO`
- CLI/TUI 文件输出：`DEBUG`
- Daemon 文件输出：`INFO`（守护进程长期运行，避免过多写入）
- Daemon 终端输出：无（daemon 不附加终端）

## 3. 日志结构

### 3.1 JSON 格式（文件输出 / AI Agent 消费）

```json
{
  "timestamp": "2026-07-08T10:30:00.123456Z",
  "level": "INFO",
  "target": "argus_core::scanner",
  "message": "scan completed",
  "span": {
    "name": "scan_directory",
    "id": 42,
    "parent": 18
  },
  "fields": {
    "path": "/home/user/Downloads",
    "file_count": 58432,
    "total_size": 2417483648,
    "duration_ms": 3420
  }
}
```

| 顶级字段 | 说明 |
|---------|------|
| `timestamp` | RFC3339 纳秒精度 |
| `level` | `TRACE` / `DEBUG` / `INFO` / `WARN` / `ERROR` |
| `target` | Rust module path，如 `argus_core::scanner` |
| `message` | 简短的人类可读描述 |
| `span` | 关联的 tracing span 信息（id + parent 用于重建调用链） |
| `fields` | 结构化 KV，每类事件自有 schema |
| `error` | ERROR/WARN 时附加的 `Display` 错误信息 |

### 3.2 文本格式（终端输出）

```
2026-07-08T10:30:00.123Z  INFO argus_core::scanner scan completed
    path=/home/user/Downloads file_count=58432 total_size=2.3GB duration_ms=3420
```

终端输出使用彩色等级标记：
- `ERROR`：红色
- `WARN`：黄色
- `INFO`：绿色
- `DEBUG` / `TRACE`：灰色（默认隐藏，`--verbose` 开启）

## 4. Span 定义

Span 覆盖所有跨越异步边界的操作，用于串联请求链路。

| Span 名 | 父 Span | 属性 | 说明 |
|---------|---------|------|------|
| `scan_directory` | — | `path`, `file_count`, `total_size`, `duration_ms` | 全目录扫描 |
| `load_snapshot` | — | `path`, `version`, `root_path` | 加载快照文件 |
| `compare_diff` | — | `old_snapshot`, `new_snapshot`, `threshold` | Tree Merge Diff 计算 |
| `ai_batch_analyze` | — | `batch_size`, `strategy`, `total_tokens` | 批量 AI 请求 |
| `ai_single_analyze` | `ai_batch_analyze` | `path`, `prompt_tokens`, `risk_level` | 单条路径 AI 诊断 |
| `delete_to_trash` | — | `path`, `size`, `success` | 移至废纸篓 |
| `delete_secure` | — | `path`, `size`, `rounds` | 安全覆写删除 |

每个 span 结束时自动记录 `duration_ms`。

## 5. 输出目标

### 5.1 终端 (stderr)

- CLI：彩色人类可读格式，等级 `>= INFO`，可被 `--verbose` 降级到 `DEBUG`
- TUI：不直接输出日志（TUI 占用终端），日志写入文件
- Daemon：不输出到终端，仅写入文件

### 5.2 文件

- 路径：`~/.config/argus/logs/argus.log`
- 格式：JSON Lines（每行一个 JSON 对象）
- 轮转策略：
  - 最大 10MB / 文件
  - 保留最近 5 个文件（`.0` — `.4`）
  - 压缩旧文件（`.gz`）
- 权限：`600`（仅当前用户可读）

### 5.3 审计日志（单独文件）

参见 `07-safety.md` §5。独立于 tracing 系统，专用于记录删除操作。

- 路径：`~/.config/argus/audit.log`
- 格式：CSV（便于 `cut` / `awk` 解析）
- 字段：`timestamp`, `user`, `path`, `size`, `method`(trash/secure), `success`

## 6. 配置

在 `config.toml` 的 `[logging]` 组：

```toml
[logging]
# 终端日志等级（CLI 模式）：trace / debug / info / warn / error
level = "info"

# 文件日志等级：比终端更详细
file_level = "debug"

# 日志文件最大体积（支持 KB / MB / GB）
max_file_size = "10MB"

# 保留的轮转文件数
max_files = 5

# 日志格式：auto / json / text
# auto = CLI 模式用 text，daemon 模式用 json
format = "auto"
```

## 7. 各模块日志契约

### 7.1 argus-core

| 事件 | 等级 | 关键字段 |
|------|------|----------|
| 扫描启动 | `INFO` | `path` |
| 扫描完成 | `INFO` | `file_count`, `total_size`, `duration_ms` |
| 扫描取消 | `INFO` | `file_count` (已处理数) |
| 权限不足跳过 | `WARN` | `path`, `error` |
| 硬链接去重跳过 | `DEBUG` | `path`, `inode`, `device` |
| 快照加载成功 | `INFO` | `path`, `version`, `root_path` |
| 快照加载失败 | `ERROR` | `path`, `error` |
| Diff 计算完成 | `INFO` | `changed_count`, `total_delta`, `threshold` |
| AI Context 提取 | `DEBUG` | `path`, `top_files_count`, `extensions_count` |
| 批量 Prompt 组装 | `DEBUG` | `batch_size`, `estimated_tokens` |
| API 调用成功 | `INFO` | `batch_size`, `prompt_tokens`, `completion_tokens` |
| API 调用失败 | `WARN` | `batch_size`, `error`, `retry_count` |

### 7.2 argus-cli

| 事件 | 等级 | 关键字段 |
|------|------|----------|
| 命令接收 | `DEBUG` | `command`, `args` |
| 命令完成 | `INFO` | `command`, `duration_ms` |
| 退出码返回 | `DEBUG` | `exit_code` |

### 7.3 argusd（Phase 3+）

| 事件 | 等级 | 关键字段 |
|------|------|----------|
| Daemon 启动 | `INFO` | `version`, `watch_dirs` |
| UDS 监听开始 | `INFO` | `socket_path` |
| 客户端连接/断开 | `INFO` | `client_pid` |
| 文件事件收到 | `TRACE` | `event_type`, `path` |
| 去抖合并 | `DEBUG` | `pending_events`, `debounce_ms` |
| 快照归档 | `INFO` | `retention_hourly`, `retention_daily` |

### 7.4 argus-tui（Phase 2+）

| 事件 | 等级 | 关键字段 |
|------|------|----------|
| 光标移动 | `TRACE` | `path` |
| 排序切换 | `DEBUG` | `sort_by` |
| AI 诊断触发 | `INFO` | `path` (手动 or 自动) |
| 删除流程开始 | `INFO` | `path`, `risk_level` |
| 删除流程取消 | `INFO` | `path`, `reason` |
| 删除流程完成 | `INFO` | `path`, `method`, `success` |

## 8. 初始化代码（Phase 1 模板）

```rust
use tracing_subscriber::{
    fmt,
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

pub fn init_logging(config: &LoggingConfig) -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("argus/logs");
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::Builder::new()
        .max_log_files(config.max_files)
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("argus")
        .build(log_dir)?;

    let stdout_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(true);

    let json_layer = fmt::layer()
        .json()
        .with_writer(file_appender)
        .with_target(true)
        .with_current_span(true)
        .with_span_list(true);

    let filter = EnvFilter::builder()
        .with_default_directive(config.level.parse()?)
        .from_env()?
        .add_directive("hyper=warn".parse()?)
        .add_directive("reqwest=warn".parse()?);

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(json_layer)
        .init();

    Ok(())
}
```

## 9. AI Agent 调试指南

### 9.1 快速定位故障

```bash
# 查看最近错误
tail -n 100 ~/.config/argus/logs/argus.log | jq 'select(.level == "ERROR")'

# 查看特定目录的扫描记录
jq 'select(.fields.path // "" | startswith("/home/user/Downloads"))' ~/.config/argus/logs/argus.log

# 追踪一次扫描操作（按 span id）
jq 'select(.span.id == 42 or .span.parent == 42)' ~/.config/argus/logs/argus.log

# 查看慢操作（> 5s 的 span）
jq 'select(.fields.duration_ms // 0 > 5000)' ~/.config/argus/logs/argus.log

# 查看删除操作审计日志
cat ~/.config/argus/audit.log
```

### 9.2 开发期快捷命令

```bash
# CLI 模式开启 DEBUG 日志
RUST_LOG=argus_core=debug argus scan --path ~/Downloads --output snap.json

# 只追踪 scanner 模块
RUST_LOG=argus_core::scanner=trace argus scan --path ~/Downloads --output snap.json

# JSON 日志直接交给 AI Agent 分析
argus scan --path ~/Downloads --output snap.json 2>/dev/null
cat ~/.config/argus/logs/argus.log | jq '...'
```

### 9.3 日志不包含的内容（如需调试自行添加）

- 扫描出的完整 FileTree（改为输出根路径 + 文件数 + 总大小）
- AI 对话原始内容（改为输出 token 数 + 风险等级摘要）
- 网络请求 Raw Payload（`async-openai` 层应在 `TRACE` 记录 header 摘要但不记 body）
