# 系统架构设计

## 1. 整体三层架构

项目采用**核心库 + 多端适配 + 守护进程**的解耦架构：

```
+---------------------------------------------------------------+
|                    表现层 (Clients)                            |
|  +-------------------+  +------------------+  +-------------+  |
|  |    argus-cli      |  |    argus-tui     |  |  argus-gui  |  |
|  | (集成测试/自动化)  |  | (Vim-like TUI)   |  | (Slint/Tauri)| |
|  +---------+---------+  +--------+---------+  +------+------+  |
+-----------|---------------------|-------------------|---------+
            |                     |                   |
            +----------+----------+-------------------+
                       | (IPC: UDS / Named Pipes)
                       v
+---------------------------------------------------------------+
|                      服务层 (Service Layer)                    |
|  +---------------------------------------------------------+  |
|  |                      argusd                             |  |
|  |  后台守护进程: 事件循环 / FSEvents / Inotify / 去抖引擎   |  |
|  +----------------------------+----------------------------+  |
+-------------------------------|-------------------------------+
                               | (库调用 / 链接)
                               v
+---------------------------------------------------------------+
|                      核心引擎层 (Core Engine)                  |
|  +---------------------------------------------------------+  |
|  |                    argus-core                            |  |
|  |  - FileTree & Diff 算法 (核心数据结构)                    |  |
|  |  - 快照序列化 (JSON / 二进制)                             |  |
|  |  - AI 特征提取器 (结构化 Prompt 组装)                    |  |
|  |  - 多线程并行扫描器 (基于 ignore 库)                      |  |
|  +---------------------------------------------------------+  |
+---------------------------------------------------------------+
```

## 2. Cargo Workspace 组织

项目以 Monorepo 管理，目录结构如下：

```
argus/
├── Cargo.toml              # Workspace 根配置
├── argus-core/             # 纯逻辑库 (lib)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── model.rs        # 核心数据结构
│       ├── scanner.rs      # 文件扫描引擎
│       ├── diff.rs         # 时间差分算法
│       └── ai_feature.rs   # AI 特征提取
├── argusd/                 # 守护进程 (bin)
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
├── argus-cli/              # 命令行客户端 (bin)
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
├── argus-tui/              # TUI 客户端 (bin)
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
└── argus-gui/              # GUI 客户端 (bin - 后期)
    ├── Cargo.toml
    └── src/
        └── main.rs
```

## 3. 客户端通信模式

### 3.1 双模驱动

| 模式 | 适用场景 | 实现原理 |
|------|---------|---------|
| **独立模式 (Standalone)** | CLI 自动化测试、一次性扫描；TUI 默认启动模式 | Clients 直接调用 `argus-core`，数据写入本地快照文件 (`~/.config/argus/snapshots/`) |
| **服务模式 (Client-Server)** | TUI/GUI 需要秒级历史 Diff | 通过 Unix Domain Socket (UDS) 与 `argusd` 通信。Windows 使用 Named Pipes |

**独立模式下的文件树**：TUI 始终展示一个统一的文件树，delta 是树上的可选覆盖层（受顶部筛选栏控制）。详见 `03-core-features.md §5.0`。

| 筛选栏状态 | TUI 行为 | 等效工具 |
|-----------|---------|---------|
| 无时间/阈值（默认） | 纯文件树浏览 | `ncdu` |
| 选择了时间范围 | 加载对应快照，显示叠加 delta | `ncdu` + delta 增强 |

所有行为均通过调用 `argus-core` 实现，不依赖外部服务。

### 3.2 IPC 通信协议

Phase 3 守护进程通信使用基于 `serde` 的 RPC 消息体；传输编码优先采用 `bincode`，与 Phase 1 快照 JSON 格式相互独立：

```rust
enum ArgusRequest {
    GetDiff { from_timestamp: u64, to_timestamp: u64, threshold_bytes: u64 },
    GetAIContext { path: PathBuf },
    TriggerDelete { path: PathBuf, secure: bool },
}
```

## 4. 技术栈选型

| 组件 | 技术 | 选型理由 |
|------|------|---------|
| 核心扫描 | `ignore` (ripgrep 同款) | 多线程高性能，自动尊重 .gitignore |
| 文件监控 | `notify` | 跨平台文件变动通知 (inotify/FSEvents) |
| 快照序列化 | `serde` + `serde_json` | Phase 1 快照格式，便于 Debug |
| IPC 编码 (Phase 3) | `serde` + `bincode` | 守护进程 RPC 传输编码，低开销；不影响快照 JSON 格式 |
| 日志 | `tracing` + `tracing-subscriber` | 结构化 JSON + 终端彩色，支持 span 链路追踪 |
| TUI 界面 | `ratatui` + `crossterm` | tui-rs 正统续作，事件驱动组件化 |
| 异步运行时 | `tokio` | 全异步操作，保证 TUI 流畅 |
| AI 客户端 | `async-openai` | 兼容 OpenAI 格式的本地/云端模型 |
| 守护进程通信 | Unix Domain Socket | 低延迟、安全、跨平台 |
| 错误类型 | `thiserror` | Rust 标准 error 派生宏 |
| 快照 hash | `sha2` | 生成 root_path_hash 用于快照文件命名（前 8 字符） |
| 数据持久化 (后期) | SQLite / sled | 轻量嵌入式数据库 |
