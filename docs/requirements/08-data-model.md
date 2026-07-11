# 数据模型与核心算法

## 1. 核心数据结构

### 1.1 FileNode（文件/目录节点）

文件树的最小单元，支持序列化。

```rust
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileNode {
    pub name: String,
    pub is_dir: bool,
    pub file_type: FileType,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inode: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<u64>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub has_metadata: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub children: HashMap<String, FileNode>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Fifo,
    Socket,
    Device,
    Other,
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | `String` | 文件/目录名（不含路径） |
| `is_dir` | `bool` | 是否为目录 |
| `file_type` | `FileType` | 文件类型。Phase 1 用于标记符号链接、FIFO、socket、设备文件等特殊文件；目录节点必须为 `Directory`，普通文件为 `File` |
| `size` | `u64` | 当前时间点的总大小（字节） |
| `modified` | `Option<DateTime<Utc>>` | 最后修改时间（部分文件系统不可用，故为 Option） |
| `inode` | `Option<u64>` | 文件 inode 号，用于硬链接去重（macOS/Linux）。扫描器内部维护 `HashSet<(device, inode)>`，已见过的 inode 不再累加 size。非序列化关键字段，`skip_serializing_if` 减少快照体积 |
| `device` | `Option<u64>` | 文件所在设备 ID，与 inode 组合唯一标识一个文件 |
| `has_metadata` | `bool` | 该节点是否持有可展示的扫描元数据。`true` 表示普通文件/目录节点可显示 size；`false` 仅用于结构占位节点（例如浅扫目录的深层子孙），UI 应显示 `"..."` |
| `children` | `HashMap<String, FileNode>` | 子节点（目录专用）。**Phase 1 使用 `HashMap`，输出时临时排序**。`// FUTURE: 迁移至 IndexMap 以保持插入序，或 BTreeMap 保持字典序` |

### 1.2 Snapshot（快照）

单次扫描的完整持久化结构。

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Snapshot {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub root_path: PathBuf,
    pub root_path_hash: String,
    pub total_size: u64,
    pub root_node: FileNode,
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `root_path_hash` | `String` | `sha256(root_path)` 前 8 字符。用于快照文件命名、快速校验路径一致性，防止加载错误路径的快照。写入时由 `Snapshot::new()` 自动计算 |

> **2026-07 架构变更**：`DiffNode` 已被移除。Phase 2（独立模式）无 delta 功能。Delta 将在 Phase 3（Daemon 模式）中通过增量事件聚合实现，数据结构届时重新设计。

## 2. 核心算法

### 2.1 自底向上体积构建

扫描过程：
1. 使用 `WalkBuilder` 递归遍历目录。
2. 收集扁平化 `(PathBuf, Metadata)` 列表。
3. 扫描时维护 `HashSet<(u64, u64)>` 记录已见过的 `(device, inode)`。若当前文件的 `(device, inode)` 已存在，则**跳过该文件**（不累加 size，不创建 FileNode），仅记录日志。此机制防止同一物理文件被多路径引用时重复统计。
4. 按路径深度从深到浅排序。
5. 将子节点 size 累加到父节点。

> **2026-07 架构变更**：AI 特征提取（`AiContext`、`AiResult`）随 Snapshot Diff 一同移除，将在 Phase 4（Daemon 模式 + AI）中重新设计。

### 2.4 惰性目录列举（list_dir）

独立模式下，TUI 通过 `list_dir` 实现惰性文件树导航，避免全量扫描前无数据可用。

```rust
/// 惰性读取目录一级内容。返回文件/目录的 FileNode，其中：
/// - 文件：size = metadata().len()（真实文件大小）
/// - 目录：size = 0（未递归求和）
/// - 目录节点的 has_metadata = true（普通未扫描目录）
/// - 子目录的 children = 空（惰性加载，展开时再读取）
///
/// 错误：PathNotFound, PermissionDenied, Io
pub fn list_dir(path: &Path) -> Result<FileNode, ScanError>
```

`list_dir` 与递归 `scan_path` 的关系：

| 维度 | list_dir | scan_path |
|------|----------|-----------|
| 递归 | 否，只读一级 | 是，全量递归 |
| 目录 size | 0（不求和） | 自底向上汇总 |
| children | 仅文件有，子目录为空 | 完整子树 |
| 用途 | TUI 惰性导航 | 保存快照、计算 diff |
| 性能 | O(n) 一级条目 | O(N) 全量遍历 |

### 2.5 批量分析与反馈映射

当用户在 TUI 中触发 AI 分析（按 `a` 键或光标停顿），系统可发送一批路径给 AI 以提高效率。

**映射策略（三阶降级）**：

| 优先级 | 策略 | 适用模型 | 原理 |
|--------|------|---------|------|
| 1 | **JSON 模式** | 支持 response_format=json_object 的模型 | 要求 AI 返回 `{"path1": AiResult, "path2": AiResult, ...}`，解析后以 path 为键存入 AiCache |
| 2 | **编号索引** | 任意文本模型 | 在 Prompt 中为每项加 `[N]` 前缀，要求 AI 以 `[N]` 开头回复。使用正则 `\[(\d+)\](.*)` 提取 |
| 3 | **逐条发送** | 兜底 | 每次只送一条路径，无映射问题。最慢但最可靠 |

**伪代码**：

```rust
/// 批量分析：输入一批 AiContext，输出 path → AiResult 的映射
pub async fn batch_analyze(
    contexts: Vec<AiContext>,
    client: &OpenAIClient,
    strategy: MappingStrategy,
) -> HashMap<PathBuf, AiResult> {
    // 1. 构造批量 Prompt（见 06-ai-design.md 模板）
    // 2. 按策略发送请求
    // 3. 解析响应 → HashMap<PathBuf, AiResult>
    // 4. 写入 AiCache 返回
}

fn parse_json_response(raw: &str) -> Result<HashMap<PathBuf, AiResult>>;
fn parse_indexed_response(raw: &str) -> Result<HashMap<PathBuf, AiResult>>;
```

**AiCache 生命周期**：
- 存储在客户端内存中（TUI App State 或 CLI 单次执行上下文），与 Diff 树同级
- 用户在同一会话中重复查看同一目录 → 直接命中缓存，不重复请求
- 用户关闭客户端 → 缓存释放（AI 结果不持久化，因 LLM 版本迭代后旧结论可能不准）

## 3. 扫描历史持久化

### 3.1 SQLite 主存储

- 单次扫描结果写入 SQLite 数据库 `~/.config/argus/argus.db`。
- 数据库中保存扫描事件、路径记录、根路径哈希和必要的文件系统元数据。
- TUI 启动时从 SQLite 加载当前工作目录对应的最新扫描结果；若无扫描数据，则降级为 `list_dir` 的文件系统视图。
- 结构占位节点来自浅扫缓存，`has_metadata = false`，UI 用 `"..."` 表示。

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Snapshot {
    pub version: u32,
    pub timestamp: DateTime<Utc>,
    pub root_path: PathBuf,
    pub root_path_hash: String,
    pub total_size: u64,
    pub root_node: FileNode,
}
```

- `Snapshot` 仍然作为内存中的视图模型存在，用于 TUI/CLI 渲染和 diff 输入，但不再作为主持久化格式。
- `root_path_hash` 用于数据库分组和路径一致性校验。

### 3.2 后期导出

- 如果未来需要文件级导出或离线调试，可再考虑 JSON / binary export。
- 这些导出格式不属于当前主存储路径。

## 4. API 设计（面向 Daemon）

守护进程与客户端之间的 IPC 协议。Delta 查询按路径范围 + 时间范围进行：

```rust
enum ArgusRequest {
    GetDelta {
        path: PathBuf,           // 查询子树根路径（如 "/home/user/Downloads"）
        from_timestamp: u64,
        to_timestamp: u64,
        threshold_bytes: u64,    // 可选阈值过滤
    },
    GetAIContext {
        path: PathBuf,
    },
    TriggerDelete {
        path: PathBuf,
        secure: bool,
    },
    ListScans,
    GetConfig,
    SetConfig { key: String, value: String },
}

enum ArgusResponse {
    DeltaResult { root: DiffNode },
    AIContext { context: AiContext },
    DeleteResult { success: bool, path: PathBuf },
    ScanList { scans: Vec<ScanInfo> },
    ConfigData { content: String },
    Error { message: String },
}
```

## 5. 错误类型体系

所有公开 API 返回 `Result<T, XxxError>`，使用 `thiserror` 派生。错误类型分层如下：

> **依赖说明**：`thiserror` 必须加入 `argus-core/Cargo.toml`。详见 `10-phase1-guide.md` §2.1。

### 5.1 ScanError

```rust
#[derive(thiserror::Error, Debug)]
pub enum ScanError {
    #[error("路径不存在: {0}")]
    PathNotFound(PathBuf),

    #[error("权限不足: {0}")]
    PermissionDenied(PathBuf),

    #[error("扫描被用户取消")]
    Cancelled,

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}
```

**行为策略**：
- `PermissionDenied`：记录日志，跳过该文件/目录，继续扫描（不终止）。
- `Cancelled`：立即停止扫描并返回错误。Phase 1 不返回已构建的部分树，避免调用方误把不完整目录树写入快照。
- `PathNotFound`：终止扫描，返回错误。

### 5.2 DbError

```rust
#[derive(thiserror::Error, Debug)]
pub enum DbError {
    #[error("数据库错误: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("找不到扫描数据: {0}")]
    NoScanFound(String),

    #[error("时间戳解析错误: {0}")]
    TimestampParse(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}
```

**行为策略**：
- `NoScanFound`：提示用户当前根路径没有可用扫描记录。
- `TimestampParse`：提示数据库中的时间字段损坏或格式不合法。

### 5.3 DiffError

```rust
#[derive(thiserror::Error, Debug)]
pub enum DiffError {
    #[error("快照根路径不匹配: {0} vs {1}")]
    RootPathMismatch(PathBuf, PathBuf),

    #[error("内部错误: {0}")]
    Internal(String),
}
```

**行为策略**：
- `RootPathMismatch`：不允许对比不同根路径的快照（语义上无意义）。
- `Internal`：panic 等价，作为兜底。正常流程不应触发。

## 6. 数据库时序模型

SQLite 是扫描历史和时序查询的主存储层。Daemon 模式下仍使用同一套表结构和查询语义。

### 6.1 核心表

```sql
CREATE TABLE scan_events (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp      TEXT NOT NULL,
    root_path      TEXT NOT NULL,
    root_path_hash TEXT NOT NULL,
    total_size     INTEGER NOT NULL,
    total_files    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_scan_events_root_hash_ts
    ON scan_events(root_path_hash, timestamp);

CREATE TABLE path_records (
    scan_id     INTEGER NOT NULL REFERENCES scan_events(id),
    path        TEXT NOT NULL,
    size        INTEGER NOT NULL,
    is_dir      INTEGER NOT NULL DEFAULT 0,
    file_type   TEXT NOT NULL,
    modified    TEXT,
    inode       INTEGER,
    device      INTEGER,
    PRIMARY KEY (scan_id, path)
);

CREATE INDEX idx_path_records_path ON path_records(path);
```

### 6.2 查询模式

Time-Series Query 的核心差异在于：**从"加载两份完整树做 diff"变为"SQL 聚合任意子树在任意时间范围的 delta"**。

```sql
SELECT
    ap.path,
    COALESCE(sf.size, 0) AS size_from,
    COALESCE(st.size, 0) AS size_to,
    COALESCE(st.size, 0) - COALESCE(sf.size, 0) AS delta
FROM ...
```

### 6.3 与树渲染的关系

- `Snapshot` 用于表示当前视图树，由 SQLite materialize 得到。
