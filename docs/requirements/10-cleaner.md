# 磁盘清理模块设计 (Cleaner)

## 1. 目标

为 Argus 增加类似 mole 的磁盘清理能力：自动发现可清理的缓存/日志/临时文件、智能卸载应用及残留、扫描项目构建产物。坚持"安全第一"——所有删除操作走废纸篓 + 风险分级。

## 2. 架构决策

### 2.1 模块位置：`argus-core`，feature-gated

```
argus-core/Cargo.toml
    [features]
    cleanup = ["dep:trash"]
```

与 `ai` 模块相同的 feature gate 模式。核心逻辑全在 core，所有客户端复用。

### 2.2 不独立 crate

- CLI/TUI/GUI 都需要清理能力，放在 core 避免三端重复实现
- 不依赖 tokio/async（clean 操作是同步的）

### 2.3 `trash` 移入 core

目前 `trash` 只在 TUI 依赖，但 CLI 也需要。移到 core 的 `cleanup` feature 下。

## 3. 模块设计

### 3.1 `cleaner/mod.rs`

模块入口，重导出所有公共类型。结构：

```rust
pub mod audit;
pub mod categories;
pub mod cleaner;
pub mod purge;
pub mod safety;
pub mod uninstaller;
```

### 3.2 `cleaner/safety.rs` — 受保护路径与风险判定

将 `docs/requirements/07-safety.md` §2.1 的黑名单落地到代码。

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Safe,
    Low,
    Medium,
    High,
}

pub fn is_protected(path: &Path) -> bool;
pub fn classify_risk(path: &Path) -> RiskLevel;
pub fn check_deletion_allowed(path: &Path) -> Result<(), CleanerError>;
```

- macOS/Linux 各一套硬编码黑名单（`#[cfg]` 隔离）
- 动态规则：`/Users/<user>/Library/Caches` → Low，`/var/tmp` → Medium，系统目录 → High

### 3.3 `cleaner/categories.rs` — 已知清理目标

macOS 已知的缓存/日志/临时文件位置清单。每个目标有路径、描述、预估大小、风险等级。

```rust
pub struct CleanTarget {
    pub id: String,           // 唯一标识，如 "user-app-cache"
    pub label: String,        // 展示名，如 "User App Cache"
    pub paths: Vec<PathBuf>,  // 要扫描的路径列表
    pub risk: RiskLevel,      // 预置风险等级
    pub category: TargetCategory,
}

pub enum TargetCategory {
    AppCache,
    BrowserCache,
    DevTools,
    SystemLogs,
    TempFiles,
    Trash,
}

pub fn default_clean_targets() -> Vec<CleanTarget>;
pub fn scan_target_size(target: &CleanTarget) -> Result<u64, CleanerError>;
```

### 3.4 `cleaner/cleaner.rs` — 清理编排

dry-run → 筛选 → 执行 → 报告 标准流程。

```rust
pub struct CleanPlan {
    pub targets: Vec<CleanTarget>,
    pub total_bytes: u64,
    pub items: Vec<CleanItem>,
}

pub struct CleanItem {
    pub path: PathBuf,
    pub size: u64,
    pub risk: RiskLevel,
    pub target_id: String,
}

pub struct CleanReport {
    pub total_attempted: u64,
    pub total_succeeded: u64,
    pub total_failed: u64,
    pub freed_bytes: u64,
    pub errors: Vec<(PathBuf, String)>,
}

pub fn plan_clean(targets: &[CleanTarget]) -> Result<CleanPlan, CleanerError>;
pub fn dry_clean(targets: &[CleanTarget]) -> Result<CleanPlan, CleanerError>;
pub fn exec_clean(items: &[CleanItem], force: bool) -> Result<CleanReport, CleanerError>;
```

### 3.5 `cleaner/audit.rs` — 审计日志

所有删除操作记录到 `~/.config/argus/audit.log`（JSON lines 格式）。

```rust
pub struct AuditEntry {
    pub timestamp: DateTime<Utc>,
    pub operation: AuditOp,
    pub paths: Vec<PathBuf>,
    pub total_bytes: u64,
    pub success: bool,
    pub error: Option<String>,
}

pub enum AuditOp {
    Clean,
    Uninstall,
    Purge,
    Delete,
}

pub fn log_operation(entry: &AuditEntry) -> Result<(), CleanerError>;
pub fn read_audit_log(limit: usize) -> Result<Vec<AuditEntry>, CleanerError>;
```

### 3.6 `cleaner/uninstaller.rs` — App 发现与卸载

```rust
pub struct AppInfo {
    pub id: String,          // bundle id，如 "com.microsoft.VSCode"
    pub name: String,        // 展示名
    pub path: PathBuf,
    pub size: u64,
    pub last_used: Option<DateTime<Utc>>,
    pub is_from_app_store: bool,
}

pub struct AppLeftovers {
    pub app: AppInfo,
    pub leftover_paths: Vec<PathBuf>,
    pub total_leftover_bytes: u64,
}

pub fn find_installed_apps() -> Result<Vec<AppInfo>, CleanerError>;
pub fn find_leftovers(app: &AppInfo) -> Result<AppLeftovers, CleanerError>;
pub fn uninstall_app(app: &AppInfo, remove_leftovers: bool) -> Result<CleanReport, CleanerError>;
```

### 3.7 `cleaner/purge.rs` — 项目构建产物扫描

```rust
pub struct Artifact {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub size: u64,
    pub last_modified: DateTime<Utc>,
    pub project_name: String,
    pub age_days: u64,
}

pub enum ArtifactKind {
    NodeModules,
    Target,
    Build,
    Dist,
    Venv,
    NextCache,
    Terraform,
}

pub fn find_artifacts(roots: &[PathBuf]) -> Result<Vec<Artifact>, CleanerError>;
pub fn remove_artifacts(artifacts: &[Artifact]) -> Result<CleanReport, CleanerError>;
```

## 4. 与现有系统集成

### 4.1 CLI 新增子命令

```
argus clean             # 交互式选择清理项 (默认 dry-run 预览)
argus clean --dry-run   # 只报告不操作
argus clean --yes       # 跳过确认直接清理
argus uninstall         # 列出已安装 App → 选择 → 卸载 + 清理残留
argus uninstall --dry-run
argus purge             # 扫描项目构建产物
argus purge --paths ~/Projects ~/Work   # 指定搜索根
argus purge --dry-run
```

### 4.2 TUI 新增面板

新增 `AppMode::CleanupPanel`，展示分类清理项：
- 用现有列表渲染模式展示 `CleanTarget` 列表
- 勾选/取消（`Space`），确认（`Enter`）
- 复用已有的 `DeletePrompt`/`DeletePermanentPrompt` 组件
- 串联 AI 分析的路径直接可被 cleanup 使用

### 4.3 与安全文档对齐

- `cleaner/safety.rs` 是 `07-safety.md` §2.1/§2.2 的代码化
- 所有清理触发前先过 `check_deletion_allowed()`
- 审计日志写入 `~/.config/argus/audit.log`（§5）

## 5. 实施计划

| 步骤 | 模块 | 说明 |
|------|------|------|
| 1 | `safety.rs` | 受保护路径 + 风险等级 + 单元测试 |
| 2 | `categories.rs` | macOS 已知清理目标 + size 快速计算 |
| 3 | `cleaner.rs` | Plan → DryRun → Execute 编排 |
| 4 | `audit.rs` | 审计日志读写 |
| 5 | `uninstaller.rs` | App 发现 + 残留扫描 + 卸载 |
| 6 | `purge.rs` | 构建产物扫描 |
| 7 | CLI | 子命令接入 |
| 8 | TUI | 清理面板 |

## 6. 错误处理

```rust
#[derive(Debug, thiserror::Error)]
pub enum CleanerError {
    #[error("path is protected: {0}")]
    ProtectedPath(PathBuf),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("trash error: {0}")]
    Trash(#[from] trash::Error),
    #[error("audit log error: {0}")]
    Audit(String),
    #[error("{0}")]
    Other(String),
}
```
