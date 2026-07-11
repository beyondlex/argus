# Phase 3: Daemon 自动化 — 设计文档

## 1. 概览

Phase 3 实现 `argusd` 守护进程：文件系统事件监听、delta 存储与查询、UDS IPC 通信。
TUI 升级支持服务模式，叠加 delta 可视化覆盖层。

### 交付物

- `argusd` 守护进程二进制
- 升级后的 `argus-tui`（standalone ↔ server 双模）
- SQLite delta 数据库 (schema + 查询)
- UDS IPC 协议

## 2. 系统架构

```
                        argus-tui / argus-cli
                              │
                     [UDS Client: DaemonRequest]
                              │
                              ▼
┌──────────────────────── argusd ──────────────────────────┐
│                                                          │
│  ┌──────────┐    ┌────────────┐    ┌──────────────────┐  │
│  │ watcher  │───▶│  debounce  │───▶│  db (SQLite)     │  │
│  │ (notify) │    │  10s 窗口   │    │  delta_events    │  │
│  └──────────┘    │ 路径合并    │    │  + 查询 API      │  │
│                  └────────────┘    └────────┬─────────┘  │
│                                             │            │
│  ┌──────────────────────────────────────────┘            │
│  │                                                       │
│  ▼                                                       │
│  ┌──────────────────────────────────────┐                │
│  │  ipc_server (UDS UnixListener)       │                │
│  │  GetDelta {path, from, to}           │                │
│  │  Ping / Pong                         │                │
│  └──────────────────────────────────────┘                │
│                                                          │
│  ┌──────────────────────────────────────┐                │
│  │  main.rs                             │                │
│  │  1. load config                      │                │
│  │  2. init db                          │                │
│  │  3. start watcher                    │                │
│  │  4. start debounce engine            │                │
│  │  5. bind UDS                         │                │
│  │  6. wait shutdown signal             │                │
│  └──────────────────────────────────────┘                │
└──────────────────────────────────────────────────────────┘
```

## 3. 数据库 Schema

### 3.1 delta_events 表

位置：`argus-core/src/db.rs` （core 层，daemon 和 client 共用）

```sql
CREATE TABLE IF NOT EXISTS delta_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    path        TEXT    NOT NULL,       -- 绝对路径
    delta_size  INTEGER NOT NULL,       -- 字节变化 (+/-)
    event_type  TEXT    NOT NULL,       -- 'create' | 'modify' | 'delete'
    timestamp   INTEGER NOT NULL,       -- Unix 毫秒
    is_agg      INTEGER DEFAULT 0,      -- 是否为去抖聚合记录
    process_info TEXT   DEFAULT NULL     -- 预留：未来可追踪进程信息
);

CREATE INDEX IF NOT EXISTS idx_delta_path_time
    ON delta_events(path, timestamp);

CREATE INDEX IF NOT EXISTS idx_delta_timestamp
    ON delta_events(timestamp);
```

### 3.2 DB 查询 API （`argus-core/src/db.rs`）

```rust
/// 查询指定路径在时间范围内的 delta 汇总
pub fn query_delta_total(
    conn: &Connection, path: &Path, from_ms: u64, to_ms: u64
) -> Result<i64, DbError>

/// 查询 delta 明细（用于 UI 展开时间线）
pub fn query_delta_detail(
    conn: &Connection, path: &Path, from_ms: u64, to_ms: u64
) -> Result<Vec<DeltaEntry>, DbError>

/// 批量插入去抖后的事件（事务内）
pub fn insert_events(
    conn: &Connection, events: &[DeltaEvent]
) -> Result<(), DbError>

/// 删除指定时间之前的数据（保留策略触发）
pub fn purge_events_before(
    conn: &Connection, before_ms: u64
) -> Result<u64, DbError>

/// 初始化数据库表
pub fn init_db(conn: &Connection) -> Result<(), DbError>
```

### 3.3 DeltaEntry / DeltaEvent 结构

放在 `argus-core/src/model.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaEvent {
    pub path: PathBuf,
    pub delta_size: i64,
    pub event_type: String,
    pub timestamp: u64,
    pub is_agg: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaEntry {
    pub path: PathBuf,
    pub delta_size: i64,
    pub event_type: String,
    pub timestamp: u64,
}
```

## 4. 文件监听 —— watcher

### 4.1 技术选型

- macOS: `notify::event::EventKind` 底层使用 FSEvents
- Linux: `notify::event::EventKind` 底层使用 inotify
- 跨平台统一事件处理，不直接依赖平台 API

### 4.2 事件处理流程

```
notify::Event
  │  {kind, paths, attrs}
  ▼
watcher 层
  │ 1. 过滤: 排除 ignore 规则 / 非监控目录 / 系统临时文件
  │ 2. 获取 size: stat() 读取文件当前大小
  │ 3. 维护 hardlink 缓存: (device, inode) → path
  │ 4. 组装 DeltaEvent {path, delta_size, event_type, timestamp}
  │
  ▼
tokio::sync::mpsc 通道
  │
  ▼
debounce 引擎
```

### 4.3 EventKind → delta_size 映射

| EventKind | delta_size 计算 |
|-----------|----------------|
| `Create(_)` | `+stat(path).len()` (新文件大小) |
| `Modify(_)` | `+new_size - old_size` (差值，通过 size_cache 获取旧值) |
| `Remove(_)` | `-size_cache.get(path)` (从缓存获取最后已知大小) |
| `Rename(_)` | 等效于 Remove(old) + Create(new)，目标路径重新 stat |

### 4.4 Size Cache

watcher 维护一个 `HashMap<PathBuf, u64>` 记录每个文件最后已知大小：
- Create: 插入 `path → size`
- Modify: 更新 `path → new_size`
- Remove: 删除条目
- 去抖引擎在写入 DB 后也会刷新 size cache
- daemon 启动时可选择做一次全量扫描填充初始 size cache

## 5. 去抖引擎 —— debounce

### 5.1 目标

连续大量文件变动（如 `cargo build` 写入 `target/`）需要合并后写入。
10 秒窗口内同一路径的多次事件聚合成一条记录。

### 5.2 合并规则

| 同一窗口内的序列 | 合并结果 |
|-----------------|---------|
| Create + Modify + Modify | 一条 Create，delta = 最终 size |
| Modify + Modify | 一条 Modify，delta = 最终差值 |
| Create + Remove | 抵消，不入库 |
| Modify + Remove | 一条 Remove，delta = 移除 last_known_size |
| 路径无关的事件 | 各自独立，不合并 |

### 5.3 实现

```rust
struct DebounceEntry {
    event: DeltaEvent,
    expires_at: Instant,
}

struct DebounceEngine {
    pending: HashMap<PathBuf, DebounceEntry>,
    window: Duration,  // 默认 10s
    event_rx: mpsc::Receiver<DeltaEvent>,
    db: Connection,
}
```

- 新事件到达: 更新 `pending[path]`，重置计时器
- Tick 检查: 每秒扫描 `pending`，`expires_at` 已过的写入 DB 并移除
- Daemon 退出时: 刷新所有 pending 事件到 DB

## 6. IPC 协议

### 6.1 传输层

- 协议: Unix Domain Socket
- 路径: 默认 `/tmp/argusd.sock`（可配置）
- 帧格式: 4 字节大端长度前缀 + bincode 负载

```
┌────────────────┬──────────────────────────────┐
│  4 bytes (u32) │   N bytes (bincode payload)  │
│   payload_len  │  DaemonRequest / Response     │
└────────────────┴──────────────────────────────┘
```

### 6.2 消息类型

```rust
#[derive(Serialize, Deserialize, Debug)]
pub enum DaemonRequest {
    /// 查询 delta 汇总
    GetDelta { path: PathBuf, from_ms: u64, to_ms: u64 },
    /// 查询 delta 明细
    GetDeltaDetail { path: PathBuf, from_ms: u64, to_ms: u64 },
    /// 健康检查
    Ping,
    /// 获取 daemon 状态
    GetStatus,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DaemonResponse {
    Delta { total_delta: i64, entries: Vec<DeltaEntry> },
    DeltaDetail { entries: Vec<DeltaEntry> },
    Pong,
    Status { version: String, watch_dirs: Vec<PathBuf>, uptime_secs: u64 },
    Error { message: String },
}
```

### 6.3 UDS Server

```rust
// tokio::net::UnixListener
// 每个连接 spawn 一个 task:
//   1. 读取 4 字节 length prefix
//   2. 读取 N 字节 payload
//   3. bincode::deserialize 为 DaemonRequest
//   4. 匹配处理 → 查询 DB → 返回 DaemonResponse
```

注意事项:
- `SO_RCVTIMEO` 防止死连接长期占用
- 每个连接独立事务，不互相阻塞
- 连接断开时自动清理

### 6.4 协议代码位置

IPC 协议类型定义放在 `argus-core` 中（`core/src/ipc.rs`），这样 daemon 和 client 端共享同一个消息定义。`argusd` 中的 server 实现和 `argus-tui` 中的 client 实现各自引用 core 中的类型。

## 7. TUI 服务模式升级

### 7.1 启动检测

```
TUI 启动
  ├── 尝试连接 UDS (/tmp/argusd.sock)
  ├── 连接成功 → server 模式
  │   ├── 发送 Ping → 收到 Pong → 确认 daemon 存活
  │   ├── 文件树新增 delta 列
  │   └── 时间筛选栏可见
  └── 连接失败 → standalone 模式
      ├── 无 delta 列
      ├── 无时间筛选
      └── 行为同 Phase 2
```

### 7.2 Delta 覆盖层

server 模式下文件树新增第三列 `Δ`:

```
  name               size         Δ
  ├── Downloads/    4.4 GB    +1.2 GB
  │   ├── big.iso   2.0 GB    +2.0 GB
  │   └── logs/     1.5 GB    -200 MB
```

颜色:
- `+` 变化: 红色/黄色 (theme config)
- `-` 变化: 绿色
- 零变化: 灰色 (或隐藏)

delta 值显示的阈值过滤: `|delta| < threshold` 时显示 `-`

### 7.3 时间筛选栏

新增底部栏: `[From: 10:00] [To: 15:00] [Apply]`
- 默认: 最近 1 小时
- 快捷键: `t` 切换时间范围预设 (1h / 6h / 24h / 7d / custom)
- 按 `Apply` 后发送 `GetDelta` 请求，刷新全树 delta

### 7.4 降级机制

- daemon 连接断开时: TUI 检测到 I/O 错误
- 自动移除 delta 列和时间筛选栏
- 状态栏显示 `[Server: disconnected]` 提示
- 用户可手动重连 (快捷键 `R`)

## 8. 实现顺序

### Step 1: 数据结构与 DB
- 在 `argus-core/src/model.rs` 添加 `DeltaEvent`, `DeltaEntry`
- 完善 `argus-core/src/db.rs` 实现完整 SQLite schema 和查询 API
- 单元测试覆盖所有查询场景

### Step 2: IPC 协议类型
- 新建 `argus-core/src/ipc.rs` 定义 `DaemonRequest`, `DaemonResponse`
- 单元测试: 序列化/反序列化 roundtrip

### Step 3: argusd crate 创建
- 新建 `argusd/Cargo.toml` (依赖: `argus-core`, `tokio`, `notify`, `bincode`, `tracing`)
- 新建 `argusd/src/main.rs` (启动骨架)
- 注册到 workspace

### Step 4: watcher 模块
- `argusd/src/watcher.rs`: notify 事件循环
- size cache + hardlink dedup
- 单元测试: mock 事件输入 → 验证 delta event 输出

### Step 5: debounce 模块
- `argusd/src/debounce.rs`: 事件缓冲、路径合并、延迟写入
- 单元测试: 合并规则、写入后刷新

### Step 6: IPC Server
- `argusd/src/ipc.rs`: UDS listener + 请求分发
- 集成测试: client 连接 → 发送请求 → 验证响应

### Step 7: Daemon 主流程
- 配置加载、信号处理、优雅退出
- 冷启动全量扫描 (可选)
- 端到端测试

### Step 8: TUI IPC Client
- `argus-tui` 新增 UDS 连接、`Ping`/`GetDelta` 请求
- 启动时 daemon 探测 + 自动模式切换
- 降级重连机制

### Step 9: TUI Delta 覆盖层
- 文件树新增 delta 列
- 时间筛选栏
- delta 明细弹窗 (选中节点 + Enter 展开时间线)

### Step 10: 集成测试
- 启动 daemon → 创建/修改/删除文件 → 查询 delta → 验证
- 去抖时间窗口验证
- UDS 并发客户端
- TUI standalone ↔ server 切换

## 9. 配置变更

`config.toml` 守护进程配置组 (已有定义 in `04-configuration.md` §5，确认完备):

```toml
[daemon]
watch_dirs = ["/Users/lex/Downloads", "/Users/lex/Desktop"]
debounce_seconds = 10
uds_path = "/tmp/argusd.sock"

[daemon.snapshot_retention]
hourly_retention_days = 7
daily_retention_days = 30

[logging]
file_level = "info"
```

## 10. 关键设计决策

| 决策 | 选择 | 理由 |
|------|------|------|
| Delta 存储 | SQLite (已有 rusqlite 依赖) | 一致性强、查询灵活、不需要额外依赖 |
| IPC 编码 | bincode | 紧凑、反序列化快，适合 UDS 低延迟 |
| 消息帧 | 4 字节长度前缀 | 简单可靠，无额外协议栈 |
| 去抖窗口 | 10s 固定窗口 | 实现简单，满足大部分使用场景 |
| size cache | HashMap 内存缓存 | watcher 高频使用，避免每次 stat |
| 协议类型位置 | argus-core | daemon 和 client 端共享同一消息定义 |
| 硬链接去重 | (device, inode) → path | 复用 Phase 1 的 dedup |
| 冷启动扫描 | 可选 | 启动时填充 size cache，可配置跳过 |

## 11. 依赖变更

### argus-core 新增依赖
- `bincode = "1"` — IPC 序列化

### argusd 新增依赖
- `argus-core` — 共享模型和 DB 层
- `tokio = { version = "1", features = ["full"] }` — 异步运行时
- `notify = "7"` — 文件系统事件监听
- `bincode = "1"` — IPC 序列化
- `tracing` + `tracing-subscriber` — 日志

### argus-tui 新增依赖
- `bincode = "1"` — IPC 反序列化 (client 端)

## 12. 文件变更清单

### 新增文件

```
argusd/
├── Cargo.toml
└── src/
    ├── main.rs
    ├── watcher.rs
    ├── debounce.rs
    └── ipc.rs

argus-tui/src/
├── ipc_client.rs (新增)
└── components/
    ├── delta_column.rs (新增)
    └── time_filter.rs (新增)
```

### 修改文件

```
argus-core/src/
├── lib.rs          — 导出 ipc 模块, 更新 db 导出
├── model.rs        — 新增 DeltaEvent, DeltaEntry
├── db.rs           — 完整实现 SQLite schema + 查询 API
└── ipc.rs          — 新增 IPC 协议类型

argus-tui/src/
├── app.rs          — server_mode 字段, delta_cache, time_range
├── handler.rs      — t 切换时间, R 重连, delta 快捷键
├── components/
│   ├── file_tree.rs  — 新增 delta 列渲染
│   ├── mod.rs        — 注册新组件
│   └── status_bar.rs — 显示 daemon 连接状态
├── config.rs       — DaemonConfig

argus-tui/Cargo.toml — 添加 bincode 依赖
Cargo.toml           — 添加 argusd 成员
```

## 13. 边界情况

| 场景 | 行为 |
|------|------|
| daemon 启动时 UDS 地址已占用 | 清理旧 socket 文件后重试，若重用失败则报错退出 |
| 监控目录被卸载/unmount | watcher 检测到错误，记录 WARN 日志，暂停监控该目录，保留已有数据 |
| 数据库写入失败 | 记录 ERROR，保留去抖缓存，3 秒后重试，连续 3 次失败则退出 |
| daemon 磁盘满 | 保留策略自动触发 purge，释放空间后继续 |
| 并发查询量大 | SQLite WAL 模式，读写不互相阻塞 |
| 单个监控目录 100 万+ 文件 | watcher 事件流可能压力大，去抖引擎会聚合 -> DB 写入批次限制 |
| TUI 连接中断后重连 | 自动退化为 standalone，按 `R` 重连，重连后在 DB 中查询中断期间数据 |