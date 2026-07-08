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
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub children: HashMap<String, FileNode>,
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | `String` | 文件/目录名（不含路径） |
| `is_dir` | `bool` | 是否为目录 |
| `size` | `u64` | 当前时间点的总大小（字节） |
| `modified` | `Option<DateTime<Utc>>` | 最后修改时间（部分文件系统不可用，故为 Option） |
| `children` | `HashMap<String, FileNode>` | 子节点（目录专用；TUI 展示时需排序，可考虑 IndexMap/BTreeMap） |

### 1.2 Snapshot（快照）

单次扫描的完整持久化结构。

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Snapshot {
    pub timestamp: DateTime<Utc>,
    pub root_path: PathBuf,
    pub total_size: u64,
    pub root_node: FileNode,
}
```

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
3. 按路径深度从深到浅排序。
4. 将子节点 size 累加到父节点。

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

### 2.3 AI 特征提取

```rust
pub struct AiContext {
    pub target_path: String,
    pub size_delta_mb: f64,
    pub current_size_mb: f64,
    pub top_large_files: Vec<(String, u64)>,
    pub primary_extensions: Vec<(String, f32)>,
}
```

提取逻辑：
1. 根据用户选中的子路径，从 Diff 树中截取对应子树。
2. 统计子树下变动最大的 Top 5 文件。
3. 计算主要后缀名分布（如 `.log` 占 90%）。

## 3. 快照持久化

### 3.1 MVP 阶段（JSON）

- 使用 `serde_json` 序列化/反序列化。
- 存储路径：`~/.config/argus/snapshots/{timestamp}.json`。
- 优点：开发时可肉眼 Debug，无需额外工具。

### 3.2 后期演进（二进制格式）

- 可迁移至 FlatBuffers 或 `bincode` 以提升性能和减小体积。
- 保留 JSON 格式作为兼容选项。

## 4. API 设计（面向 Daemon）

守护进程与客户端之间的 IPC 协议：

```rust
enum ArgusRequest {
    GetDiff {
        from_timestamp: u64,
        to_timestamp: u64,
        threshold_bytes: u64,
    },
    GetAIContext {
        path: PathBuf,
    },
    TriggerDelete {
        path: PathBuf,
        secure: bool,     // 是否启用安全模式（废纸篓优先）
    },
    ListSnapshots,
    GetConfig,
    SetConfig { key: String, value: String },
}

enum ArgusResponse {
    DiffResult { root: DiffNode },
    AIContext { context: AiContext },
    DeleteResult { success: bool, path: PathBuf },
    SnapshotList { timestamps: Vec<u64> },
    ConfigData { content: String },
    Error { message: String },
}
```
