# AI 功能设计

## 1. 设计原则

### 1.1 AI 非必选

- AI 功能**默认关闭**。
- 用户无需配置 API 即可完美使用传统的静态扫描与时间 Diff 功能。
- 仅在用户主动配置了 `api_url` 和 `api_key` 且 `ai.enabled = true` 时激活。

### 1.2 术语统一

- CLI 命令名：`argus explain`（Phase 1 仅打印 Prompt，不调用 API）
- 配置快捷键名：`ai_diagnose`（映射到键盘 `a` 键）
- 内部模块：`ai_feature.rs`（已实现为 `argus-core/src/ai.rs`）
- 三者均指向同一"AI 诊断"功能。对外文档统一使用 **AI 诊断** 作为功能名。

### 1.3 隐私红线

AI **绝不**扫描或上传用户的文件具体内容，仅上传结构化元数据：
- 目录绝对路径（如 `~/.cache/pypoetry/`）
- 目录名
- 文件类型后缀及其分布比例
- Top 5 最大文件/增长最快文件的文件名和大小
- 修改时间信息

### 1.4 多语言支持

AI 输出的所有文本字段（label_detail, description, suggestion）使用用户配置的语言。
默认 `en-US`，用户可通过 `[ai].language` 配置为 `zh-CN`、`ja-JP` 等。

## 2. 核心类型

### 2.1 AiContext（数字指纹）

```rust
pub struct AiContext {
    pub target_path: String,
    pub size_delta_mb: f64,
    pub current_size_mb: f64,
    pub top_large_files: Vec<(String, u64)>,
    pub primary_extensions: Vec<(String, f32)>,
}
```

### 2.2 AiResponse（统一 JSON 格式）

单文件与批量分析使用完全相同的 JSON 格式，区别仅在于键的数量：

```rust
pub struct AiResponse {
    pub label_detail: String,  // 具体来源实体名
    pub description: String,   // 用途说明
    pub risk_level: String,    // "safe" | "low" | "medium" | "high"
    pub suggestion: String,    // 治理建议
    pub deletable: bool,       // 是否建议删除
    pub confidence: f64,       // 置信度 0.0-1.0
}
```

## 3. 数据流

```
用户选择路径（单条或批量）
       │
       v
argus-core: 为每条路径提取 AiContext（数字指纹）
       │
       v
argus-core: build_prompt(contexts, language) → 统一 Prompt
       │
       v
AI API: 返回 JSON（格式固定，单文件单键，批量多键）
       │
       v
argus-core: try_parse_json(raw) → HashMap<Path, AiResponse>
       │
       ├── 成功 → AiCache → TUI 渲染
       │
       └── 失败 → 重试相同 batch（最多 2-3 次）
                    │
                    ├── 成功 → AiCache → TUI 渲染
                    │
                    └── 全部失败 → 报告错误给用户
```

## 4. Prompt 模板

### 统一 Prompt

单文件与批量使用同一模板，`{contexts}` 为 JSON 数组序列化：

```
You are a disk cleanup expert for macOS and Linux.
Respond in {language}. All text fields (label_detail, description, suggestion)
must be in {language}.

Analyze the following directories. Return a JSON object where keys are
directory paths and values have this exact schema:

{
  "<target_path>": {
    "label_detail": "specific source entity name",
    "description": "what this directory is used for",
    "risk_level": "safe|low|medium|high",
    "suggestion": "governance advice",
    "deletable": true,
    "confidence": 0.95
  }
}

The "label" field (program category like "build-artifacts", "package-dependencies")
is determined automatically by the program. Do NOT output a label field.
Only output label_detail as the specific source description.

Directories to analyze:
{batch_contexts}
```

## 5. 响应解析

### 5.1 解析策略

始终使用 JSON 解析。只有一种解析器：

```rust
/// Parse unified JSON response. Returns empty map on failure.
/// Caller decides whether to fall back to sequential retry.
pub fn try_parse_json(raw: &str) -> HashMap<String, AiResponse>;
```

### 5.2 重试策略

批量解析失败时：
- 重试相同 batch，最多 2-3 次
- 不拆成单条——JSON 格式与 batch 大小无关，单条不会比批量更稳定
- 全部重试失败后，向用户报告错误

### 5.3 分块策略（Chunking）

唯一需要拆分 batch 的场景是 Context 总 Token 数超过 `max_tokens_per_request`。
此时按 Context 数组分块，每块独立请求，每块仍是多键 JSON：

```rust
fn chunked_analyze(
    contexts: &[AiContext],
    language: &str,
    max_tokens: usize,
) -> HashMap<PathBuf, AiResponse> {
    let mut results = HashMap::new();
    for chunk in contexts.chunks(chunk_size(contexts, max_tokens)) {
        let prompt = build_prompt(chunk, language);
        let raw = call_api(&prompt);
        results.extend(try_parse_json(&raw));
    }
    results
}
```

## 6. 环境感知场景

### 6.1 场景一：暴涨目录根因分析

用户发现某个目录体积在短时间内暴涨，按下 `a` 键触发 AI 诊断。

**Context 示例**：
```
target_path: /var/log/nginx/
current_size_mb: 4200
size_delta_mb: 4190
top_large_files: [("access.log", 4100000000), ("error.log", 90000000)]
```

**AI 输出**（en-US）：
```json
{
  "/var/log/nginx/": {
    "label_detail": "Nginx HTTP access and error logs",
    "description": "Web server request and error log files",
    "risk_level": "safe",
    "suggestion": "Configure log rotation and consider fail2ban for aggressive crawling",
    "deletable": true,
    "confidence": 0.95
  }
}
```

### 6.2 场景二：未知目录百科查询

**AI 输出**（zh-CN，当 `language = "zh-CN"` 时）：
```json
{
  "~/.docker/buildx/": {
    "label_detail": "Docker 桌面端缓存 (Buildx)",
    "description": "用于加速 Docker 镜像的多阶段构建缓存",
    "risk_level": "safe",
    "suggestion": "安全。删除不会导致容器丢失，但下次构建需重新下载依赖",
    "deletable": true,
    "confidence": 0.9
  }
}
```

## 7. 缓存策略

| 维度 | 规则 |
|------|------|
| 缓存粒度 | 每路径一条 `AiResponse` |
| 数据结构 | `HashMap<PathBuf, AiResponse>`（内存态，与 DiffTree 同级） |
| 失效时机 | 用户退出客户端 / 手动刷新 / 新扫描产生新 DiffTree |
| 不过期 | 会话内不自动过期（用户短时间内反复查看同一目录不重复请求） |
| 不持久化 | AI 模型版本迭代后旧结论可能不准确，不做磁盘持久化 |

## 8. Token 消耗统计

### 8.1 需求背景

用户对大模型 API 费用和用量高度敏感，系统必须内置 Token 统计机制。

### 8.2 实现

```rust
/// 粗略估算 prompt 的 Token 数（3 字符 ≈ 1 token，中英文混合保守值）
/// 不依赖 tokenizer 库，仅用于 UI 提示
pub fn estimate_tokens(prompt: &str) -> usize {
    let char_count = prompt.chars().count();
    (char_count + 2) / 3
}
```

### 8.3 仪表盘展示

在配置页或 AI 观察窗底部展示：
- 单次消耗
- 本日累计
- 历史总计消耗
- 每日 Token 上限（如有配置）

## 9. API 兼容性

- 兼容所有提供 OpenAI 格式接口的服务（OpenAI、Azure OpenAI、本地 ollama、vLLM 等）。
- 用户只需配置 `api_url` 和 `api_key` 即可切换任意后端。
- JSON mode 为可选优化；不支持 response_format=json_object 的模型仍可解析自然 JSON 输出。