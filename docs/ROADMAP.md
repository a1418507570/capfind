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

状态：Go HTTP route parser 第一版已进入 `main`。

目标：覆盖大型混合语言后端仓库的主要能力入口。

- Go parser：Gin / Echo / Hertz / net/http route，service/interface 方法
  - 第一版已覆盖 `router.GET/POST/...`、`http.HandleFunc`、`mux.HandleFunc(...).Methods(...)`
- Proto parser：service / rpc，HTTP annotation 映射
- 增量索引：`file_stats`、`repo_fp`、`--rehash`
- 更完整的 config schema：include/exclude、search 参数、自定义 synonyms
- 大仓库 benchmark 与索引性能优化

## v0.3 — Agent / MCP 集成

状态：`capfind agent` 前置检查入口已进入 `main`，可供 Hook/IDE 自动调用。

目标：让 CodeBuddy、Claude Code、Cursor 等 AI coding agent 可以直接检索和复用能力。

- `capfind agent`：Agent Hook / IDE Hook 可调用的 JSON 前置检查入口
- `capfind-mcp` server
- 稳定 JSON schema
- 重复能力检测：对“我要新增一个能力”的需求返回已有候选
- PR / code review 前置检查
- IDE 集成示例

## 当前重点

短期优先级已从 v0.1 收敛转向 v0.2 / v0.3 前置能力：Go HTTP route parser 第一版和 `capfind agent` 前置检查入口已进入实现；下一步继续扩展 Go 框架覆盖、补充真实 Go fixture，并启动 Proto/RPC parser 与 MCP/Hook 自动触发。
