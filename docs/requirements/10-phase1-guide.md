# Phase 1 实施指导文档 (Implementation Guide)

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
ignore = "0.4"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
walkdir = "2.4"
rayon = "1.8"
chrono = { version = "0.4", features = ["serde"] }
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
使用 `ignore::WalkBuilder` 实现多线程扫描：
1. 收集扁平化文件条目。
2. 按路径深度排序，自底向上构建树。
3. 自动跳过 `.gitignore` 匹配的路径。

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
```

### 3.2 CLI 命令设计

| 命令 | 参数 | 功能 |
|------|------|------|
| `scan` | `--path <PATH> --output <FILE>` | 扫描目录并保存快照 |
| `diff` | `--old <FILE> --new <FILE> [--threshold-bytes <N>]` | 对比两个快照 |
| `explain` | `--old <FILE> --new <FILE> --target-path <PATH>` | 模拟 AI 诊断 |

### 3.3 main.rs 结构

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "argus")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Scan { path: PathBuf, output: PathBuf },
    Diff { old: PathBuf, new: PathBuf, threshold_bytes: u64 },
    Explain { old: PathBuf, new: PathBuf, target_path: PathBuf },
}

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Scan { path, output } => { /* 调用 Scanner 并写入文件 */ },
        Commands::Diff { old, new, threshold_bytes } => { /* 读取、计算并打印 */ },
        Commands::Explain { old, new, target_path } => { /* 计算 Diff 树，提取特征并 println!(Prompt) */ },
    }
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
cargo run -p argus-cli -- diff --old ./snap_old.json --new ./snap_new.json --threshold-bytes 5242880
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

## 5. 开发顺序建议

```
第 1 步：model.rs — 数据结构定义
第 2 步：scanner.rs — 扫描引擎（含单元测试）
第 3 步：diff.rs — Diff 算法（含单元测试）
第 4 步：ai_feature.rs — 特征提取 + Prompt 生成
第 5 步：argus-cli/main.rs — CLI 命令解析与调用
第 6 步：手动验收测试（按 4.1-4.5 执行）
```
