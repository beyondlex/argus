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

**不属于 Phase 2**：Daemon IPC 接入、真实 AI API 调用、配置修改命令、审计查看命令。这些留待 Phase 3/4。

### 独立模式路径语义

独立模式下，TUI 展示以某个扫描根路径为根的文件树。不同于旧模型的"路径选择器"：

| 方面 | 旧模型（废弃） | 新模型 |
|------|--------------|--------|
| 启动流程 | 检查快照状态 → Fresh/Single/Dual 三态 → 路径选择器 | 直接展示上次扫描路径的树（或提示按 `s` 扫描）；无路径选择器 |
| 路径切换 | 通过路径选择器切换 | 按 `s` 重新扫描其他路径 |
| Delta 控制 | 自动根据快照数量决定是否有 delta | 顶部筛选栏控制；无时间选择时纯 ncdu |
| 多路径快照 | 混乱堆积 | 仅用于时间选择器提供可用的时间戳列表 |

> **为什么移除路径选择器**：路径选择器对用户来说"有点怪，不知要选哪个"。快照文件随着扫描增多越来越乱。新模型让用户始终专注于一个树状视图——从 `/` 或 `~` 开始，delta 是该树上的可选覆盖层。

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
serde_json = "1.0"
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

TUI 启动时始终显示文件树。delta 是树上的可选的筛选覆盖层：

```
┌──────┐
│ 启动 │
└──┬───┘
   │
   ├── 有快照 ──▶ 加载最近扫描的快照 ──▶ 渲染文件树（纯 size，无 delta）
   │                                      │
   │                                      ├── 用户无操作 → 默认 ncdu 模式
   │                                      │
   │                                      └── 用户在筛选栏选择时间范围
   │                                           │
   │                                           ├── 有该路径的两个快照 → 后台 diff → 显示 delta
   │                                           └── 缺少快照 → 提示"需再次扫描此路径以对比"
   │
   └── 无快照 ──▶ 提示"按 s 扫描目录" ──▶ 用户按 s
                                              │
                                              ├── 默认扫描当前 cwd
                                              └── 或输入自定义路径
                                                   │
                                                   └── 扫描(tokio task) → 渲染文件树
```

**核心原则**：

1. **没有路径选择器**：首次使用提示扫描 → 扫描后树根即该路径。之后每次启动自动加载该路径的最新快照。按 `s` 可重新扫描（同路径或新路径）。

2. **Delta 始终可选**：筛选栏的时间选择器列出该路径下可用的快照时间戳。Phase 2 仅支持选择两个时间点（from/to 都选）做 diff，不支持单点或范围。用户选择后触发后台 `compare_trees` 任务。

3. **无快照时不报错**：降级为 ncdu 模式，仅显示当前扫描结果。等用户再次扫描获得第二个快照后，筛选栏自然激活。

4. **筛选栏状态**：时间选择器为空 → 无 delta 列。时间选择器有值但快照不够 → 灰色不可用状态，提示原因。

### 2.4 数据流

```rust
// 后台 → UI 的消息类型
enum AppMessage {
    ScanProgress { file_count: u64, total_bytes: u64 },
    ScanComplete(Snapshot),
    DiffComplete(DiffNode),
    Error(String),
}
```

```rust
// UI → 后台的请求
enum AppCommand {
    Scan { path: PathBuf, cancel: Arc<AtomicBool> },
    Diff { old: Snapshot, new: Snapshot },
}
```

**筛选状态与 diff 的关系**：筛选栏状态存储在 `AppState` 中（`filter_from: Option<DateTime>`, `filter_to: Option<DateTime>`, `filter_threshold: Option<u64>`）。当筛选状态改变：

```
筛选栏时间变化
  → 检查该路径下是否有对应时间点的快照
  → 无 → 灰显 + 提示
  → 有 → 记录 (from_hash, to_hash)
  → 后台加载两个 Snapshot → compare_trees → 返回 DiffNode
  → 覆盖到当前树显示
  → 用户清除筛选 → 清除 DiffNode → 回到纯 ncdu 模式
```

## 3. 组件实现说明

### 3.1 FileTree 组件

**数据结构**：将 `DiffNode` 展平为可滚动的行列表。每个行节点包含：

```rust
struct TreeLine {
    depth: usize,         // 缩进层级
    node: DiffNode,       // 引用或 Arc
    expanded: bool,       // 目录是否展开
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
- Phase 2 仅支持按增量排序（`o` 键切换，忽略目录内部的递归聚合计，仅对顶层子节点排序）

```rust
#[derive(PartialEq)]
enum SortMode {
    Name,
    Delta,  // Phase 2 默认
    Size,   // Phase 2 可选
}
```

**行数优化**：树的叶子节点可能数十万行。FileTree 应使用虚拟滚动（只渲染视口行数 + 上下缓冲区各 10 行）。`ratatui` 通过 `List` + `Scrollbar` 实现，无需自定义虚拟滚动。

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
- `Enter` 激活区域：from/to 展开可用快照时间列表供选择；threshold 进入数值输入
- `Esc` 或选择后关闭区域
- 清除按钮重置所有筛选状态
- 选择完成后触发后台加载快照 + diff

**Phase 2 仅支持"固定快照列表"选择**：可用时间列表源于 `~/.config/argus/snapshots/` 中当前路径的快照文件名解析。用户从列出的时间戳中选择 from 和 to，TUI 加载对应的 JSON 快照并计算 diff。

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
第 1 步：argus-tui 脚手架（Cargo.toml + main.rs 事件循环 + App skeleton）
第 2 步：config.rs — TUI 配置加载
第 3 步：FileTree 组件 — 树展平 + 渲染 + 滚动 + 展开/收起
第 4 步：handler.rs — 按键绑定 + 光标移动 + 目录导航
第 5 步：FilterBar 组件 — 筛选栏渲染 + 可用快照时间戳列表选择
第 6 步：Metadata 面板 — 选中节点详情
第 7 步：Status Bar — 状态信息 + 进度显示
第 8 步：后台任务 — 扫描 + diff 异步化 + 筛选触发 diff
第 9 步：删除交互 — DeletePrompt 模式 + 二次确认
第 10 步：Help Popup — 帮助面板（P1）
第 11 步：手动验收测试
```

## 6. 验收标准

### 6.1 启动与布局

```bash
cargo run -p argus-tui
```

- 启动后加载 `~/.config/argus/config.toml`（不存在时使用默认值）
- 检查 `~/.config/argus/snapshots/` 下的快照文件：
  - **有快照**：加载最近扫描路径的最新快照，显示文件树（无 delta 列）
  - **无快照**：显示"按 `s` 扫描目录"提示
- 展示四栏布局：筛选栏 | 文件树（左70%）| 元数据（右30%）| 底部状态栏

### 6.2 文件树浏览

```bash
# 创建测试快照
cargo run -p argus-cli -- scan --path ~/Downloads
cargo run -p argus-tui
```

- 文件树正确显示目录层级
- `j`/`k` 上下移动光标
- `h`/`l` 展开/收起目录
- 默认无 delta 列（纯 ncdu 模式）
- 按 `s` 可以重新扫描

### 6.3 筛选栏与 Delta 展示

```bash
# 创建第二个快照（在 ~/Downloads 中制造 50MB 变动后）
cargo run -p argus-cli -- scan --path ~/Downloads
cargo run -p argus-tui
```

- 筛选栏显示时间选择器和阈值输入框
- 时间选择器列出该路径的所有可用快照时间戳
- 选择 from/to 后，后台自动加载快照并计算 diff
- 筛选完成后文件树增加 delta 列，正 delta 红色，负 delta 绿色
- 阈值筛选器仅显示 `|delta| >= threshold` 的节点
- 清除按钮重置所有筛选，回到纯 ncdu 模式
- 筛选栏为空时 delta 列隐藏

### 6.4 扫描与进度

- TUI 中手动触发扫描时，状态栏显示进度百分比
- 扫描可被 `Esc` 取消
- 取消后回到之前的状态

### 6.5 元数据显示

- 光标移动到文件/目录时，元数据面板更新
- 显示路径、大小、增量、文件数、修改时间

### 6.6 删除交互

- 在文件上按 `d` 触发删除确认弹窗
- 确认后调用系统废纸篓（`trash` crate 或 shell 命令）
- 取消后返回浏览状态

## 7. 安全注意事项

- 受保护路径（系统黑名单，见 `07-safety.md`）即使在 TUI 中按下 `d` 也不触发删除流程，仅显示"受保护路径，无法删除"提示
- 废纸篓操作使用 `trash` crate，不直接 `remove_dir_all`
- 所有删除操作需要二次确认，默认光标停在"取消"上

## 8. 已知边界

| 场景 | 行为 |
|------|------|
| 无快照文件 | 提示按 `s` 扫描；扫描后渲染文件树（无 delta） |
| 仅有一个快照 | 加载该快照，树显示纯 size（无 delta）；筛选栏时间选择器列出该路径可用的快照（仅一个时无法选 from/to，灰显提示"需再次扫描"） |
| 快照版本不兼容 | 提示重新扫描，不崩溃 |
| 扫描百万级目录 | 显示进度，可取消，不阻塞 UI |
| 终端窗口 resize | 自动重排布局（ratatui 自动处理） |
| 超大 delta 值 | `i64` 范围，格式化使用 `format_size` |
| 多个路径的快照 | 仅加载最近扫描路径的树；其他路径的快照不干扰当前视图。按 `s` 重新扫描其他路径来切换 |
| 多个路径的快照 | 仅加载最近扫描的路径；其他路径的快照不干扰当前视图；通过 `s` 重新扫描其他路径来切换 |
