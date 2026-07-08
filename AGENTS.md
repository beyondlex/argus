# AGENTS.md — Argus 项目开发指南

本文件指导 AI Agent 如何高效、高质量地参与 Argus 项目的开发。请每次开始工作前完整阅读。

## 1. 项目速览

Argus = 个人桌面级磁盘智能治理工具。Rust 实现，Cargo Workspace 组织。

```
argus-core/  核心逻辑库 (扫描、Diff、AI 特征提取)
argus-cli/   命令行客户端 (快速验证与自动化测试)
argusd/      守护进程 (后台监控)
argus-tui/   TUI 客户端 (ratatui)
argus-gui/   GUI 客户端 (后期)
```

完整需求文档见 `docs/requirements/index.md`。

## 2. 核心架构原则

### 2.1 分层解耦

- **argus-core** 不得依赖任何客户端代码或 UI 库。它是纯逻辑库，可被任意客户端复用。
- **argusd** 作为独立进程通过 UDS 与客户端通信，不嵌入任何客户端。
- 客户端 (CLI/TUI/GUI) 之间不共享代码 —— 它们各自依赖 argus-core。

### 2.2 双模驱动

所有客户端都支持两种模式：
- **Standalone**：直接调用 argus-core，读写本地快照文件，无需 daemon。
- **Client-Server**：通过 UDS 连接 argusd 获取实时增量数据。

设计时优先保证 Standalone 模式可用，Server 模式为增强体验的进阶选项。

### 2.3 AI 是插件，非核心

- AI 功能默认关闭。所有核心功能（扫描、Diff、浏览、删除）在无 AI 配置时须完整可用。
- AI 相关代码通过 feature flag 隔离，不影响核心编译体积。

## 3. 开发纪律

### 3.1 TDD 优先

- **先写测试，再写实现**。核心算法（Diff、Tree Merge、特征提取）必须伴随单元测试。
- 单元测试覆盖所有边界情况：空目录、单文件、深层嵌套、对称/非对称 Diff。
- CLI 命令通过集成测试验证（`cargo test --test integration`）。
- 测试使用 mock 数据而非真实文件系统（除集成测试外）。

### 3.2 文档与代码同步（双向链）

**向前同步**（需求文档 ← 代码）：修改实现后立即更新对应的需求文档，确保需求文档始终反映实际行为。

| 代码变动 | 需同步的需求文档 |
|---------|----------------|
| 修改数据结构 | `docs/requirements/08-data-model.md` |
| 新增/修改 CLI 命令 | `docs/requirements/05-ux-interaction.md` |
| 修改配置项 | `docs/requirements/04-configuration.md` |
| 添加依赖库 | 在 PR 描述中说明选型理由 |

**向后同步**（代码 → 用户/开发者文档）：新增或变更功能后，同步更新面向用户的文档和面向开发者的文档。

| 代码变动 | 需同步的文档 |
|---------|-------------|
| 新增 CLI 命令或参数 | `README.md` 中的使用示例、`--help` 输出 |
| 修改配置项 | `README.md` 中的配置说明、配置示例 |
| 新增环境依赖或运行时要求 | `README.md` 中的安装/前置要求部分 |
| 修改构建流程或测试命令 | `CONTRIBUTING.md`（若有）或项目根 `README.md` |
| 新增模块或重构公共 API | 模块级 doc comment（`///`），在对应 `lib.rs` 中补充模块文档 |

- 文档不是一次性工作。每次提交时审视：**这个改动是否需要更新某份文档？**
- 如果当前还没有对应的文档文件（如 `CONTRIBUTING.md`），至少更新 `README.md`，后续再拆分。

### 3.3 渐进式架构

- 不为未来过度设计。当前 Phase 的代码只解决当前 Phase 的问题。
- 若某个设计决策会影响未来 Phase（如数据结构需兼容 daemon IPC），加 `// FUTURE:` 注释标记。
- 公共 API 设计时预留扩展点（如使用 `enum` 而非 `bool` 参数），但不要提前实现未使用的抽象。

## 4. 代码规范

### 4.1 Rust 风格

- 使用 `cargo fmt` 和 `cargo clippy`（必须通过，0 warnings）。
- 遵循 Rust API Guidelines (https://rust-lang.github.io/api-guidelines/)。
- 公有 API 需有 doc comment。内部函数在复杂度高时加注释说明"为什么这么做"。
- 错误类型使用 `thiserror` 定义，避免 `unwrap()`/`expect()`（仅在测试和不可达路径中使用）。
- 异步使用 `tokio`，同步代码不使用 async。

### 4.2 命名约定

- 类型：PascalCase（`FileNode`, `Snapshot`）
- 函数/方法：snake_case（`compare_trees`, `scan_path`）
- 模块：snake_case，短名称（`model`, `scanner`, `diff`）
- 错误类型：以 `Error` 结尾（`ScanError`, `DiffError`）
- 特征（trait）：以动词命名（`Scanner`, `Differ`），而非 `-able` 后缀

### 4.3 文件组织

- 每个模块一个文件。如果一个模块超过 500 行，拆分子模块。
- 测试放在模块末尾的 `#[cfg(test)] mod tests { ... }` 块中，而非独立测试文件。（集成测试除外，它放在 `tests/` 目录。）
- 公开类型在 `lib.rs` 中 re-export，外部代码不直接引用深层路径。

### 4.4 可维护性

- 函数不超过 50 行。超过则拆分为辅助函数。
- 避免深层嵌套（>3 层）。用早期 return 或组合子模式简化控制流。
- 核心算法（如 `compare_trees`）必须有 ASCII 图或伪代码注释说明逻辑。
- 每次提交后运行 `cargo test && cargo clippy && cargo fmt --check`。

## 5. 测试策略

| 层级 | 工具 | 覆盖内容 |
|------|------|---------|
| 单元测试 | `#[cfg(test)]` | 每个核心函数，mock 数据 |
| 集成测试 | `tests/` | CLI 命令端到端 |
| 快照测试 | `insta` (可选) | Diff 输出格式 |
| 性能测试 | `criterion` (可选) | 扫描大目录场景 |

测试命名：`test_<被测函数>_<场景>`（如 `test_compare_trees_both_empty`）。

## 6. AI 开发原则

### 6.1 代码生成

- 优先阅读并复用项目中已有的代码模式，而非重新发明。
- 同一次对话中连续修改同一文件时，使用 `read + edit` 而非反复 `write`。
- 新增依赖包前检查 `Cargo.toml` 是否存在同类替代。
- 不要添加未被需求的抽象（如"为 future 准备的 factory pattern"）。

### 6.2 实现顺序

对于 Phase 1 的实施，严格按此顺序进行：

```
model.rs  →  scanner.rs（含测试） →  diff.rs（含测试）
  →  ai_feature.rs（含测试） →  cli/main.rs →  集成测试
```

每完成一个模块运行 `cargo test` 确认无误再进入下一个。

### 6.3 遇阻处理

- 如果某个接口设计不确定，查阅 `docs/requirements/` 中对应的规范文档。
- 如果遇到 Rust 编译器错误无法解决，先尝试简化代码（减少泛型、临时使用具体类型）。
- 如果某个库的功能不符合预期，查阅其官方文档而非猜测 API 行为。

## 7. 关键约束

- **禁止**直接调用 `std::process::Command` 执行 shell 命令（如 `rm -rf`）。文件操作必须通过 Rust 标准库或系统 API。
- **禁止**在 argus-core 中引入任何 GUI/TUI 依赖。
- **禁止**在 `Cargo.toml` 中使用 `*` 版本号。
- **禁止**提交包含 API Key、Token 或任何凭据的代码（使用环境变量或配置文件）。
- **必须**处理所有 `Result` 和 `Option` —— 不允许 `unwrap()`（测试除外）。
- **必须**在 `Cargo.toml` 中显式启用要用到的 feature，不依赖 transitive features。

## 8. 与架构师的协作

- 实现前先输出简要的技术方案（选择哪种设计、为什么），获得确认再写代码。
- 核心算法和数据结构的改动需要 review。
- 如果发现需求文档中的设计在实际实现中不合理，提出替代方案而非强行适配。
