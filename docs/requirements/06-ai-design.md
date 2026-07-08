# AI 功能设计

## 1. 设计原则

### 1.1 AI 非必选

- AI 功能**默认关闭**。
- 用户无需配置 API 即可完美使用传统的静态扫描与时间 Diff 功能。
- 仅在用户主动配置了 `api_url` 和 `api_key` 且 `ai.enabled = true` 时激活。

### 1.2 隐私红线

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
【文件头部采样】:
  "192.168.1.100 - - [08/Jul/2026:14:15:00] "GET /api/v1/auth HTTP/1.1" 401 ..."
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

## 4. Prompt 组装模板

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

## 5. Token 消耗统计 (Token Metric Tracker)

### 5.1 需求背景

用户对大模型 API 费用和用量高度敏感，系统必须内置 Token 统计机制。

### 5.2 实现需求

| 阶段 | 功能 | 说明 |
|------|------|------|
| 请求前预估 | 根据 Prompt 字符长度进行轻量级 Token 预估 | 在界面提示"预计消耗 ~300 tokens" |
| 请求后计量 | 解析大模型返回体的 `usage` 字段 | 精确记录 `prompt_tokens` + `completion_tokens` |
| 本地持久化 | Token 消耗数据写入本地数据库 | 支持历史查询 |

### 5.3 仪表盘展示

在配置页或 AI 观察窗底部展示：
- 单次消耗
- 本日累计
- 历史总计消耗
- 每日 Token 上限（如有配置）

## 6. API 兼容性

- 兼容所有提供 OpenAI 格式接口的服务（OpenAI、Azure OpenAI、本地 ollama、vLLM 等）。
- 用户只需配置 `api_url` 和 `api_key` 即可切换任意后端。
