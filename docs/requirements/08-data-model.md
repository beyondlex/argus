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

### 1.3 DiffNode（差分节点）

对比结果，非持久化，用于展示。

```rust
#[derive(Debug, Clone)]
pub struct DiffNode {
    pub name: String,
    pub is_dir: bool,
    pub current_size: u64,
    pub size_delta: i64,      // 增长为正，减少为负
    pub children: HashMap<String, DiffNode>,
}
```

## 2. 核心算法

### 2.1 自底向上体积构建

扫描过程：
1. 使用 `WalkBuilder` 递归遍历目录。
2. 收集扁平化 `(PathBuf, Metadata)` 列表。
3. 扫描时维护 `HashSet<(u64, u64)>` 记录已见过的 `(device, inode)`。若当前文件的 `(device, inode)` 已存在，则**跳过该文件**（不累加 size，不创建 FileNode），仅记录日志。此机制防止同一物理文件被多路径引用时重复统计。
4. 按路径深度从深到浅排序。
5. 将子节点 size 累加到父节点。

### 2.2 Tree Merge Diff 算法

```
输入：node_a（快照 A 的 FileNode）, node_b（快照 B 的 FileNode）
输出：DiffNode（合并后的差分树）

算法步骤：
1. 若 A 和 B 均不存在 → 返回 None
2. 若 A 存在而 B 不存在 → 文件被删除
   size_delta = -(A.size), current_size = 0
3. 若 B 存在而 A 不存在 → 文件新增
   size_delta = B.size, current_size = B.size
4. 若 A 和 B 均存在 → 对比变化
   size_delta = B.size - A.size, current_size = B.size
5. 如果是目录，递归合并子节点（Union Keys）
6. 过滤掉 size_delta == 0 且 current_size == 0 的未变动节点
7. 自底向上：子节点 size_delta 累加到父节点
```

### 2.3 AI 特征提取 & 结果映射

```rust
/// 发送给 AI 的单条目录上下文（输入）
pub struct AiContext {
    pub target_path: String,
    pub size_delta_mb: f64,
    pub current_size_mb: f64,
    pub top_large_files: Vec<(String, u64)>,
    pub primary_extensions: Vec<(String, f32)>,
}

/// 风险等级（与 07-safety.md 一致）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RiskLevel {
    Safe,       // 用户个人目录下的非系统缓存
    Low,        // 应用程序缓存目录
    Medium,     // 系统级辅助目录（如 /var/tmp）
    High,       // 系统目录
}

/// AI 对单个路径的分析结论（输出）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiResult {
    pub path: PathBuf,
    pub label: String,           // 来源实体名，如 "Docker Buildx", "pip cache"
    pub description: String,     // 用途说明
    pub risk_level: RiskLevel,   // 删除风险等级
    pub suggestion: String,      // 治理建议
    pub deletable: bool,         // AI 认为是否可删
    pub confidence: f32,         // AI 置信度 0.0-1.0
}

/// AI 缓存：以全路径为键，避免重复请求同一目录
pub type AiCache = HashMap<PathBuf, AiResult>;
```

提取逻辑：
1. 根据用户选中的子路径，从 Diff 树中截取对应子树。
2. 统计子树下变动最大的 Top 5 文件。
3. 计算主要后缀名分布（如 `.log` 占 90%）。

### 2.4 批量分析与反馈映射

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

## 3. 快照持久化

### 3.1 MVP 阶段（JSON）

- 使用 `serde_json` 序列化/反序列化。
- 存储路径：`~/.config/argus/snapshots/{root_path_hash}_{timestamp}.json`（`root_path_hash` 防止多盘扫描冲突，取 SHA256 前 8 字符）。
- 快照文件头部包含 `version` 字段（当前为 `1`），用于格式演进：

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Snapshot {
    pub version: u32,          // 快照格式版本号，当前 = 1
    pub timestamp: DateTime<Utc>,
    pub root_path: PathBuf,
    pub root_path_hash: String, // SHA256(root_path) 前 8 字符
    pub total_size: u64,
    pub root_node: FileNode,
}
```

- `root_path_hash` 用于：
  - 快照文件名去重：`{root_path_hash}_{timestamp}.json`
  - 加载时校验路径一致性（防止 `--old snap_a.json --new snap_b.json` 时错用不同根路径的快照）
- 依赖：`sha2 = "0.10"`（见 Phase 1 实施指南）

- 反序列化时校验 `version`：不匹配时返回 `SnapshotError::VersionMismatch`，阻止加载旧格式。
- 优点：开发时可肉眼 Debug，无需额外工具。

### 3.2 后期演进（二进制格式）

- 可迁移至 FlatBuffers 或 `bincode` 以提升性能和减小体积。
- 保留 JSON 格式作为兼容选项。

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

### 5.2 SnapshotError

```rust
#[derive(thiserror::Error, Debug)]
pub enum SnapshotError {
    #[error("快照版本不兼容: 期望 v{expected}, 实际 v{actual}")]
    VersionMismatch { expected: u32, actual: u32 },

    #[error("快照文件损坏: {0}")]
    Corrupted(String),

    #[error("快照文件不存在: {0}")]
    NotFound(PathBuf),

    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),
}
```

**行为策略**：
- `VersionMismatch`：提示用户快照版本不兼容，推荐重新扫描，不自动修复。
- `Corrupted`：记录日志，建议用户删除损坏快照重新生成。

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

## 6. 数据库时序模型（Phase 3+）

Daemon 模式下，用 SQLite 替代 JSON 快照实现灵活的时序查询。

### 6.1 核心表

```sql
-- 每次扫描的记录，全局唯一标识一次扫描事件
CREATE TABLE scan_events (
    id          INTEGER PRIMARY KEY,
    timestamp   TEXT NOT NULL,  -- ISO 8601
    root_path   TEXT NOT NULL,  -- 扫描根路径
    total_size  INTEGER NOT NULL
);

-- 每个文件/目录在每次扫描中的大小记录
CREATE TABLE path_records (
    scan_id     INTEGER NOT NULL REFERENCES scan_events(id),
    path        TEXT NOT NULL,   -- 绝对路径，如 "/home/user/Downloads/big.iso"
    size        INTEGER NOT NULL, -- 该时间点的大小（字节）
    is_dir      INTEGER NOT NULL, -- 0/1
    PRIMARY KEY (scan_id, path)
);

CREATE INDEX idx_path_records_path ON path_records(path);
CREATE INDEX idx_path_records_scan_id ON path_records(scan_id);
```

### 6.2 查询模式

Time-Series Query 的核心差异在于：**从"加载两份全量 JSON 做 diff"变为"SQL 聚合任意子树在任意时间范围的 delta"**。

```sql
-- 查询 /home/user/Downloads 在 scan_id=(A, B) 之间的 delta
SELECT
    p1.path,
    p2.size - p1.size AS delta,
    p2.size AS current_size
FROM path_records p1
JOIN path_records p2 ON p1.path = p2.path
WHERE p1.scan_id = 1  -- from
  AND p2.scan_id = 2  -- to
  AND p1.path LIKE '/home/user/Downloads/%'
```

### 6.3 与快照对比的关系

- **快照对比 = 一次性加载完整子树，内存中做 Tree Merge**
- **时序查询 = 按需 SQL 查询（仅加载可见节点），无 Tree Merge**
- 两者共享相同的 `DiffNode` 展示结构。TUI 的 diff 渲染层对数据来源无感知
- 独立模式输出 `DiffNode`，daemon 模式也输出 `DiffNode`，TUI 统一渲染
