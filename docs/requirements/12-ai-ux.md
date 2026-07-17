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
[浏览模式] → 选择路径 → 触发 AI → 查看结果 → 执行/关闭
                 │            │          │
             光标/a      弹出窗         j/k 浏览
             Tab 多选/A  mock/真实 AI   d 标记删除
                                       Enter 确认/关闭
                                       Esc 关闭
```

### 2.1 触发方式

| 操作 | 作用域 | 行为 |
|------|--------|------|
| `a` | 当前光标项 | 分析单个路径 |
| `A` | 多选模式下所有已选 | 分析批量路径 |
| `:ai` | 命令面板 | 分析当前光标或已选路径（进阶入口） |

不在多选模式时 `a` 分析单条；在多选模式时 `A` 分析所有已选，`a` 仍分析单条。

### 2.2 AI 分析中状态

弹窗顶部显示 "Analyzing 3 paths..." + 旋转动画（复用 scan spinner 样式）。
分析完成前用户可按 `Esc` 取消，回到浏览模式。

### 2.3 结果浏览

分析完成后进入 `AiReview` 模式，全屏弹窗列出每条路径的分析卡片：

```
┌───────────── AI Analysis ───────────────────────┐
│                                                   │
│  Selected 3 paths, total 4.7 GB                  │
│                                                   │
│  ○ target/                               3.2 GB  │  ← 绿色 (Safe)
│    Rust build artifacts                          │
│    Build cache, safe to delete, will rebuild      │
│    [d] Mark delete                               │
│                                                   │
│  ● node_modules/                         423 MB  │  ← 黄色 (Caution)
│    Node.js dependencies                          │
│    Can reinstall via npm install                  │
│    [d] Unmark                                    │
│                                                   │
│  ○ Downloads/secret.zip                 1.5 GB   │  ← 红色 (High)
│    Unknown purpose                               │
│    Cannot determine — review manually             │
│    [d] Mark delete                               │
│                                                   │
│  ─────────────────────────────────────────────    │
│  Marked: 1 item (423 MB)                         │
│  [Enter] Confirm  [Esc] Close                    │
└──────────────────────────────────────────────────┘
```

### 2.4 按键绑定（AiReview 模式）

| 键 | 作用 |
|----|------|
| `j`/`k` | 上下移动光标切换路径 |
| `d` | 标记/取消标记当前路径待删除 |
| `Enter` | 确认删除所有标记的路径 |
| `Esc` | 关闭面板，不执行任何操作 |

### 2.5 状态栏变化

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
          │  ├─ cursor: usize │
          │  └─ marked: Set   │
          └───────────────────┘
```

### 3.1 数据结构

```rust
pub struct AiReviewState {
    pub results: Vec<AiPathVerdict>,
    pub cursor: usize,
    pub mark_for_delete: HashSet<usize>,
    pub status: AiStatus,
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

### Phase 2: 真实 AI 接入

- Ollama / OpenAI 接口调用
- SQLite 持久化缓存
- 批量分析（多条路径一次请求）
- 真实删除操作

### Phase 3: 增强

- `:ai config` 命令配置 AI Provider
- Token 消耗统计
- 隐私警告（云端模式）
- 分析进度流式显示