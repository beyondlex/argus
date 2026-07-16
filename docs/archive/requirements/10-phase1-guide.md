# Phase 1 实施指导文档 (Implementation Guide)

## 0. Phase 1 实现契约

AI Agent 实施 Phase 1 时以本文件为入口，但遇到跨文档重复定义时遵循以下权威来源：

| 主题 | 权威文档 |
|------|---------|
| 数据结构与错误类型 | `08-data-model.md` |
| CLI 命令、参数、退出码 | `05-ux-interaction.md` 与本文件 §3 |
| 安全删除与系统黑名单 | `07-safety.md` |
| 配置文件字段 | `04-configuration.md` |
| 日志字段与输出 | `11-logging.md`（Phase 1 可暂不实现完整日志系统） |

Phase 1 只交付 `argus-core` 与 `argus-cli`。守护进程、TUI、真实 AI API 调用、删除操作、配置修改命令、审计查看命令均不属于 Phase 1。

## 1. 环境准备与 Workspace 初始化

### 1.1 创建目录结构

```bash
mkdir argus && cd argus
touch Cargo.toml
cargo init --lib argus-core
cargo init argus-cli
```

### 1.2 根目录 Cargo.toml

```toml
[workspace]
members = [
    "argus-core",
    "argus-cli"
]
resolver = "2"
```

## 2. argus-core 核心库实现

### 2.1 Cargo.toml

```toml
[package]
name = "argus-core"
version = "0.1.0"
edition = "2021"

[dependencies]
ignore = "=0.4.23"
serde = { version = "=1.0.217", features = ["derive"] }
serde_json = "=1.0.138"
toml = "=0.8.20"
chrono = { version = "=0.4.40", features = ["serde"] }
thiserror = "=2.0"
sha2 = "=0.10"
# indexmap = "=2.7"  # FUTURE: TUI 阶段替代 HashMap 保持插入序
```

### 2.2 模块结构

```
argus-core/src/
├── lib.rs
├── model.rs         # FileNode, Snapshot
├── scanner.rs       # Scanner 扫描引擎
└── db.rs            # 未来 daemon 数据库基础设施（预留）
```

### 2.3 核心模块说明

#### model.rs
定义 `FileNode`、`Snapshot` 核心结构体，全部派生 `Serialize/Deserialize`。

#### scanner.rs
使用 `ignore::WalkBuilder` 实现扫描。Phase 1 优先选择同步 `Walk`，保证行为简单可测；`WalkParallel` 留到大目录性能优化时引入。
1. 收集扁平化文件条目。扫描时维护 `HashSet<(u64, u64)>` 记录已见过的 `(device, inode)`。重复的硬链接**跳过**，不累加 size（参见 `08-data-model.md` §2.1）。
2. 按路径深度排序，自底向上构建树。
3. 自动跳过 `.gitignore` 匹配的路径。
4. **取消机制**：使用 `AtomicBool` 共享取消标志。在 `Walk::filter_entry` 回调中定期检查（每 1000 个文件）。扫描函数签名 `scan(path, cancel: &AtomicBool) -> Result<Snapshot, ScanError>`。收到取消信号时返回 `Err(ScanError::Cancelled)`，不返回部分快照。
5. **进度感知**：扫描 30 秒后自动启用进度指示器。每扫描 10,000 个文件通过 `mpsc::Sender` 推送一次进度更新 `(file_count, total_bytes)`，上层 CLI/TUI 可选择监听渲染。

#### db.rs
预留数据库基础设施，供未来 daemon 使用。当前仅提供 `default_db_path()` 和 `open_db()`。

### 2.4 补充模块

#### config.rs（Phase 1 简易版）

Phase 1 仅加载配置文件中的 `[ignore]` 组规则，用于扫描参数默认值。配置文件路径：`~/.config/argus/config.toml`。文件不存在时不报错，使用全默认值。

```rust
#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub ignore: IgnoreConfig,
}

#[derive(Deserialize, Default)]
pub struct IgnoreConfig {
    pub ignore_hidden: Option<bool>,
    pub follow_symlinks: Option<bool>,
    pub custom_ignore_paths: Option<Vec<String>>,
}

impl Config {
    /// 加载配置文件，不存在时返回 Default
    pub fn load() -> Self;
}
```

> 完整配置系统（AI/快捷键/主题等）留待 Phase 2+ 实现。

## 3. argus-cli 验证端实现

### 3.1 Cargo.toml

```toml
[package]
name = "argus-cli"
version = "0.1.0"
edition = "2021"

[dependencies]
argus-core = { path = "../argus-core" }
clap = { version = "4.4", features = ["derive"] }
serde_json = "1.0"
human_bytes = "0.4"
anyhow = "1.0"
```

### 3.2 CLI 命令设计

| 命令 | 参数 | 功能 |
|------|------|------|
| `scan` | `--path <PATH>` | 扫描目录并打印摘要（纯内存） |

**阈值解析说明**（`argus-core` 提供工具函数）：
```rust
/// 解析 "50MB" → 52_428_800, "2.5GB" → 2_684_354_560
/// 支持单位: B, KB, MB, GB, TB (二进制前缀)
fn parse_human_size(input: &str) -> Result<u64, ParseSizeError>;
```

### 3.3 main.rs 结构

```rust
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "argus")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Scan { path: PathBuf },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Scan { path } => {
            // 调用 Scanner::scan(path, &AtomicBool::new(false))
            // 打印摘要
        }
    }
    Ok(())
}
```

**退出码契约**（在 `main()` 中通过 `std::process::exit(code)` 实现）：

| 退出码 | 含义 |
|--------|------|
| `0` | 成功 |
| `2` | 参数错误（clap 自动处理非法参数） |
| `3` | IO 错误（文件不存在、权限不足） |
| `4` | 内部错误（快照损坏、反序列化失败） |

## 4. 验收标准 (Definition of Done)

### 4.1 测试全盘扫描

```bash
cargo run -p argus-cli -- scan --path ~/Downloads
```

**验证点**：终端应打印扫描路径、文件总数、总大小。

### 4.6 单元测试要求

- 优先编写 `scanner` 和 `db` 层的单元测试（`#[cfg(test)]`）。
- 使用 mock 的简易 `FileNode` 树进行断言。
- 通过单元测试可发现 80% 的算法 Debug 问题。

### 4.7 测试数据构造辅助

使用 `file_tree!` 宏（或等价 builder 模式）快速构造测试树，避免手动嵌套 `FileNode`。

> **注意**：`FileNode.name` 只存文件名最后一级（如 `Documents`），不存完整路径。宏使用**路径数组**语法，从根节点名称开始逐级嵌套。以下示例构造 `home > user > Documents` 三层：

```rust
// 期望的测试写法（macro 或 builder）
// 语法：路径从根节点名开始，逐级嵌套，最后为 size
let tree = file_tree! {
    // 第 1 行：根节点 "home"（目录），size=1000
    // 第 2 行：home 下的子目录 "user"（目录），size=800
    // 第 3 行：user 下的子目录 "Documents"（目录），size=500
    // 第 4 行：user 下的下载目录中的文件
    "home" => 1000,
    "home/user" => 800,
    "home/user/Documents" => 500,
    "home/user/Downloads/big_file.iso" => 300,
};

// builder/macro 负责补齐 FileNode 的 file_type、modified、inode、device、children 等字段。
```

**宏实现概要**：宏接收 `"path/to/node" => size` 条目列表。对每个条目，按 `/` 分割路径，逐级在树中 upsert。叶子节点 size 使用给定值，中间节点 size 为子节点累加和（由自底向上构建逻辑保证）。

**测试覆盖场景清单**：

| 场景 | 输入 | 预期 |
|------|------|------|
| 空目录 | 空目录路径 | `Snapshot` 含根节点，无子节点 |
| 单文件 | 含一个文件的目录 | `Snapshot` 含一个文件节点 |
| 嵌套目录 | 3 级目录树，含多个文件 | 正确的树结构，汇总 size 等于子节点之和 |
| 硬链接去重 | 同一 inode 出现两次 | size 只计一次 |
| 取消扫描 | 触发 `AtomicBool` true | 返回 `Err(ScanError::Cancelled)` |
| 扫描摘要 | 扫描后校验文件和总数 | 与 `count_files` 一致 |

## 5. 开发顺序建议

```
第 1 步：model.rs — 数据结构定义
第 2 步：scanner.rs — 扫描引擎（含单元测试）
第 3 步：argus-cli/main.rs — CLI 命令解析与调用
第 4 步：手动验收测试（按 4.1 执行）
```
