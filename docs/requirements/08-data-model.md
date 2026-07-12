# 数据模型与核心算法

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

## Delta 查询

- `query_delta_total(conn, path, from, to)` → `i64`：按路径前缀 + 时间范围求和
- `query_delta_detail(conn, path, from, to)` → `Vec<DeltaEntry>`：返回所有匹配行

两者都使用 `WHERE (path = ? OR path LIKE ? || '/%') AND timestamp BETWEEN ? AND ?` 语义。

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

### 合并示例

```
原始: /target/debug/foo.o (+500), /target/debug/bar.o (+300), ... (600 files)
合并后: /target/debug/ (agg, +150000)
```

## Scanner 跳过目录

通过 `skip_dirs` 配置，在扫描时跳过 `node_modules`, `target` 等高频变动目录。
跳过目录会记录总大小（单独统计），但不展开内部结构。