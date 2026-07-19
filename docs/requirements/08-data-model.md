# 数据模型与核心算法

## Label 类型

Label 是 AI 分析结果中的分类标签，由程序基于内置启发式规则和用户配置确定：

```rust
pub type Label = String;

// 内置常量定义
pub mod labels {
    pub const BUILD_ARTIFACTS: &str = "build-artifacts";
    pub const PACKAGE_DEPENDENCIES: &str = "package-dependencies";
    pub const VCS_DATA: &str = "vcs-data";
    pub const APP_CACHE: &str = "app-cache";
    pub const LOG_FILES: &str = "log-files";
    pub const TEMP_FILES: &str = "temp-files";
    pub const DOWNLOADS: &str = "downloads";
    pub const FRAMEWORK_CACHE: &str = "framework-cache";
    pub const HIDDEN_CONFIG: &str = "hidden-config";
    pub const UNCATEGORIZED: &str = "uncategorized";
}
```

Label 确定优先级：`内置启发式 → 用户配置 custom_mappings → AI 补充`。
后一层覆盖前一层。AI 不直接输出 label，只输出 label_detail 作为补充描述。

## AiPathVerdict

```rust
pub struct AiPathVerdict {
    pub path: PathBuf,
    pub size: u64,
    pub label: Label,           // 程序确定，稳定可控，用于分组排序
    pub label_detail: String,   // AI 确定，自由描述，仅用于展示
    pub purpose: String,
    pub risk_level: RiskLevel,
    pub suggestion: String,
    pub deletable: bool,
}
```

## DeltaEvent / DeltaEntry

- `DeltaEvent`: 全字段版本（含 `is_agg`, `process_info`），用于 watcher → debounce 管道
- `DeltaEntry`: 精简版，用于 DB 读写与 IPC 传输

```rust
pub struct DeltaEntry {
    pub path: PathBuf,       // 事件路径
    pub delta_size: i64,     // 字节变化量（正=增长，负=减少）
    pub event_type: String,  // "create" | "modify" | "delete" | "agg"
    pub timestamp: u64,      // 毫秒时间戳
    pub is_agg: bool,        // 是否为合并记录
}
```

## 数据库表

`delta_events` 表：

```sql
CREATE TABLE delta_events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    path         TEXT    NOT NULL,
    delta_size   INTEGER NOT NULL,
    event_type   TEXT    NOT NULL,
    timestamp    INTEGER NOT NULL,
    is_agg       INTEGER DEFAULT 0,
    process_info TEXT    DEFAULT NULL
);
CREATE INDEX idx_delta_path_time ON delta_events(path, timestamp);
CREATE INDEX idx_delta_timestamp ON delta_events(timestamp);
```

### is_agg 字段
- `0`: 普通事件（来自 watcher 的原始数据）
- `1`: 合并后的事件（由后台合并任务生成）

### 覆盖口径

`is_agg = 1` 的记录表示“这个路径已经代表了一整棵子树的汇总值”。查询和渲染时必须把它当作覆盖层，而不是再与同一子树下的叶子明细叠加一次。

规则如下：

- 同一子树内，如果父目录已有 `is_agg = 1` 行，则它下面的后代明细不应再参与同一次聚合结果。
- `query_delta_total` 和 `query_delta_detail` 的结果必须使用同一套口径，否则 TUI 会把同一批变化算两遍。
- 回归测试应覆盖“父目录汇总行 + 子孙明细同存”场景，确认不会再次出现重复计数。

## Delta 查询

- `query_delta_total(conn, path, from, to)` → `i64`：按路径前缀 + 时间范围求和
- `query_delta_detail(conn, path, from, to)` → `Vec<DeltaEntry>`：返回与 `query_delta_total` 同口径的匹配行

注意：这里的“匹配行”不是“简单前缀扫描后把所有祖先都加起来”，而是要先排除被更高层 `is_agg` 记录覆盖的后代明细。

两者都以路径前缀 + 时间范围为基础，但实际结果集必须去重，避免父目录汇总与子孙明细重复出现。

## 数据保留

`purge_events_before(conn, before_ms)` → 删除早于指定时间戳的所有事件。
由 daemon 的 retention worker 周期性调用，默认每 60 分钟执行一次。

## 事件合并

`consolidate_events(conn, threshold)` → 合并同目录下子级过多的事件：

1. 查询所有 `is_agg = 0` 的事件
2. 按 `Path::parent()` 分组
3. 对直接子级数 > threshold 的目录：
   - DELETE 所有直接子级事件（不递归子目录）
   - 如果该目录已有 agg 条目则 UPDATE 累加，否则 INSERT 新 agg 条目
4. 使用 SQLite 事务保证原子性

### 易错点

`consolidate_events` 生成的父目录 `is_agg = 1` 行，和它子树内仍保留的叶子行不能在同一次查询里一起按“独立增量”解释。否则：

1. `query_delta_total` 会把父目录值和子孙值同时算进去。
2. TUI 再把这些值向上聚合一次时，父目录会翻倍。

这类问题最容易在 `target/debug/`、`build/`、`deps/` 这类“父目录本身就有聚合行，同时下面还有大量叶子”的路径上暴露出来。

### 合并示例

```
原始: /target/debug/foo.o (+500), /target/debug/bar.o (+300), ... (600 files)
合并后: /target/debug/ (agg, +150000)
```

## Scanner 跳过目录

通过 `skip_dirs` 配置，在扫描时跳过 `node_modules`, `target` 等高频变动目录。
跳过目录会记录总大小（单独统计），但不展开内部结构。
