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
├── model.rs         # FileNode, Snapshot, DiffNode
├── scanner.rs       # Scanner 扫描引擎
├── diff.rs          # compare_trees Diff 算法
└── ai_feature.rs   # AiContext, extract_feature, generate_prompt
```

### 2.3 核心模块说明

#### model.rs
定义 `FileNode`、`Snapshot`、`DiffNode` 三个核心结构体，全部派生 `Serialize/Deserialize`（DiffNode 除外，它仅用于展示）。

#### scanner.rs
使用 `ignore::WalkBuilder` 实现扫描。Phase 1 优先选择同步 `Walk`，保证行为简单可测；`WalkParallel` 留到大目录性能优化时引入。
1. 收集扁平化文件条目。扫描时维护 `HashSet<(u64, u64)>` 记录已见过的 `(device, inode)`。重复的硬链接**跳过**，不累加 size（参见 `08-data-model.md` §2.1）。
2. 按路径深度排序，自底向上构建树。
3. 自动跳过 `.gitignore` 匹配的路径。
4. **取消机制**：使用 `AtomicBool` 共享取消标志。在 `Walk::filter_entry` 回调中定期检查（每 1000 个文件）。扫描函数签名 `scan(path, cancel: &AtomicBool) -> Result<Snapshot, ScanError>`。收到取消信号时返回 `Err(ScanError::Cancelled)`，不返回部分快照。
5. **进度感知**：扫描 30 秒后自动启用进度指示器。每扫描 10,000 个文件通过 `mpsc::Sender` 推送一次进度更新 `(file_count, total_bytes)`，上层 CLI/TUI 可选择监听渲染。

#### diff.rs
实现 `compare_trees` 递归函数：
1. 处理四种状态（都存在/仅A/仅B/都不存在）。
2. 目录节点递归合并子节点。
3. 自底向上聚合 size_delta。
4. 支持阈值过滤。

#### ai_feature.rs
实现 `extract_feature` 和 `generate_prompt`：
- 从 Diff 树中按路径提取子树。
- 统计 Top 5 大文件和后缀分布。
- 组装结构化 Prompt 文本。

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
| `scan` | `--path <PATH> --output <FILE>` | 扫描目录并保存快照 |
| `diff` | `--old <FILE> --new <FILE> [--threshold <SIZE>] [--format <FMT>]` | 对比两个快照。`--threshold` 支持人类可读格式（`50MB`, `2.5GB`），默认 `0`。`--format` 支持 `text`（默认）/ `json` / `markdown` |
| `explain` | `--old <FILE> --new <FILE> --target-path <PATH>` | 模拟 AI 诊断（仅打印 Prompt，不调用任何 API） |

**阈值解析说明**（`argus-core` 提供工具函数）：
```rust
/// 解析 "50MB" → 52_428_800, "2.5GB" → 2_684_354_560
/// 支持单位: B, KB, MB, GB, TB (二进制前缀)
fn parse_human_size(input: &str) -> Result<u64, ParseSizeError>;
```

### 3.3 main.rs 结构

```rust
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "argus")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(ValueEnum, Clone, Default)]
enum OutputFormat {
    #[default]
    Text,
    Json,
    Markdown,
}

#[derive(Subcommand)]
enum Commands {
    Scan { path: PathBuf, output: PathBuf },
    Diff {
        old: PathBuf,
        new: PathBuf,
        #[arg(long = "threshold", default_value = "0")]
        threshold: String,
        #[arg(long = "format", default_value = "text")]
        format: OutputFormat,
    },
    Explain { old: PathBuf, new: PathBuf, target_path: PathBuf },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Scan { path, output } => {
            // 调用 Scanner::scan(path, &AtomicBool::new(false))
            // 写入 Snapshot 到 output 文件
        }
        Commands::Diff { old, new, threshold, format } => {
            // 1. 读取两个 Snapshot 文件
            // 2. 调用 diff::compare_trees()
            // 3. 根据 format 打印结果
            // 4. 退出码：无超阈值变动 → 0，有 → 1
        }
        Commands::Explain { old, new, target_path } => {
            // 计算 Diff 树，提取特征并 println!(Prompt)
        }
    }
    Ok(())
}
```

**退出码契约**（在 `main()` 中通过 `std::process::exit(code)` 实现）：

| 退出码 | 含义 |
|--------|------|
| `0` | 成功，无超阈值变动（scan 完成 / diff 无显著差异） |
| `1` | 发现超阈值变动（diff 存在 `size_delta >= threshold` 的条目） |
| `2` | 参数错误（clap 自动处理非法参数） |
| `3` | IO 错误（快照文件不存在、权限不足） |
| `4` | 内部错误（快照损坏、反序列化失败） |

```rust
// Phase 1：简易实现，可直接在 main 中 match 退出
fn exit_with_code(result: &DiffResult, threshold: u64) -> i32 {
    if result.has_significant_changes(threshold) { 1 } else { 0 }
}
```

## 4. 验收标准 (Definition of Done)

### 4.1 测试全盘扫描与持久化

```bash
cargo run -p argus-cli -- scan --path ~/Downloads --output ./snap_old.json
```

**验证点**：检查 `./snap_old.json` 是否生成，内含合规的树状 JSON 数据。

### 4.2 测试空间变动制造

在 `~/Downloads` 中手动创建一个 50MB 临时文件，或向某个 log 追加大量文本。

### 4.3 测试第二次扫描

```bash
cargo run -p argus-cli -- scan --path ~/Downloads --output ./snap_new.json
```

### 4.4 测试时间差分输出

```bash
cargo run -p argus-cli -- diff --old ./snap_old.json --new ./snap_new.json --threshold 5MB
```

**验证点**：终端应清晰打印出制造的 50MB 变动文件及其父目录路径，变动量为正。

### 4.5 测试 AI Prompt 组装

```bash
cargo run -p argus-cli -- explain --old ./snap_old.json --new ./snap_new.json --target-path ~/Downloads
```

**验证点**：拷贝终端打印的完整 Prompt 贴给大模型，检查回答是否契合"打消用户不敢删的恐惧"这一核心诉求。

### 4.6 单元测试要求

- 优先编写 `compare_trees` 的单元测试（`#[cfg(test)]`）。
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
| 两空树 | `A = {}, B = {}` | `None` |
| 单文件新增 | `A = {}`, `B = {file: 100}` | `size_delta = +100` |
| 单文件删除 | `A = {file: 100}`, `B = {}` | `size_delta = -100` |
| 目录新增 | `A = {}`, `B = {dir: {file: 200}}` | `size_delta = +200` |
| 目录缩小 | `A = {dir: {f1: 100, f2: 200}}`, `B = {dir: {f1: 100}}` | `size_delta = -200` |
| 深层嵌套 | 3 级目录树，中间节点有变动 | 子节点 delta 累加到所有祖先 |
| 阈值过滤 | delta < 50 的节点 | 结果中不包含小变动节点 |

## 5. 开发顺序建议

```
第 1 步：model.rs — 数据结构定义
第 2 步：scanner.rs — 扫描引擎（含单元测试）
第 3 步：diff.rs — Diff 算法（含单元测试）
第 4 步：ai_feature.rs — 特征提取 + Prompt 生成
第 5 步：argus-cli/main.rs — CLI 命令解析与调用
第 6 步：手动验收测试（按 4.1-4.5 执行）
```
