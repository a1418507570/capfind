# Changelog

所有重要变更都会记录在这里。本文档中文优先，并保留英文摘要。

All notable changes are documented here. Chinese comes first, with English summaries where useful.

## [Unreleased]

## [0.2.0] - 2026-05-18

### 新增 / Added

- **Go HTTP route parser**：支持 `router.GET/POST/...`、简单 `Group("/v1")` 前缀拼接、chi 风格 `r.Get/Post/...`、`http.HandleFunc`、`mux.HandleFunc(...).Methods(...)`。
- **Proto/RPC parser**：支持 `service` / `rpc`，并解析简单 `google.api.http` 的 `get/post/put/delete/patch` 映射。
- **引用内容索引 / Referenced-content indexing**：索引工程引用的外部内容，用于发现可复用外部 API，避免重复开发。
  - Maven `pom.xml` 依赖。
  - Gradle `build.gradle(.kts)` 依赖。
  - 本地 `*.jar` 文件名。
  - jar 内 class 名。
  - jar 内 `public/protected` 方法签名。
  - `*-sources.jar` 方法签名与 Javadoc 摘要。
  - `*-javadoc.jar` 方法文档摘要。
  - Java 源码外部 `import`。
- **Agent preflight**：新增 `capfind agent`，输出稳定 JSON schema `capfind.agent.v1`。
- **自动触发支撑**：新增 `--auto-index`、`--fail-on-candidates` 和 `scripts/capfind-agent-hook.sh`。
- **MCP / Agent 工具集成**：新增 `capfind mcp`。
  - `--list-tools` 工具目录。
  - `--call` 一次性工具调用。
  - `--stdio` JSON-RPC stdio server 基础版。
  - 工具：`capfind_search`、`capfind_show`、`capfind_agent_preflight`。
- **增量索引基础版**：记录 `mtime/size/blake3`，默认复用未变化文件；`--rehash` 强制全量重建。
- **文档**：新增 `docs/HOOKS.md`、`docs/MCP.md`，README 持续双语更新。

### 改进 / Changed

- `find --json` / `agent --json` 输出增加引用内容元数据：`is_reference`、`tags`、`annotations`、`doc`。
- `show` 命令补充 RPC 请求/响应类型展示。
- CI shell 语法检查覆盖新增 Hook 脚本。
- README 当前能力、常用命令、状态与路线图全部同步 v0.2 能力。

### 修复 / Fixed

- Go `mux.HandleFunc(...).Methods(...)` 避免重复识别为 `ANY` route。
- Go 简单 Group 路径拼接与 chi 风格方法解析补充回归覆盖。
- Java 外部 import 会过滤项目内部 package 与 `java.*`，避免噪声。
- Maven `scope=test` 依赖默认跳过。

### 已知限制 / Known limitations

- 当前 jar 方法索引解析 classfile 方法表，不解析方法体。
- `*-sources.jar` / `*-javadoc.jar` 解析为轻量启发式，复杂 HTML / 泛型签名可能需要后续增强。
- `capfind mcp --stdio` 是基础 JSON-RPC stdio server，后续仍可补更完整 MCP 协议能力和 IDE 配置模板。
- 当前本地环境缺少 `cargo/rustc/rustfmt`，尚未在本机完成完整 Rust 验证。

## [0.1.0] - 2026-05

### 新增 / Added

- Java Spring endpoint / service / repository parser。
- 准确 `file:line` 引用。
- BM25 + 字段权重 + 层级 boost 搜索。
- `find --lang/--kind/--path` 过滤。
- `find --explain` / `explain` 评分解释。
- `.capfind/config.toml` 与 `.capfindignore`。
- CLI 集成测试、GitHub Actions CI。
- `cargo xtask dist` 发布打包。
- GitHub Release workflow 与 `scripts/install.sh`。
