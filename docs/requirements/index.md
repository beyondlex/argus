# Argus 项目需求文档

## 项目简介

**Argus** 是一款基于 Rust 开发的、面向个人桌面用户的**时间差分（Time Diff）与 AI 辅助**磁盘空间治理工具。通过后台轻量级监控与常驻，帮助用户直观发现"某个目录在过去一段时间内暴涨了多少"，并通过 AI 百科解决用户"不知道这个目录是干什么的、不敢删"的痛点。

- TUI 客户端：**Argus**（百眼巨人，巨细靡遗的监控者）
- 守护进程：**Argusd**

---

## 文档目录

| # | 文档 | 内容简介 |
|---|------|---------|
| 01 | [项目概述与定位](01-overview.md) | 背景、定位、目标用户、核心价值、设计原则 |
| 02 | [系统架构设计](02-architecture.md) | 三层架构、Cargo Workspace、双模驱动、技术栈选型 |
| 03 | [核心功能需求](03-core-features.md) | 时间差分引擎、守护进程、安全删除、高性能扫描 |
| 04 | [配置系统设计](04-configuration.md) | AI/键位/主题/忽略/守护进程/Token 配置 |
| 05 | [多端交互设计](05-ux-interaction.md) | CLI/TUI/GUI 三端交互规范与快捷键 |
| 06 | [AI 功能设计](06-ai-design.md) | 特征提取、百科诊断、Token 统计、API 兼容 |
| 07 | [安全设计](07-safety.md) | 系统黑名单、风险分级、废纸篓机制、审计日志 |
| 08 | [数据模型与算法](08-data-model.md) | FileNode/DiffNode 结构、Tree Merge 算法、IPC 协议 |
| 09 | [迭代路线图](09-roadmap.md) | Phase 1-5 完整演进规划 |
| 10 | [Phase 1 实施指南](10-phase1-guide.md) | MVP 阶段具体开发指导与验收标准 |
| 11 | [日志系统设计](11-logging.md) | 等级规范、JSON 结构、Span 定义、AI Agent 调试 |
| 12 | [Phase 2 实施指南](12-phase2-guide.md) | TUI 极客版具体开发指导与验收标准 |

---

## 核心设计原则

- **AI 非必选**：无需 AI 即可完美使用全部传统功能
- **极致轻量**：守护进程闲置 CPU ≈ 0%，内存 < 30MB
- **隐私优先**：AI 仅上传路径/类型特征，不扫描文件内容
- **安全红线**：黑名单 + 废纸篓 + 强交互确认，杜绝误删
- **跨平台**：首阶段支持 macOS + Linux

## 五阶段演进

```
Phase 1         Phase 2      Phase 3        Phase 4      Phase 5
MVP+CLI   →    TUI极客版  →  Daemon自动化 → AI完全体   → GUI桌面版
```

## 需求来源

当前以本目录下的 `01`-`11` 文档作为实现需求源。历史讨论稿不随仓库维护，AI Agent 实施时不应依赖仓库外部需求文件。

## 进度入口

如需跨会话续接实现进度，先读 [AGENT_PROGRESS.md](../../AGENT_PROGRESS.md)。
