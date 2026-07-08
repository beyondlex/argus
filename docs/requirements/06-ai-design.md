# AI 功能设计

## 1. 设计原则

### 1.1 AI 非必选

- AI 功能**默认关闭**。
- 用户无需配置 API 即可完美使用传统的静态扫描与时间 Diff 功能。
- 仅在用户主动配置了 `api_url` 和 `api_key` 且 `ai.enabled = true` 时激活。

### 1.2 术语统一

- CLI 命令名：`argus explain`（Phase 1 仅打印 Prompt，不调用 API）
- 配置快捷键名：`ai_diagnose`（映射到键盘 `a` 键）
- 内部模块：`ai_feature.rs`
- 三者均指向同一"AI 诊断"功能。对外文档统一使用 **AI 诊断** 作为功能名。

### 1.3 隐私红线

AI **绝不**扫描或上传用户的文件具体内容，仅上传结构化元数据：
- 目录绝对路径（如 `~/.cache/pypoetry/`）
- 目录名
- 文件类型后缀及其分布比例
- Top 5 最大文件/增长最快文件的文件名和大小
- 修改时间信息

## 2. 数字指纹特征提取 (Feature Extraction)

当用户在 TUI 中选中某一目录时，Rust 后端异步提取该目录的"数字指纹"：

```rust
pub struct AiContext {
    pub target_path: String,
    pub size_delta_mb: f64,
    pub current_size_mb: f64,
    pub top_large_files: Vec<(String, u64)>,
    pub primary_extensions: Vec<(String, f32)>,  // 后缀名及占比
}
```

## 3. AI 智能诊断场景

### 3.1 场景一：暴涨目录根因分析

用户发现某个目录体积在短时间内暴涨，按下 `a` 键触发 AI 诊断。

**自动组装 Context 示例**：

```
【系统环境】: OS: macOS (M4), Shell: zsh
【当前路径】: /var/log/nginx/
【暴涨现象】: 过去 2 小时，从 10MB → 4.2GB (增长 > 4000%)
【目录采样】:
  - access.log (+4.1GB)
  - error.log (+10MB)
```

**AI 输出**：根因分析 + 治理建议（如配置 fail2ban 规则、限流配置等）。

### 3.2 场景二：未知目录百科查询

用户光标停在某个不认识目录上，右侧 AI 观察窗自动展示卡片式信息：

```
📦 来源实体：Docker 桌面端缓存 (Buildx)
🔍 用途解释：用于加速 Docker 镜像的多阶段构建
⚠️ 删除影响：安全。删除不会导致容器丢失，
   但下次 docker build 需重新下载依赖
💡 Argus 建议：该目录 6 个月未变动，体积 12GB，
   强烈建议一键清理
```

### 3.3 场景三：大文件/久未更新文件的治理建议

- AI 可识别长时间未修改但占用大量空间的文件（如实验遗留数据）。
- 结合文件修改时间和项目活跃度，给出"删除/归档/保留"的个性化建议。

## 4. 批量分析与结果映射

### 4.1 为什么需要批量

TUI 文件树中可能有多条用户关心的目录（如所有暴涨 `+100MB` 的目录）。逐条发送不仅慢（N 次往返），且 Token 浪费在重复的系统提示上。批量分析将 N 条压缩为 1 次请求。

### 4.2 数据流

```
用户选中一批目录 / 光标在暴涨列表间移动
       │
       v
argus-core: 为每条路径提取 AiContext（数字指纹）
       │
       v
argus-core: 组装批量 Prompt（见 §4.4）
       │
       v
AI API: 返回结构化或编号化响应
       │
       v
argus-core: ResponseParser 提取 path → AiResult 映射
       │
       v
AiCache: 写入内存缓存 (HashMap<PathBuf, AiResult>)
       │
       v
TUI 渲染: 根据当前光标路径从 AiCache 查找，展示对应结果
```

### 4.3 接口定义

参见 `08-data-model.md` §2.3-2.4 的 `AiContext`、`AiResult`、`AiCache`、`batch_analyze`。

### 4.4 批量 Prompt 模板

**JSON 模式模板**（模型支持 response_format=json_object 时）：

```text
你是一个精通 macOS/Linux 系统运作和现代软件架构的磁盘清理专家。
请分析以下目录，对每个目录给出分析结论。
严格按照以下 JSON 格式返回（键为目录完整路径）：

{
  "<target_path>": {
    "label": "来源实体名",
    "description": "用途说明",
    "risk_level": "safe | low | medium | high",
    "suggestion": "治理建议",
    "deletable": true,
    "confidence": 0.9
  }
}

目录列表：
{batch_contexts}
```

**编号索引模式模板**（通用文本模型）：

```text
你是一个精通 macOS/Linux 系统运作和现代软件架构的磁盘清理专家。
以下是一批需要分析的目录，每个目录标有编号 [N]。
请依次分析每个目录，在每行开头用 [N] 标记对应编号。

目录列表：
[1] 路径: /var/log/nginx/
    当前体积: 4200 MB
    净增长: +4190 MB
    大文件采样: [("access.log", 4100MB), ("error.log", 90MB)]

[2] 路径: ~/Library/Caches/pip/
    当前体积: 2400 MB
    净增长: +800 MB
    大文件采样: [("wheels/*.whl", 2000MB), ("http/*.whl", 400MB)]

请按以下格式回复：
[N] 来源实体：xxx
    用途：xxx
    风险等级：safe/low/medium/high
    建议：xxx
```

### 4.5 响应解析器

```rust
/// 尝试 JSON 解析 → 回退编号解析 → 回退逐条
pub fn parse_batch_response(
    raw: &str,
    expected_paths: &[PathBuf],
    strategy: MappingStrategy,
) -> HashMap<PathBuf, AiResult> {
    match strategy {
        MappingStrategy::Json => try_parse_json(raw, expected_paths)
            .unwrap_or_else(|| fallback_parse_indexed(raw, expected_paths)),
        MappingStrategy::Indexed => parse_indexed(raw, expected_paths)
            .unwrap_or_else(|| fallback_sequential(expected_paths)),
        MappingStrategy::Sequential => unreachable!(), // 逐条发送不涉及解析
    }
}

fn try_parse_json(raw: &str, paths: &[PathBuf]) -> Option<HashMap<PathBuf, AiResult>>;
fn parse_indexed(raw: &str, paths: &[PathBuf]) -> Option<HashMap<PathBuf, AiResult>>;
fn fallback_sequential(paths: &[PathBuf]) -> HashMap<PathBuf, AiResult>; // 返回空 map，调用方需逐条重试
```

### 4.6 缓存策略

| 维度 | 规则 |
|------|------|
| 缓存粒度 | 每路径一条 `AiResult` |
| 数据结构 | `HashMap<PathBuf, AiResult>`（内存态，与 DiffTree 同级） |
| 失效时机 | 用户退出客户端 / 手动刷新 / 新扫描产生新 DiffTree |
| 不过期 | 会话内不自动过期（用户短时间内反复查看同一目录不重复请求） |
| 不持久化 | AI 模型版本迭代后旧结论可能不准确，不做磁盘持久化 |

## 5. 单目录 Prompt 模板（回退/单条场景）

当 JSON 模式和编号索引均不可用时（如模型不支持），回退到单条发送：

```text
你是一个精通 macOS/Linux 系统运作和现代软件架构的磁盘清理专家。
当前用户发现以下目录在短时间内体积暴涨：

- 路径: {target_path}
- 当前体积: {current_size_mb:.2} MB
- 净增长量: {size_delta_mb:.2} MB
- 目录下大文件采样: {top_large_files:?}

请简明扼要地告诉用户：
1. 这是什么软件或系统进程产生的？有什么用？
2. 能不能删？直接执行删除（或清空）会有什么安全后患
   或对软件性能有什么影响？
3. 给出治理建议（保留、放心一键删除、清空部分日志等）。
```

## 6. Token 消耗统计 (Token Metric Tracker)

### 6.1 需求背景

用户对大模型 API 费用和用量高度敏感，系统必须内置 Token 统计机制。

### 6.2 实现需求

| 阶段 | 功能 | 说明 |
|------|------|------|
| 请求前预估 | 根据 Prompt 字符长度进行轻量级 Token 预估 | 在界面提示"预计消耗 ~300 tokens" |
| 请求后计量 | 解析大模型返回体的 `usage` 字段 | 精确记录 `prompt_tokens` + `completion_tokens` |
| 本地持久化 | Token 消耗数据写入本地数据库 | 支持历史查询 |

**Token 预估算法**：不同模型 tokenizer 差异大（cl100k_base / p50k_base / llama 等），不做精确计算。Phase 4 使用粗略启发式：`estimated = prompt.chars().count() / 4`（按英文约 4 字符/token 估算）。界面上显示 `~{estimated}` 前缀以示为估计值。

```rust
/// 粗略估算 prompt 的 Token 数（4 字符 ≈ 1 token，中英文混合取 2 字符 ≈ 1 token）
/// 不依赖 tokenizer 库，仅用于 UI 提示
pub fn estimate_tokens(prompt: &str) -> usize {
    let char_count = prompt.chars().count();
    // 中英文混合场景取保守值 3 字符/token
    (char_count + 2) / 3
}
```

> `// FUTURE: 使用 tiktoken-rs 或 tokenizers 库做精确估算`

### 6.3 仪表盘展示

在配置页或 AI 观察窗底部展示：
- 单次消耗
- 本日累计
- 历史总计消耗
- 每日 Token 上限（如有配置）

## 7. API 兼容性

- 兼容所有提供 OpenAI 格式接口的服务（OpenAI、Azure OpenAI、本地 ollama、vLLM 等）。
- 用户只需配置 `api_url` 和 `api_key` 即可切换任意后端。
