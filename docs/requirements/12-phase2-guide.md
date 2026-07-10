# Phase 2 实施指导文档 — TUI 极客版

## 0. Phase 2 实现契约

| 主题 | 权威文档 |
|------|---------|
| TUI 布局、快捷键、列定义 | `05-ux-interaction.md` |
| 配置文件字段（keybindings / theme） | `04-configuration.md` |
| 安全删除与系统黑名单 | `07-safety.md` |
| 核心数据结构与错误类型 | `08-data-model.md` |
| 日志字段与输出 | `11-logging.md` |

Phase 2 交付 `argus-tui`，工作在**独立模式**（Standalone），直接调用 `argus-core`，不依赖 `argusd` 守护进程。

**不属于 Phase 2**：Daemon IPC 接入、真实 AI API 调用、AI 自动诊断面板、配置修改命令、审计查看命令。这些留待 Phase 3/4。

### 独立模式路径语义

独立模式下，TUI 始终以当前工作目录 (cwd) 为根的文件树。文件树由 **FS 结构 + size overlay** 驱动：

| 方面 | 当前模型 |
|------|--------|
| 启动流程 | 加载 scan_cache + 以 cwd 为根；有数据则补齐 size，无数据则 `list_dir` 惰性读 FS |
| 目录 size | 有扫描数据则展示，无则显示 `"-"` |
| 文件 size | 始终有（`stat` 单次调用低成本） |
| 按 `s` | 直接扫描当前树根（cwd），不再弹输入框 |
| 路径切换 | 用 `h` 向上导航到父目录（改变树根），用 `l` 向下展开 |
| Delta 控制 | 顶部筛选栏控制；无时间选择时无 delta 列 |
| 多路径快照 | 全量加载到 scan_cache，仅当前树根的历史用于 diff |

> **为什么改变模型**：之前的"无快照则空白"体验差，用户无法先浏览再决定扫哪里。cwd 根的文件树 + 惰性加载让用户打开 TUI 就能看到当前目录，按需扫描。

## 1. 环境准备

### 1.1 Cargo.toml

```toml
[package]
name = "argus-tui"
version = "0.1.0"
edition = "2021"

[dependencies]
argus-core = { path = "../argus-core" }
ratatui = "0.29"
crossterm = "0.28"
tokio = { version = "1", features = ["full"] }
chrono = "0.4"
anyhow = "1.0"
toml = "0.8"
```

**依赖说明**：

| 依赖 | 用途 |
|------|------|
| `ratatui` | 终端 UI 框架（tui-rs 继承者），提供 widget、布局、渲染 |
| `crossterm` | 终端后端（事件捕获、raw mode、颜色），与 ratatui 配对 |
| `tokio` | 异步运行时，用于后台扫描、diff 计算不阻塞 UI |
| `toml` | 读取 `~/.config/argus/config.toml` |

### 1.2 在 Workspace 注册

在根 `Cargo.toml` 的 `members` 中添加 `"argus-tui"`。

### 1.3 模块结构

```
argus-tui/src/
├── main.rs              # 入口，初始化终端，启动事件循环
├── app.rs               # App 状态机（状态 + mode + 全局状态）
├── components/
│   ├── mod.rs
│   ├── file_tree.rs     # 文件树浏览器组件
│   ├── filter_bar.rs    # 顶部筛选栏（时间范围 + delta 阈值）
│   ├── metadata.rs      # 元数据面板组件
│   ├── status_bar.rs    # 底部状态栏
│   └── help_popup.rs    # 帮助弹窗
├── event.rs             # 事件循环 + 异步任务管理
├── handler.rs           # 按键分发逻辑
├── config.rs            # 加载 ~/.config/argus/config.toml
└── util.rs              # 格式化、颜色工具
```

## 2. 核心架构

### 2.1 App 状态机

```
                ┌─────────────┐
                │  Browsing   │ ← 默认状态，浏览文件树
                └──────┬──────┘
                       │ d
                ┌──────v──────┐
                │ DeletePrompt│ ← 删除确认弹窗
                └──────┬──────┘
                       │ y / n
                ┌──────v──────┐
                │  Browsing   │ ← 回到浏览状态
                └─────────────┘
```

```
enum AppMode {
    Browsing,       // 普通浏览
    DeletePrompt,   // 删除二次确认
    Help,           // 帮助面板覆盖层
}
```

**Phase 2 范围**：仅实现 `Browsing` 和 `DeletePrompt` 模式。`Help` 模式为 P1 优先级。

### 2.2 布局结构

```
+--------------------------------------------------+
|  Argus v0.1.0              [?] Help  [Q] Quit    |
+--------------------------------------------------+
|  时间: [2026-06-01 → 2026-07-01]  Δ≥ [50 MB]  [清除] │
+---------------------------+----------------------+
|                           |  Metadata Panel      |
|  File Tree (70%)          |  - path              |
|                           |  - current/delta     |
|  ~/                       |  - file count        |
|  ├── Desktop/ +1.2GB     |  - modified time     |
|  ├── file.iso  +500MB    +----------------------+
|  └── ...                  |  Status Bar          |
|                           |  file: 1024 | scanning...
+---------------------------+----------------------+
```

- **Filter Bar**：第 2 行，时间范围选择 + delta 阈值 + 清除按钮（详见 §3.6）
- **File Tree**：左 70%，可滚动文件树
- **Metadata Panel**：右 30%，显示选中节点的详情
- **Status Bar**：底部，显示状态信息（进度、模式提示）

**P1 增强**：Metadata Panel 底部增加文件类型分布条形图。

### 2.3 独立模式工作流

TUI **始终以 cwd 为根**展示文件树。delta 是树上的可选的筛选覆盖层：

```
┌───────────┐
│   启动    │
└─────┬─────┘
      │
      ├── 从 SQLite 加载扫描历史到 scan_cache (HashMap<PathBuf, Snapshot>)
      │
      ├── 确定 cwd = std::env::current_dir()
      │
      ├── scan_cache 有 cwd 的数据？
      │     ├── 是 → 以 FS 结构为底叠加 size（目录可展开，结构不变）
      │     └── 否 → list_dir(cwd) → 渲染 FS 树
      │                 ├── 文件：真实 size
      │                 └── 目录：size = "-"（无汇总）
      │
      ├── auto_scan_on_start = true?
      │     └── 是 → 后台 scan_path(cwd) → 完成后更新树
      │
      │
      ├── 用户导航（l / h 切换目录）
      │     ├── scan_cache 有该目录 → 以 FS 结构为底叠加 size
      │     └── scan_cache 无该目录 → list_dir 惰性读取 → 渲染 FS 树
      │
      ├── 用户按 s → 扫描当前树根（cwd）→ 无输入框
      │     ├── scan_path(cwd) → 写入 SQLite → 更新 scan_cache
      │     └── 刷新当前树根的 size overlay
      │
      └── 用户在筛选栏选择时间范围
            ├── 有 ≥2 个该路径的扫描记录 → 后台 diff 查询 → 显示 delta
            └── 缺少记录 → 提示"需再次扫描此路径以对比"
```

**核心原则**：

1. **文件树永远存在**：以 cwd 为根，始终可自由游走。扫描是增强，不是前提。见 `docs/plans/standalone-fs-navigation-refactor.md`。

2. **没有路径选择器**：按 `s` 不弹输入框，直接扫描当前树根（cwd）。要扫描其他路径，先导航到该目录，按 `s`。

3. **Delta 始终可选**：筛选栏的时间选择器列出当前树根路径下可用的扫描时间戳。Phase 2 仅支持选择两个时间点（from/to 都选）做 diff。用户选择后触发后台 SQLite delta 查询任务。

4. **无快照或不匹配时不报错**：降级为 FS 导航模式，树展示目录结构（文件有真实 size，目录显示 `"-"`；结构占位节点显示 `"..."`）。用户按 `s` 扫描获得数据后自然升级。

5. **筛选栏状态**：时间选择器为空 → 无 delta 列。时间选择器有值但快照不够 → 灰色不可用状态，提示原因。

### 2.4 数据流

```rust
// 后台 → UI 的消息类型
enum AppMessage {
    ScanProgress { file_count: u64, total_bytes: u64 },
    ScanComplete(Snapshot),
    DiffComplete(DiffNode),
    Error(AppError),
}
```

```rust
// UI → 后台的请求
enum AppCommand {
    Scan { path: PathBuf, cancel: Arc<AtomicBool> },
    Diff { path: PathBuf, from: DateTime<Utc>, to: DateTime<Utc> },
}
```

```rust
enum AppError {
    Scan(ScanError),
    Storage(DbError),
    Diff(DiffError),
    Io(std::io::Error),
}
```

**筛选状态与 diff 的关系**：筛选栏状态存储在 `AppState` 中（`filter_from: Option<DateTime>`, `filter_to: Option<DateTime>`, `filter_threshold: Option<u64>`）。当筛选状态改变：

```
筛选栏时间变化
  → 检查该路径下是否有对应时间点的扫描记录
  → 无 → 灰显 + 提示
  → 有 → 记录 (from_timestamp, to_timestamp)
  → 后台执行 SQLite delta 查询 → 返回 DiffNode
  → 覆盖到当前树显示
  → 用户清除筛选 → 清除 DiffNode → 回到纯 ncdu 模式
```

## 3. 组件实现说明

### 3.1 FileTree 组件

**数据结构**：将当前根节点树或 `DiffNode` 展平为可滚动的行列表。这里的 `TreeNode` 是 TUI 层统一视图节点，可封装 `FileNode` 或 `DiffNode`。例如：

```rust
enum TreeNode {
    Snapshot(FileNode),
    Diff(DiffNode),
}
```

每个行节点包含：

```rust
struct TreeLine {
    depth: usize,         // 缩进层级
    node: TreeNode,       // 当前视图节点，可能来自 Snapshot 树或 Diff 树
    expanded: bool,       // 目录是否展开
    has_scan_data: bool,  // 该目录是否有可展示的扫描 size
    delta: i64,           // 当前 diff 查询叠加到该行的 delta
    selected: bool,       // 光标是否在此行
}
```

**行为**：
- `j`/`k`：移动光标上/下一行（跨层级滚动）
- `h`：收起当前目录或返回父级
- `l`：展开当前目录（有子节点时）或进入子目录
- 根节点 `~/` 始终显示且不可收起
- 展开/收起状态存储在 `AppState.expanded: HashSet<String>`（键为全路径）

**渲染**：
- 使用 `ratatui::widgets::List` 或自定义 `Paragraph` 块
- 缩进：每层 2 空格 + `├──`/`└──` 前缀（非 ASCII 艺术，用简单字符）
- 列格式：`name` + `current_size` + `delta`
- delta 正数为红色（`Style::fg(Color::Red)`），负数为绿色（`Style::fg(Color::Green)`）
- Phase 2 支持按名称 / 体积 / 增量排序（`o` 键切换）；排序仅作用于当前展开层级，不改变整棵树的聚合结果。

```rust
#[derive(PartialEq)]
enum SortMode {
    Name,
    Delta,  // Phase 2 默认
    Size,   // Phase 2 可选
}
```

**行数优化**：树的叶子节点可能数十万行。FileTree 只维护当前展开分支的扁平行缓存，并按视口范围切片渲染，避免一次性渲染整棵树。`Scrollbar` 只负责可视化当前位置，不承担虚拟化逻辑。

### 3.2 Metadata 面板

显示选中节点信息：

| 字段 | 内容 |
|------|------|
| Path | 选中节点的完整相对路径 |
| Current Size | 该节点的当前总大小 |
| Size Delta | 变动量（带 +/-） |
| File Count | 该目录下的文件总数（仅目录有） |
| Last Modified | 最近修改时间（仅文件有） |
| File Type | 文件类型分布（仅目录，P1） |

Phase 2 直接使用 `ratatui::widgets::Paragraph` + `Block` 实现，不引入复杂 widget。

### 3.3 Status Bar

```
file: 1024 | scanning: 50% | [?] Help  [Q] Quit
```

底部固定行，不随文件树滚动。显示：
- 当前文件计数
- 扫描进度（后台有扫描任务时）
- 快捷键提示

### 3.4 Help Popup（P1）

`?` 键触发居中覆盖层，列出所有可用快捷键。

### 3.5 Config 加载

```rust
pub struct TuiConfig {
    pub keybindings: Keybindings,
    pub theme: Theme,
}

impl Default for TuiConfig {
    fn default() -> Self {
        // 硬编码默认值，对应 Vim 式快捷键
    }
}
```

从 `~/.config/argus/config.toml` 加载 `[keybindings]` 和 `[theme]` 组。文件不存在时使用全默认值。

**Phase 2 范围**：仅读取配置，不提供热重载。配置在启动时加载一次。

### 3.6 FilterBar 组件

文件树上方的筛选栏，用于控制 delta 展示。

```rust
struct FilterState {
    from_idx: Option<usize>,   // 在可用快照列表中的索引
    to_idx: Option<usize>,
    threshold: Option<u64>,    // 字节
    dirty: bool,               // 是否有未应用的筛选变更
}
```

**交互**：
- 通过 `Tab` 将焦点移到筛选栏
- 左右箭头切换「from」「to」「threshold」「清除」四个区域
- `Enter` 激活区域：from/to 展开可用扫描时间列表供选择；threshold 进入数值输入
- `Esc` 或选择后关闭区域
- 清除按钮重置所有筛选状态
- 选择完成后触发后台 SQLite delta 查询 + diff

**Phase 2 仅支持"固定扫描列表"选择**：可用时间列表源于 SQLite 中当前树根路径的扫描记录。用户从列出的时间戳中选择 from 和 to，TUI 直接执行 delta 查询并计算 diff。

**渲染**：使用 `ratatui::widgets::Paragraph` + 自定义样式区分激活/非激活区域。

## 4. 事件循环（event.rs）

```rust
pub async fn run(app: &mut App) -> Result<()> {
    let mut terminal = ratatui::init();
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(16); // ~60fps

    loop {
        terminal.draw(|f| ui::render(f, app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_default();

        if crossterm::event::poll(timeout)? {
            match crossterm::event::read()? {
                Event::Key(key) => handle_key(key, app)?,
                Event::Resize(..) => {}
                _ => {}
            }
        }

        // 处理后台消息
        while let Ok(msg) = app.rx.try_recv() {
            app.handle_message(msg);
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    ratatui::restore();
    Ok(())
}
```

**关键点**：
- 使用 `crossterm::event::poll` 实现非阻塞事件检查，保证 60fps 帧率
- 后台任务通过 `tokio::sync::mpsc` 发送消息
- UI 线程从 `app.rx` 接收消息并更新状态
- `ratatui::init()` 和 `ratatui::restore()` 管理终端 raw mode

## 5. 开发顺序

```
第 0 步：argus-core list_dir() + 测试（惰性目录读取 API）
第 1 步：argus-tui 脚手架（Cargo.toml + main.rs 事件循环 + App skeleton）
第 2 步：config.rs — TUI 配置加载（含 [browsing] 配置组）
第 3 步：app.rs 重构 — scan_cache、view_root_path、load_from_db（SQLite）、rebuild_tree
第 4 步：FileTree 组件 — `"-"` / `"..."` 渲染 + has_scan_data + 惰性展开
第 5 步：handler.rs — s 直接扫描树根 + h 上导航 + l 展开/进入
第 6 步：event.rs — 移除 empty/scan 弹窗 + filter bar 数据源切换
第 7 步：FilterBar 组件 — 按 view_root_path 筛选历史快照
第 8 步：Metadata 面板 — 扫描状态展示
第 9 步：Status Bar — 不变
第 10 步：后台任务 — 扫描完成更新 scan_cache + rebuild_tree
第 11 步：删除交互 — DeletePrompt 模式 + 二次确认（不变）
第 12 步：Help Popup — 帮助面板（P1，更新快捷键说明）
第 13 步：测试编写 — 见 §7
第 14 步：手动验收测试
```

## 6. Phase 2 开发 Checklist

- [ ] `argus-core`: 实现 `list_dir()` + 单元测试
- [ ] 创建 `argus-tui` crate，并注册到 workspace
- [ ] 接入 `ratatui` / `crossterm` / `tokio` / `toml`
- [ ] 配置加载: `BrowsingConfig` + `auto_scan_on_start`
- [x] App 重构: `view_root_path`, `scan_cache`, `load_from_db()`（SQLite 替代旧 JSON 快照主方案）
- [ ] App 重构: `navigate_up()`, `expand_in_place()`, `rebuild_tree()`
- [ ] App 重构: `start_scan()` 直接扫树根（无输入框）
- [ ] App 重构: `handle_message(ScanComplete)` 更新 scan_cache + 刷新树
- [ ] FileTree 渲染: `"- "` 用于无扫描数据的目录、`"..."` 用于结构占位节点、`has_scan_data` 样式
- [ ] handler: `s` 不再弹输入框，直接 `start_scan(app)`
- [ ] handler: `h` 在根节点时 `navigate_up()` 改变树根
- [ ] handler: `l` 展开时检查 scan_cache，有数据则在当前 FS 结构上补 size overlay，无数据用 `list_dir`
- [ ] FilterBar: 按 `view_root_path` 的 hash 筛选历史记录
- [ ] Metadata: 展示扫描状态（`Last scan: ...` / `Not scanned yet`）
- [ ] event: 删除 `render_empty_prompt` / `render_scan_prompt`
- [ ] 实现 `DeletePrompt` 与安全删除前置检查（不变）
- [ ] 补齐单元测试、渲染测试、事件测试、集成测试
- [ ] 跑通手动验收：无快照启动 → 导航 cwd → 按 s 扫描 → 两次扫描 → 筛选对比 → 清除筛选 → 删除确认

## 7. TUI 自动化测试策略

`ratatui` 提供 `TestBackend`，可在无终端环境渲染内容到内存 buffer 并断言。结合状态逻辑的纯单元测试，形成分层测试策略：

### 7.1 测试分层

| 层 | 测什么 | 方式 | 断言目标 |
|----|--------|------|---------|
| **状态逻辑** | AppState 状态迁移、筛选栏状态变化、DiffNode 展平为 TreeLine 列表、展开/收起逻辑 | 纯函数测试，无渲染 | 状态字段值、列表长度/顺序 |
| **组件渲染** | 各组件在给定 state 下渲染出正确的字符布局 | `TestBackend` + `terminal.draw()` | buffer 内容（字符 + 样式） |
| **事件处理** | 按键 → 状态变化 → 重绘结果 | `TestBackend` + mock 按键注入 | 状态 + buffer 内容联合断言 |
| **端到端** | 完整链路：启动无快照 → 输入扫描路径 → 扫描完成 → 渲染树 → 筛选 → diff → 清除 | `TestBackend` + 消息通道模拟 | 多帧 states + buffer 序列 |

### 7.2 状态逻辑测试（纯单元测试）

```rust
#[test]
fn test_tree_flatten_sorts_by_delta() {
    let mut root = DiffNode::mock_root();
    root.children.insert("a".into(), DiffNode::mock("a", 100, 10));
    root.children.insert("b".into(), DiffNode::mock("b", 200, 200));
    root.children.insert("c".into(), DiffNode::mock("c", 50, -50));

    let lines: Vec<TreeLine> = flatten_tree(&root, SortMode::Delta, &HashSet::new());
    // 按 delta 绝对值降序：b(200), c(-50), a(10)
    assert_eq!(lines[0].node.name, "b");
    assert_eq!(lines[1].node.name, "c");
    assert_eq!(lines[2].node.name, "a");
}

#[test]
fn test_filter_state_from_empty_to_set() {
    let mut app = App::new();
    assert!(app.filter_state.is_empty());

    app.filter_state.set_from(Some(0));
    app.filter_state.set_to(Some(1));
    assert!(!app.filter_state.is_empty());
    assert!(app.filter_state.should_diff());
}

#[test]
fn test_known_snapshots_only_from_same_root() {
    let files = vec![
        "hashA_2026-06-01T00:00:00Z.json",
        "hashA_2026-07-01T00:00:00Z.json",
        "hashB_2026-06-15T00:00:00Z.json",
    ];
    let timestamps = available_timestamps(&files, "hashA");
    assert_eq!(timestamps.len(), 2);
}
```

### 7.3 组件渲染测试（TestBackend）

```rust
#[test]
fn test_file_tree_renders_with_delta() {
    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut app = App::with_mock_data();  // 预置 DiffNode + filter 激活
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();

    let buf = terminal.backend().buffer();
    // 树节点可见
    assert!(buf.content().iter().any(|c| c.symbol() == "Downloads/"));
    // delta 列可见
    assert!(buf.content().iter().any(|c| c.symbol() == "+1.2GB"));
    // 筛选栏可见
    assert!(buf.content().iter().any(|c| c.symbol() == "时间:"));

    // 清除筛选后 delta 列消失
    app.filter_state.clear();
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();
    let buf = terminal.backend().buffer();
    assert!(!buf.content().iter().any(|c| c.symbol().contains('+')));
}

#[test]
fn test_empty_state_shows_scan_prompt() {
    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    let app = App::new();  // 无快照、无树
    terminal.draw(|f| ui::render(f, &mut app)).unwrap();

    let buf = terminal.backend().buffer();
    let text: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(text.contains("按 s 扫描目录"));
}
```

### 7.4 事件处理测试（按键注入）

```rust
#[test]
fn test_key_j_moves_cursor_down() {
    let mut app = App::with_mock_tree();
    let initial = app.tree_state.cursor;

    handle_key(KeyEvent::from(KeyCode::Down), &mut app).unwrap();
    assert_eq!(app.tree_state.cursor, initial + 1);
}

#[test]
fn test_key_s_triggers_scan_prompt() {
    let mut app = App::new();  // 无快照
    handle_key(KeyEvent::from(KeyCode::Char('s')), &mut app).unwrap();
    assert!(app.scan_prompt_open());
}

#[test]
fn test_tab_focuses_filter_bar() {
    let mut app = App::with_mock_data();
    assert_eq!(app.focus, Focus::Tree);

    handle_key(KeyEvent::from(KeyCode::Tab), &mut app).unwrap();
    assert_eq!(app.focus, Focus::FilterBar);
}
```

### 7.5 端到端测试（集成）

```rust
#[test]
fn test_scan_then_filter_workflow() {
    // 1. 模拟启动（无快照）
    let mut app = App::new();
    assert!(app.screen_text().contains("按 s 扫描目录"));

    // 2. 模拟扫描完成
    app.handle_message(AppMessage::ScanComplete(mock_snapshot()));
    assert!(app.tree_is_visible());
    assert!(app.delta_column_is_hidden());  // 无 delta 列

    // 3. 模拟第二个快照到达 + 筛选
    app.receive_new_snapshot(mock_snapshot_v2());
    app.filter_state.set_from(Some(0));
    app.filter_state.set_to(Some(1));
    app.trigger_diff();
    app.handle_message(AppMessage::DiffComplete(mock_diff()));

    // 4. 验证 delta 出现
    assert!(!app.delta_column_is_hidden());
    assert!(app.screen_text().contains("+500 MB"));

    // 5. 清除筛选 → 回到 ncdu
    app.filter_state.clear();
    assert!(app.delta_column_is_hidden());
}
```

### 6.6 运行命令

```bash
# 状态逻辑 + 组件渲染 + 事件处理测试
cargo test -p argus-tui

# 单测只看 TUI 状态逻辑
cargo test -p argus-tui -- state

# 端到端集成测试
cargo test -p argus-tui --test integration
```

## 8. 验收标准

### 7.1 启动与布局

```bash
cargo run -p argus-tui
```

- 启动后加载 `~/.config/argus/config.toml`（不存在时使用默认值）
- 从 `~/.config/argus/argus.db` 加载当前 cwd 的最新扫描记录到 scan_cache
- 树根 = 当前工作目录 (cwd)
- 展示四栏布局：筛选栏 | 文件树（左70%）| 元数据（右30%）| 底部状态栏
- **有 cwd 扫描记录**：以 cwd FS 树为底补齐 size，目录显示真实汇总 size，结构保持可展开
- **无 cwd 扫描记录**：显示 cwd FS 文件树（文件有真实 size，目录显示 `"-"`）

### 7.2 文件树浏览

```bash
# 启动后立即浏览 cwd，无需快照
cargo run -p argus-tui
```

- 文件树正确显示 cwd 目录层级
- 文件始终展示真实 size，未扫描目录显示 `"-"`
- `j`/`k` 上下移动光标
- `h`/`l` 展开/收起/进入/返回目录
- `h` 在根节点时导航到父目录（改变树根）
- 默认无 delta 列
- 按 `s` 扫描当前树根（无输入框）

### 7.3 筛选栏与 Delta 展示

```bash
# 创建第二个扫描记录（在 ~/Downloads 中制造 50MB 变动后）
cargo run -p argus-cli -- scan --path ~/Downloads
cargo run -p argus-tui
```

- 筛选栏显示时间选择器和阈值输入框
- 时间选择器列出该路径的所有可用扫描时间戳
- 选择 from/to 后，后台自动执行 delta 查询并计算 diff
- 筛选完成后文件树增加 delta 列，正 delta 红色，负 delta 绿色
- 阈值筛选器仅显示 `|delta| >= threshold` 的节点
- 清除按钮重置所有筛选，回到纯 ncdu 模式
- 筛选栏为空时 delta 列隐藏

### 7.4 扫描与进度

- 按 `s` 直接扫描当前树根（不再弹路径输入框）
- TUI 中手动触发扫描时，状态栏显示进度百分比
- 扫描可被 `Esc` 取消
- 取消后回到之前的状态（FS 树或 size overlay 状态）

### 7.5 元数据显示

- 光标移动到文件/目录时，元数据面板更新
- 显示路径、大小、增量、文件数、修改时间

### 7.6 删除交互

- 在文件上按 `d` 触发删除确认弹窗
- 确认后调用系统废纸篓（`trash` crate 或 shell 命令）
- 取消后返回浏览状态

### 7.7 测试适配指南

由于文件树从"快照驱动"改为"FS 层 + SQLite 扫描历史"双源驱动，测试需覆盖：

**状态逻辑测试**：
- `test_load_from_db`：从 SQLite 正确填充 scan_cache 和 available_snapshots
- `test_rebuild_tree_from_cache`：scan_cache 命中时返回带 size overlay 的 FS 树
- `test_rebuild_tree_from_fs`：scan_cache 未命中时返回 list_dir 浅层树
- `test_navigate_up_changes_root`：导航到父目录后根路径更新
- `test_scan_complete_updates_cache`：扫描完成后 scan_cache 更新 + 树刷新

**组件渲染测试**：
- `test_file_always_shows_size`：文件始终显示真实大小
- `test_unscanned_dir_shows_dash`：未扫描目录 size 显示 `"-"`
- `test_scanned_dir_shows_aggregated_size`：已扫描目录显示汇总大小
- `test_empty_filter_bar_hides_delta`：筛选栏空时无 delta 列

**事件处理测试**：
- `test_key_s_starts_scan`：按 s 不弹窗，直接触发扫描
- `test_key_h_on_root_navigates_up`：根节点按 h 改变树根
- `test_key_l_expands_with_cache`：按 l 展开时使用 scan_cache 或 list_dir

**端到端测试**：
- `test_startup_with_no_snapshots`：无快照启动 → 显示 cwd FS 树
- `test_startup_with_snapshots`：有快照 → 显示带 size overlay 的 FS 树
- `test_scan_then_navigate_workflow`：扫描后能正常展开/导航/展示 size

## 8. 安全注意事项

- 受保护路径（系统黑名单，见 `07-safety.md`）即使在 TUI 中按下 `d` 也不触发删除流程，仅显示"受保护路径，无法删除"提示
- 废纸篓操作使用 `trash` crate，不直接 `remove_dir_all`
- 所有删除操作需要二次确认，默认光标停在"取消"上

## 9. 已知边界

| 场景 | 行为 |
|------|------|
| 无快照（首次启动） | 以 cwd 为根展示 FS 树（目录 size=`"-"`，文件有真实 size）。按 `s` 扫描后刷新 size overlay |
| 仅有一个快照 | 加载到 scan_cache，目录显示真实汇总 size（无 delta）；筛选栏列出一个时间戳，无法选 from/to |
| 快照版本不兼容 | 跳过该快照，不加载；提示用户重新扫描 |
| 导航到无快照的目录 | 调用 list_dir 惰性读取，展示 FS 树 |
| 导航到有快照的目录 | 从 scan_cache 读取可用的 size overlay，并保持当前 FS 结构 |
| 扫描百万级目录 | 显示进度，可取消，不阻塞 UI |
| 终端窗口 resize | 自动重排布局（ratatui 自动处理） |
| 超大 delta 值 | `i64` 范围，格式化使用 `format_size` |
| 多个路径的快照 | 全量加载到 scan_cache；筛选栏只展示当前树根路径的历史 |
