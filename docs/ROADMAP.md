# capfind Roadmap

`capfind` 的目标是成为一个零 LLM、确定性、AI-agent 友好的代码能力检索工具：先发现仓库里已经存在的 endpoint、RPC、service、DAO，再决定是否新增实现。

## v0.1 — Java 可发布版本

状态：v0.1.0 已发布。

目标：稳定打通 Java Spring 仓库的 `index -> find -> cite` 主链路。

- Java Spring endpoint / service / repository 解析
- 准确的 `file:line` 引用
- BM25 + 字段权重 + 层级 boost 搜索
- `find --lang/--kind/--path` 过滤语义正确
- `find --explain` / `explain` 输出命中词、字段权重和 boost 明细
- `.capfind/config.toml` 与 `.capfindignore` 的最小可用配置
- CLI 集成测试、parser fixture、CI
- Linux/macOS 二进制发布

## v0.2 — Polyglot 能力扩展

状态：v0.2.0 已发布。Go HTTP route parser、Proto/RPC parser、引用内容索引与增量索引基础版已进入发布版本；Go 已覆盖直接路由、简单 Group 前缀拼接和 chi 风格方法，Proto 已覆盖 `service` / `rpc` 与简单 `google.api.http` annotation，引用内容索引已覆盖 Maven/Gradle 依赖、本地 jar 文件名、class 名、public/protected 方法签名、`*-sources.jar` 方法签名与 Javadoc 摘要、`*-javadoc.jar` 方法文档摘要和外部 import。

目标：覆盖大型混合语言后端仓库的主要能力入口。

- Go parser：Gin / Echo / Hertz / chi / net/http route，service/interface 方法
  - 已覆盖 `router.GET/POST/...`、简单 `Group("/v1")` 前缀拼接、chi `r.Get/Post/...`、`http.HandleFunc`、`mux.HandleFunc(...).Methods(...)`
- Proto parser：service / rpc，HTTP annotation 映射
  - 第一版已覆盖 `service` / `rpc` 和简单 `google.api.http` 的 `get/post/put/delete/patch` 映射
- 引用内容索引 / Java 外部依赖/API 扫描：Maven/Gradle 依赖、本地 jar 文件名、class 名、public/protected 方法签名、`*-sources.jar` 方法签名与 Javadoc 摘要、`*-javadoc.jar` 方法文档摘要、源码外部 import
  - 命中在 JSON 中标记 `is_reference`，并输出 `tags`、`annotations`、`doc`
  - 当前解析 classfile 方法表、sources jar 方法注释和 javadoc jar 方法说明，不解析方法体
- 增量索引：`file_stats`、`repo_fp`、`--rehash`
  - 基础版已记录 mtime/size/hash，并复用未变化文件能力；`--rehash` 强制全量重建
- 更完整的 config schema：include/exclude、search 参数、自定义 synonyms
- 大仓库 benchmark 与索引性能优化

## v0.3 — Agent / MCP 集成

状态：`capfind agent` 前置检查入口已进入 `main`，并已支持 Hook 首次运行自动建索引、候选命中退出码阻断、`capfind.agent.v1` JSON schema、通用 Hook 包装脚本和 MCP-compatible 工具 shim。

目标：让 CodeBuddy、Claude Code、Cursor 等 AI coding agent 可以直接检索和复用能力。

- `capfind agent`：Agent Hook / IDE Hook 可调用的 JSON 前置检查入口
  - 已支持 `--auto-index`：Hook 第一次运行时自动生成索引
  - 已支持 `--fail-on-candidates`：命中候选能力时以退出码 `2` 阻断生成流程
  - 已提供 `scripts/capfind-agent-hook.sh` 通用包装脚本
  - 已固化 `capfind.agent.v1` JSON schema，包含 `exit_policy` 和 `next_actions`
- `capfind mcp` MCP-compatible 工具目录、一次性工具调用 shim 与 JSON-RPC stdio server
- `capfind-mcp` 独立 crate / advanced server
- IDE / Agent 集成示例
- 重复能力检测：对“我要新增一个能力”的需求返回已有候选
- PR / code review 前置检查

## 当前重点

v0.2.0 已发布，当前重点转为 v0.3：更完整 MCP 协议覆盖、独立 `capfind-mcp` crate、IDE / Agent 配置模板、PR Review 前置检查、大仓库 benchmark 与配置增强。详见 [CHANGELOG](../CHANGELOG.md)。
