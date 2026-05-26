# capfind

[![CI](https://github.com/a1418507570/capfind/actions/workflows/ci.yml/badge.svg)](https://github.com/a1418507570/capfind/actions/workflows/ci.yml)
[![Release](https://github.com/a1418507570/capfind/actions/workflows/release.yml/badge.svg)](https://github.com/a1418507570/capfind/actions/workflows/release.yml)

> 在大型多语言仓库里先找到已有能力，再决定是否新增实现。  
> Find reusable capabilities in big polyglot repos — before you build a new one.

`capfind` 是一个零 LLM、确定性的代码能力检索工具。它会扫描多模块 Java / Go / Proto 代码库，建立一个小型索引，记录仓库里已经存在的能力：HTTP Endpoint、RPC、Service 方法、DAO 方法等。你或 AI 编码助手可以直接查询类似“有没有已经查询 MDM 的接口？”这样的问题，并在毫秒级拿到带 `file:line` 引用的结果。

`capfind` is a zero-LLM, deterministic capability finder. It scans a multi-module Java / Go / Proto codebase and builds a compact index of existing capabilities such as HTTP endpoints, RPCs, service methods, and DAO methods. You — or an AI coding agent — can ask questions like “is there already an endpoint that queries MDM?” and get millisecond results with `file:line` citations.

```bash
$ capfind find mdm query
POST /mdm/query  ·  HttpEndpoint  ·  score 18.4
  MdmController#queryMdm(MdmQueryRequest)
  java/adq-biz/.../MdmController.java:57
```

## 为什么需要它 / Why

大型后端仓库通常已经有很多可复用能力，但开发者和 AI Agent 往往不知道它们在哪里，于是重复造接口、重复写 Service，最后在 Code Review 阶段才发现冲突。

Large backend repositories already contain many reusable capabilities, but developers and AI agents often do not know where they are. Duplicate endpoints and services are built, and code review catches the issue too late.

`capfind` 解决的是“先发现，再开发”的问题：

`capfind` helps you discover before you build:

- **零 LLM / Zero LLM**：同一个 query + 同一个 index，总是得到同一个结果。
- **引用内容索引 / Referenced-content indexing**：不仅索引工程内代码，还索引工程引用的外部 jar、声明依赖和实际 import，让“能不能复用已有外部 API”也能被发现。
- **低成本 / Cheap**：几秒完成中型仓库索引，毫秒级查询。
- **易部署 / Portable**：单个 Rust 二进制文件，无服务端、无 daemon、无 JVM 依赖。
- **Agent 友好 / AI-friendly**：每个结果都带 `file:line`，方便 AI Agent 验证后再行动。

## 当前能力 / Current capabilities

当前稳定能力覆盖 Java / Go / Proto、外部依赖扫描和 AI 友好的结构化输出：

The stable path covers Java / Go / Proto, referenced-content indexing, and
structured AI-friendly output.

- Java Spring `@RestController` / `@Controller` HTTP Endpoint 解析。
- Java `@Service` / `@Repository` / `@Component` 方法解析。
- Java Feign `@FeignClient` 客户端方法、Dubbo `@DubboService` RPC 方法、MyBatis mapper XML statement、JAX-RS / Jakarta REST `@Path` + `@GET/@POST/...` resource 方法解析。
- Go `router.GET/POST/...`、`Group("/v1")` 前缀拼接、chi `r.Get/Post/...`、`http.HandleFunc`、`mux.HandleFunc(...).Methods(...)` HTTP route 解析。
- Proto `service` / `rpc` 解析，支持简单 `google.api.http` annotation 映射。
- Java 外部依赖 / 外部 API 扫描：Maven `pom.xml`、Gradle `build.gradle(.kts)`、本地 `*.jar` 文件名、class 名与 public/protected 方法签名、`*-sources.jar` 方法签名与 Javadoc 摘要、`*-javadoc.jar` 方法文档摘要、源码中的外部 `import`。
- 引用内容命中会在 JSON 中标记 `is_reference`，并输出 `tags`、`annotations`、`doc`，便于 Agent 判断这是外部依赖/API 而不是工程内实现。
- 准确的 `file:line` 引用。
- BM25 + 字段权重 + 层级 boost 搜索。
- 增量索引基础版：`file_stats` 记录 mtime/size/hash，`capfind index` 复用未变化文件，`--rehash` 强制全量重建。
- `.capfind/config.toml` 中 `[search] k1/b` 评分参数生效。
- `find --lang/--kind/--path` 过滤。
- `find --explain` / `explain` 评分解释。
- `capfind agent` 提供 Agent 自动触发前置检查 JSON 输出。
- `capfind agent --auto-index` 支持 Hook 首次运行自动建索引；MCP/context 工具默认也会在已索引文件、新增可索引文件、`.capfind/config.toml` 或 `.capfindignore` 变更后刷新 stale index。
- `capfind agent --fail-on-candidates` 支持检测到候选能力时以退出码 `2` 阻断生成流程。
- `capfind context` / `capfind_context` 提供低 token 的 AI evidence packet，用于盲搜可用能力、外部 API、jar 方法和入口路径。
- `capfind diagnose-query` / `capfind_diagnose_query` 解释 query 预期不好时的问题来源：词表、过滤器、索引覆盖或排序。
- `capfind diagnose-file` / `capfind_diagnose_file` 解释单文件为什么被索引、被忽略、超限、parser 未覆盖或 parser 产出 0 个能力。
- `capfind eval` 读取 JSONL fixture，输出 `capfind.eval.v1`，用 Recall@K、MRR、known Precision@K、文件诊断通过率和服务地图 edge/diagnostic 通过率衡量质量。
- `capfind map` / `capfind_map` 提供机器可消费的代码资产/服务地图，输出模块、包、类、能力节点、服务 registry / SLO 元数据、结构化边、Java 调用边、接口实现解析边、注入依赖边、`exposes` / `wraps` 语义边，以及 Feign / Dubbo / MyBatis XML / JAX-RS / jar 方法调用边。
- `capfind doctor` / `capfind dashboard` 提供索引覆盖、资产类型、模块健康、服务 registry、服务边关系、服务拓扑图、查询趋势、评测质量趋势和指标历史视图。
- `capfind record-adoption` 可选记录候选展示、查看、采用或拒绝事件，用于本地效果分析。
- `capfind context --record-shown` / `capfind_show` `record_inspected=true` 支持 Agent 显式记录展示与查看事件，默认不记录只读搜索噪音。
- `capfind detect-adoption` / `capfind_detect_adoption` 可从 git diff 自动识别候选是否被调用，并去重写入本地日志。
- `scripts/capfind-agent-hook.sh` 提供通用 Hook 包装脚本。
- `capfind.agent.v1` 固化 Agent JSON schema，包含 `schema_version`、`exit_policy` 和 `next_actions`。
- `capfind mcp` 提供 MCP-compatible 工具目录、一次性工具调用 shim 和 JSON-RPC stdio server。
- [Agent / IDE 集成文档](./docs/AGENT_INTEGRATION.md) 提供模型使用规则、MCP 配置模板和非 MCP fallback 命令。
- [Agent Hook 集成文档](./docs/HOOKS.md) 提供触发时机、schema 和接入示例。
- [MCP / Agent 工具集成文档](./docs/MCP.md) 提供工具目录和调用示例。
- `.gitignore` + `.capfindignore` 忽略规则。
- CLI 端到端集成测试覆盖 `init/index/find/show/stats/explain/agent` 主流程。
- GitHub Actions CI 自动执行 `cargo fmt`、`cargo clippy`、`cargo test`。
- `cargo xtask dist` 生成本机 release 包和 SHA-256 校验文件。
- GitHub Release 工作流在 `v*` tag 上自动构建 Linux/macOS 包并发布 Release。
- `scripts/install.sh` 可从 GitHub Release 下载、校验并安装 `capfind`。

v0.3 发布范围升级了 AI-facing 工作流：`capfind context` / MCP `capfind_context` 是低 token 主入口，支持任务级盲搜、服务地图、诊断、可选效果记录和本地 dashboard。v0.2 的 Go / Proto / Java 外部依赖扫描继续保留；v0.3 进一步覆盖 Feign、Dubbo、MyBatis XML、JAX-RS/Jakarta REST、jar/source/javadoc 方法和服务 registry / SLO 元数据。

The v0.3 release improves AI-facing workflows: `capfind context` / MCP
`capfind_context` is the low-token entrypoint, with blind task search, service
maps, diagnosis, optional outcome logging, and a local dashboard. The v0.2 Go /
Proto / Java external dependency scanning remains available; v0.3 expands
coverage to Feign, Dubbo, MyBatis XML, JAX-RS/Jakarta REST, jar/source/javadoc
methods, and service registry / SLO metadata.

## 安装 / Install

> 当前公开 release 为 v0.3.0，可用同一安装脚本指定版本安装。
>
> The current public release is v0.3.0 and can be installed with the same script.
>
> 如果 GitHub Release 元数据或资产暂不可用，安装脚本会在本机存在 `cargo` 时回退到 `cargo install --git`。
>
> If GitHub Release metadata or assets are unavailable, the install script falls back to `cargo install --git` when `cargo` is available.

```bash
# 一键安装 / one-line install (Linux/macOS)
curl -fsSL https://raw.githubusercontent.com/a1418507570/capfind/main/scripts/install.sh | sh

# 安装指定版本 / install a specific version
CAPFIND_VERSION=v0.3.0 sh -c "$(curl -fsSL https://raw.githubusercontent.com/a1418507570/capfind/main/scripts/install.sh)"

# 自定义安装目录 / custom install directory
CAPFIND_INSTALL_DIR="$HOME/bin" sh -c "$(curl -fsSL https://raw.githubusercontent.com/a1418507570/capfind/main/scripts/install.sh)"

# Homebrew（计划中 / planned）
brew install a1418507570/capfind/capfind

# 从源码安装 / from source
cargo install --git https://github.com/a1418507570/capfind capfind-cli
```

## 快速开始 / Quickstart

```bash
cd path/to/your/repo
capfind init           # 创建 .capfind/config.toml 和 .capfindignore
capfind init --product-config  # 可选：生成项目建议配置和 Agent/MCP 模板，不覆盖已有文件
capfind index          # 扫描仓库，生成 .capfind/index.cfi
capfind find mdm query # 查询已有能力
```

English:

```bash
cd path/to/your/repo
capfind init           # creates .capfind/config.toml and .capfindignore
capfind init --product-config  # optional: writes suggested project and Agent/MCP configs safely
capfind index          # walks the repo and builds .capfind/index.cfi
capfind find mdm query # searches indexed capabilities
```

## 配置 / Configuration

`capfind init` 会生成 `.capfind/config.toml`。`[index].roots` 会限制扫描目录，`[index].include` 可进一步限制文件模式；`[search]` 控制 BM25 参数：

`capfind init` creates `.capfind/config.toml`. `[index].roots` limits scan directories, `[index].include` can further limit file patterns, and `[search]` controls BM25 parameters:

`capfind init --product-config` 会额外生成 `.capfind/config.suggested.toml` 和 `.capfind/integrations/` 下的 MCP / Agent 模板。默认只在文件不存在时写入；需要刷新时使用 `--force`。

`capfind init --product-config` also writes `.capfind/config.suggested.toml` and MCP / Agent templates under `.capfind/integrations/`. It writes only missing files by default; use `--force` to refresh them.

```toml
[index]
roots = ["services/api"]
include = ["**/*.java", "**/*.go", "**/*.proto"]

[search]
# k1 必须大于 0 / k1 must be > 0
k1 = 1.2

# b 必须在 0 到 1 之间 / b must be between 0 and 1
b = 0.4

[ownership.modules]
"services/order-api" = "Order Platform"

[ownership.packages]
"com.example.billing" = "Billing Team"

[ownership.external]
"com.fasterxml.jackson.core" = "Runtime Platform"

[services."order-api"]
name = "Order API"
module = "services/order-api"
package = "com.example.order"
owner = "Order Platform"
tier = "gold"
slo = "99.9%"
runbook = "docs/runbooks/order-api.md"
tags = ["public-api"]
```

如果不配置，`capfind` 会使用默认值。无效配置会让命令直接失败并提示具体字段。

If the file or values are missing, `capfind` uses defaults. Invalid values fail fast with field-level errors.

`[ownership.*]` 和 `[services.*]` 是可选配置：不配置时 dashboard / asset map 会按 module、package、class 和 external symbol 推断 owner；配置后会在模块、能力节点、外部依赖和 service registry 中输出 `owner`、`source`、`matched`、`kind`、`tier`、`slo` 和 `runbook`。

## 忽略规则 / Ignore rules

`capfind index` 会先遵守 `.gitignore`，再叠加仓库根目录下的 `.capfindignore`。匹配到的文件和目录会在解析前跳过。`.capfindignore` 使用 gitignore 语法，适合写 capfind 专用排除规则，例如生成代码、测试代码、vendor 目录等。

`capfind index` respects `.gitignore` first and then applies `.capfindignore` from the repo root. Matched files and directories are skipped before parsing. `.capfindignore` uses gitignore syntax and is intended for capfind-specific exclusions such as generated code, test sources, and vendored modules.

```gitignore
/generated
**/*Test.java
**/src/test/**
```

## 质量门禁 / Quality gate

每次 push 和 pull request 都会运行 GitHub Actions CI：

Every push and pull request runs GitHub Actions CI:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo build --bin capfind
CAPFIND_BIN="$PWD/target/debug/capfind" sh scripts/smoke-product-config.sh
cargo xtask dist
sh scripts/smoke-install-release.sh
sh -n scripts/install.sh
```

## 发布打包 / Release packaging

本地发布包可以通过 `cargo xtask dist` 生成。命令会构建 release 版 `capfind`，并在 `target/dist/` 下生成 `.tar.gz` 和 `.sha256`：

Local release artifacts can be generated with `cargo xtask dist`. It builds the release `capfind` binary and writes `.tar.gz` plus `.sha256` files under `target/dist/`:

```bash
cargo xtask dist
sh scripts/smoke-install-release.sh
ls target/dist/
```

多平台发布由 GitHub Actions Release 工作流负责。推送 `v*` tag 会自动构建 Linux/macOS 包，上传 workflow artifacts，并创建 GitHub Release：

Multi-platform release is handled by the GitHub Actions Release workflow. Pushing a `v*` tag builds Linux/macOS packages, uploads workflow artifacts, and creates a GitHub Release:

```bash
git tag v0.3.0
git push origin v0.3.0
```

## Agent 上下文 / Agent context

`capfind context` 是给 AI coding agent 的主入口。它返回低 token 的 `capfind.context.v1` evidence packet，包含候选类型、入口、可调用性、封装/外部关系、证据、置信度和候选质量信号。Agent 不需要先知道包名或模块名，可以直接用任务意图盲搜：

`capfind context` is the primary entrypoint for AI coding agents. It returns a low-token `capfind.context.v1` evidence packet with candidate type, entrypoint, callability, wrapper/external relationship, evidence, and confidence. Agents can blind-search by task intent without knowing package or module names first:

当用户意图是“找现有接口 / 能力 / 调用链 / 可复用实现”时，Agent 应先检查 capfind：仓库存在 `.capfind/` 或 `capfind` 命令可用时，先跑 `capfind stats` / `capfind doctor`，再用 `capfind context` 或 `capfind find` 找候选；`rg` / `git grep` 放在后面做候选验证、调用点展开或空结果补查。

When the user asks for an existing interface, capability, call chain, or reusable implementation, agents should check capfind first: if `.capfind/` exists or the `capfind` command is available, run `capfind stats` / `capfind doctor`, then use `capfind context` or `capfind find` for candidates. Use `rg` / `git grep` afterward for verification, call-site expansion, or empty-result supplementation.

```bash
test -f .capfind/index.cfi && capfind stats
capfind doctor --json
capfind context add mdm query endpoint
capfind context 获取法人身份证账号姓名 --limit 5
capfind find mdm query --limit 5
capfind context use jackson object mapper --limit 5
capfind mcp --call capfind_context --args '{"task":"use jackson object mapper","limit":5}'
```

中文业务 query 会在搜索侧确定性扩展为常见代码字段词，例如“法人 / 身份证 / 账号 / 姓名”会辅助匹配 `legalPerson`、`idCard`、`accountName` 等 identifier token。项目专有词仍建议写成代码里真实出现的类名、方法名、路径片段或字段名。

Chinese business queries are deterministically expanded at search time into common code-field terms. For example, terms like legal person, ID card, account, and name help match identifier tokens such as `legalPerson`, `idCard`, and `accountName`. For project-specific vocabulary, prefer terms that actually appear in class names, methods, paths, or fields.

`capfind agent` 仍保留为兼容 Hook / IDE Hook 的前置检查入口：

`capfind agent` remains as a compatibility preflight entrypoint for hooks and IDE workflows:

```bash
capfind agent add mdm query endpoint
capfind agent add mdm query endpoint --kind endpoint --json
capfind agent add mdm query endpoint --auto-index --fail-on-candidates --json
```

输出是稳定 JSON，schema 版本为 `capfind.agent.v1`，包含 `schema_version`、`has_candidates`、`recommendation`、`exit_policy`、`next_actions` 和带 `file:line` 的候选能力。Hook 可以根据 `recommendation` 决定先复用、先询问用户，还是继续生成新代码。`--auto-index` 适合 Hook 第一次运行时自动创建 `.capfind/index.cfi`；`--fail-on-candidates` 会在找到候选能力时以退出码 `2` 结束，适合“先阻断、再让 Agent 复核”的严格模式。

The output is stable JSON with schema version `capfind.agent.v1`. It includes `schema_version`, `has_candidates`, `recommendation`, `exit_policy`, `next_actions`, and candidate capabilities with `file:line` citations. Hooks can use `recommendation` to decide whether to reuse, ask the user, or continue implementing new code. `--auto-index` creates `.capfind/index.cfi` on first hook run; `--fail-on-candidates` exits with code `2` when candidates are found, which is useful for strict “block first, review before coding” workflows.

通用 Hook 包装脚本：

Generic hook wrapper:

```bash
# 非阻断模式 / advisory mode
scripts/capfind-agent-hook.sh add mdm query endpoint

# 阻断模式 / strict blocking mode
CAPFIND_HOOK_STRICT=1 scripts/capfind-agent-hook.sh add mdm query endpoint

# 从 stdin 接收任务 / read task from stdin
echo "add mdm query endpoint" | scripts/capfind-agent-hook.sh
```

更多模型使用规则、MCP 配置模板和非 MCP fallback 命令见 [Agent / IDE 集成模板](./docs/AGENT_INTEGRATION.md)。更多触发时机、JSON schema 和 Hook 接入建议见 [Agent Hook 集成文档](./docs/HOOKS.md)。

See [Agent / IDE Integration](./docs/AGENT_INTEGRATION.md) for model-use rules, MCP templates, and non-MCP fallback commands. See [Agent Hook Integration](./docs/HOOKS.md) for trigger points, JSON schema, and hook guidance.

## 常用命令 / Common commands

```bash
capfind init
capfind index
capfind find mdm query
capfind find mdm query --kind endpoint --limit 5
capfind find jackson databind # 查询引用内容：外部 jar / import
capfind find external dependency jackson --json
capfind explain mdm query
capfind context use jackson object mapper --limit 5
capfind context use jackson object mapper --limit 5 --record-shown
capfind diagnose-query mdm query --lang java
capfind diagnose-file src/main/java/com/demo/MdmController.java
capfind eval --suite local-golden --fail-under-recall 0.8 --fail-under-file-pass 0.8 --fail-under-edge-pass 0.8 --fail-under-diagnostic-pass 0.8 --fail-under-ownership-pass 0.8
capfind map --limit 200
capfind agent add mdm query endpoint --auto-index --json
CAPFIND_HOOK_STRICT=1 scripts/capfind-agent-hook.sh add mdm query endpoint
capfind mcp --list-tools
capfind mcp --call capfind_context --args '{"task":"use jackson object mapper","limit":5}'
capfind mcp --call capfind_context --args '{"task":"use jackson object mapper","limit":5,"record_shown":true,"session_id":"agent-run-42"}'
capfind mcp --call capfind_show --args '{"id":12,"task":"add mdm query endpoint","record_inspected":true,"session_id":"agent-run-42"}'
capfind mcp --call capfind_diagnose_query --args '{"query":"mdm query","limit":5}'
capfind mcp --call capfind_diagnose_file --args '{"file":"src/main/java/com/demo/MdmController.java"}'
capfind mcp --call capfind_map --args '{"limit":200}'
capfind mcp --call capfind_search --args '{"query":"mdm query","limit":5}'
capfind mcp --stdio
capfind show 12
capfind show 12 --task "add mdm query endpoint" --record-inspected
capfind stats
capfind doctor --json
capfind dashboard
capfind dashboard --module src/main --relationship calls --graph-limit 12
capfind dashboard --json --no-record-history
capfind record-adoption 12 --task "add mdm query endpoint" --file src/main/java/com/demo/Foo.java --session-id agent-run-42
capfind detect-adoption --since HEAD --task "reuse mdm query" --candidate-id 12 --session-id agent-run-42
capfind diagnose src/main/java/com/demo/MdmController.java
```

## 路线图 / Roadmap

- **v0.1**：Java parser、准确引用、BM25 搜索、`[search]` 配置、Explain、`.capfindignore`、CLI 集成测试、CI、多平台 Release、安装脚本、基础体验。
- **v0.2**：Go HTTP route parser、Proto/RPC parser、Java 外部 jar/API 扫描、引用内容元数据、增量索引基础版、Agent preflight、Hook 支撑、MCP-compatible 工具 shim 与 stdio server 基础版。
- **v0.3**：AI-facing context、服务地图调用链、诊断、可选效果记录、IDE / Agent 配置模板和配置增强。

See [ROADMAP](./docs/ROADMAP.md) for the detailed plan and [CHANGELOG](./CHANGELOG.md) for release notes.

## 状态 / Status

v0.3.0 已发布。`capfind` 当前支持低 token context、服务地图、诊断、黄金评测、本地 dashboard、product config、MCP auto-index 和 release 安装 smoke。

v0.3.0 has been released. `capfind` now supports low-token context, service maps, diagnosis, golden evals, local dashboard output, product config, MCP auto-index, and release install smoke coverage.

## 许可证 / License

本项目采用双许可证，用户可任选其一：

This project is dual-licensed. You may choose either license:

- [MIT](./LICENSE-MIT)
- [Apache-2.0](./LICENSE-APACHE)

SPDX 表达式 / SPDX expression:

```text
MIT OR Apache-2.0
```
