# AI 路径分析交互设计

## 1. 概述

用户在浏览过程中遇到不认识的路径（文件/目录），选中后触发 AI 分析，获得：
- 这是什么（来源实体名）
- 有什么用（用途说明）
- 能不能删（风险等级 + 建议）
- 能释放多少空间

分析结果可缓存到 SQLite，同一路径下次直接读取，不重复请求 AI。

## 2. 交互流程

```
[浏览模式] → 选择路径 → 触发 AI → 加载中 → 查看结果/错误 → 执行/关闭
                 │             │          │            │
             光标/a        弹出窗      3s mock     j/k 浏览
             Tab 多选/A   (loading)   分析完成     d/D 删除确认
                                            随机错误   y/n 确认/取消
                                              Esc 关闭
```

### 2.1 触发方式

| 操作 | 作用域 | 行为 |
|------|--------|------|
| `a` | 当前光标项 | 分析单个路径 |
| `A` | 多选模式下所有已选 | 分析批量路径 |
| `:ai` | 命令面板 | 分析当前光标或已选路径（进阶入口） |

不在多选模式时 `a` 分析单条；在多选模式时 `A` 分析所有已选，`a` 仍分析单条。

### 2.2 AI 分析中状态（Loading）

弹窗顶部显示 `AI Analysis (loading...)`，列表区逐行列出待分析路径名，summary 显示 `Selected N path(s), total X`（X 来自 scan cache 的实际大小，目录含递归子项）。

```
┌────────── AI Analysis (loading...) ───────────────┐
│                                                    │
│  Selected 3 paths, total 4.7 GB                    │
│                                                    │
│    target/                                         │
│    node_modules/                                   │
│    Downloads/secret.zip                            │
│                                                    │
│  j/k Navigate  Space Mark/Unmark  d Delete  Esc    │
└────────────────────────────────────────────────────┘
```

分析完成前用户可按 `Esc` 取消，回到浏览模式。

### 2.3 分析结果（Ready）

分析完成后进入 `Ready` 状态，列出每条路径的分析卡片：

```
┌────────────── AI Analysis ─────────────────────────┐
│                                                    │
│  Selected 3 paths, total 4.7 GB                    │
│                                                    │
│  ○ target/                               3.2 GB   │  ← 绿色 (Safe)
│    Rust build artifacts                            │
│    Build cache, safe to delete, will rebuild        │
│                                                    │
│  ● node_modules/                         423 MB    │  ← 黄色 (Caution)
│    Node.js dependencies                            │
│    Can reinstall via npm install                    │
│                                                    │
│  ○ Downloads/secret.zip                  1.5 GB    │  ← 红色 (High)
│    Unknown purpose                                  │
│    Cannot determine — review manually                │
│                                                    │
│  ─────────────────────────────────────────────      │
│  Marked: 1 item (423 MB)                            │
│  j/k Navigate  Space Mark/Unmark  d Delete  Esc     │
└────────────────────────────────────────────────────┘
```

summary 行 total 始终使用 scan cache 实际大小（与 loading 一致），不依赖 `std::fs::metadata`。

### 2.4 错误状态（Error）

分析失败时显示 `AI Analysis (error)` 标题 + 红色错误详情 + 关闭提示。

```
┌──────────── AI Analysis (error) ───────────────────┐
│                                                    │
│  Selected 3 paths, total 4.7 GB                    │
│                                                    │
│          Analysis failed                           │
│                                                    │
│    AI SDK not configured: missing API key or       │
│    model endpoint                                  │
│                                                    │
│          Press Esc/q to close                      │
│                                                    │
│  j/k Navigate  Space Mark/Unmark  d Delete  Esc    │
└────────────────────────────────────────────────────┘
```

Phase 1 模拟 4 种错误（随机出现，每次不同）：
- AI SDK 未配置（30%）
- 网络超时（20%）
- API 返回错误（20%）
- 数据解析失败（20%）
- 正常结果（10%）

### 2.5 删除确认弹窗

标记待删项后按 `d`（普通删除）或 `D`（永久删除），在 AI Analysis 弹窗上叠加确认弹窗（不切 mode，背景保留）。

```
┌────────────── AI Analysis ─────────────────────────┐
│                                                    │
│  ┌────── Delete 3 items? ────────┐                │
│  │                              │                │
│  │        WARNING:              │                │
│  │                              │                │
│  │  3 items selected for        │                │
│  │  deletion                    │                │
│  │                              │                │
│  │  This will move all selected │                │
│  │  items to trash.             │                │
│  │                              │                │
│  │    [y] Confirm delete        │                │
│  │    [n] Cancel                │                │
│  └──────────────────────────────┘                │
│                                                    │
│  j/k Navigate  Space Mark/Unmark  d Delete  Esc    │
└────────────────────────────────────────────────────┘
```

`d` 键仅在 `Ready` 状态且至少标记一项时生效。`y` 确认后清空标记并显示 Phase 1 提示消息，`n`/`Esc` 取消返回。

### 2.6 按键绑定（AiReview 模式）

| 键 | 作用 |
|----|------|
| `j`/`k` | 上下移动光标切换路径 |
| `Space` | 标记/取消标记当前路径待删除 |
| `d` | 弹出确认弹窗（普通删除） |
| `D` | 弹出确认弹窗（永久删除） |
| `y`/`n` | 确认弹窗中确认/取消 |
| `Esc` | 关闭面板 / 取消确认弹窗 |
| `q` | 关闭面板 |

### 2.7 状态栏变化

AiReview 模式下，状态栏左端显示 ` AI REVIEW ` 标签，右侧隐藏排序/筛选信息。

## 3. 数据流

```
           ┌───────────────────┐
           │   AiCache (内存)   │ ← HashMap<PathBuf, AiPathVerdict>
           └────────┬──────────┘
                    │
           ┌────────v──────────┐
           │   AiReviewState    │ ← 当前弹窗的完整状态
           │  ├─ results: Vec  │
           │  ├─ pending_paths │
           │  ├─ pending_size  │
           │  ├─ cursor: usize │
           │  ├─ marked: Set   │
           │  └─ delete_confirm│
           └───────────────────┘
```

分析在线程中计算（`std::thread::spawn`），完成后通过 `AppMessage::AiAnalysisComplete` 发回主线程。
错误通过 `AppMessage::AiAnalysisError` 传递。

### 3.1 数据结构

```rust
pub struct AiReviewState {
    pub results: Vec<AiPathVerdict>,
    pub pending_paths: Vec<PathBuf>,          // 待分析的路径（loading/error 时展示）
    pub pending_total_size: u64,              // 路径总大小（来自 scan cache）
    pub cursor: usize,
    pub mark_for_delete: HashSet<usize>,
    pub status: AiStatus,
    pub delete_confirm: Option<(Vec<PathBuf>, bool)>,  // 删除确认弹窗状态
}

pub struct AiPathVerdict {
    pub path: PathBuf,
    pub size: u64,
    pub label: String,        // "node_modules", "build cache" …
    pub purpose: String,      // 一句话用途说明
    pub risk_level: RiskLevel,
    pub suggestion: String,   // 治理建议
    pub deletable: bool,      // 是否建议删除
}

pub enum RiskLevel { Safe, Low, Medium, High }
pub enum AiStatus { Idle, Loading, Ready, Error(String) }

pub enum AppMessage {
    // ...
    AiAnalysisComplete(Vec<AiPathVerdict>),
    AiAnalysisError(String),
}
```

### 3.2 缓存策略

| 阶段 | 缓存 | 说明 |
|------|------|------|
| Phase 1 | 纯内存（不持久化） | 会话内 HashMap 缓存 |
| Phase 2 | SQLite 持久化 | 按路径 + 时间戳存储，下次直接读取 |
| Phase 3 | 增量失效 | 扫描/删除操作后清除受影响路径的缓存 |

### 3.3 隐私安全

AI 仅分析路径名 + 文件类型 + 大小，不读取文件内容。
用户可配置是否使用本地 AI（默认）或云端 API。

## 4. 未来阶段

### Phase 1: UI 骨架（当前）

- `a`/`A` 触发弹窗
- Mock 数据（基于路径名的启发式规则）
- 纯 UI 交互，无真实 AI 调用，无真实删除
- 仅内存缓存
- 3s mock 延迟模拟加载
- 随机错误模拟（4 种错误类型）
- 删除确认弹窗叠加在 AI Analysis 内
- pending_total_size 来自 scan cache 实际大小

### Phase 2: 真实 AI 接入

- Ollama / OpenAI 接口调用
- SQLite 持久化缓存
- 批量分析（多条路径一次请求）
- 真实删除操作
- 移除 3s mock 延迟和随机错误模拟

### Phase 3: 增强

- `:ai config` 命令配置 AI Provider
- Token 消耗统计
- 隐私警告（云端模式）
- 分析进度流式显示