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

当前稳定能力从 Java Spring 主链路扩展到 v0.2 的 Go / Proto / 外部依赖扫描：

The stable path has expanded from the Java Spring main loop to the v0.2 Go / Proto / external dependency scanning track.

- Java Spring `@RestController` / `@Controller` HTTP Endpoint 解析。
- Java `@Service` / `@Repository` / `@Component` 方法解析。
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
- `capfind agent --auto-index` 支持 Hook 首次运行自动建索引。
- `capfind agent --fail-on-candidates` 支持检测到候选能力时以退出码 `2` 阻断生成流程。
- `scripts/capfind-agent-hook.sh` 提供通用 Hook 包装脚本。
- `capfind.agent.v1` 固化 Agent JSON schema，包含 `schema_version`、`exit_policy` 和 `next_actions`。
- `capfind mcp` 提供 MCP-compatible 工具目录、一次性工具调用 shim 和 JSON-RPC stdio server。
- [Agent Hook 集成文档](./docs/HOOKS.md) 提供触发时机、schema 和接入示例。
- [MCP / Agent 工具集成文档](./docs/MCP.md) 提供工具目录和调用示例。
- `.gitignore` + `.capfindignore` 忽略规则。
- CLI 端到端集成测试覆盖 `init/index/find/show/stats/explain/agent` 主流程。
- GitHub Actions CI 自动执行 `cargo fmt`、`cargo clippy`、`cargo test`。
- `cargo xtask dist` 生成本机 release 包和 SHA-256 校验文件。
- GitHub Release 工作流在 `v*` tag 上自动构建 Linux/macOS 包并发布 Release。
- `scripts/install.sh` 可从 GitHub Release 下载、校验并安装 `capfind`。

v0.2 发布候选范围已覆盖常见 Go 直接路由、简单 Group 前缀、chi 风格方法、Proto `service` / `rpc`、简单 `google.api.http` annotation，以及 Java 工程声明的外部 jar 依赖、源码实际使用的外部 import、本地 jar 内的 class 名称和 public/protected 方法签名、`*-sources.jar` 中的方法签名与 Javadoc 摘要、`*-javadoc.jar` 中的方法文档摘要。引用内容会带 `external` 标签并参与检索。注意：当前外部 jar 会索引依赖坐标、本地 jar 文件名、class 名称、方法签名、sources jar 文档摘要和 javadoc jar 方法说明，暂不解析方法体。

The v0.2 release-candidate scope covers common Go direct route calls, simple Group prefixes, chi-style methods, Proto `service` / `rpc`, simple `google.api.http` annotations, Java external jar dependencies, external imports used by source code, class names and public/protected method signatures inside local jars, method signatures and Javadoc summaries in `*-sources.jar`, plus method documentation summaries in `*-javadoc.jar`. Referenced content is tagged as `external` and participates in search. Note: the external-jar slice indexes dependency coordinates, local jar file names, class names, method signatures, sources-jar doc summaries, and javadoc-jar method docs; it does not parse method bodies yet.

## 安装 / Install

> 当前公开 release 为 v0.1.0；v0.2.0 正在发布收敛中，发布后可用同一安装脚本指定版本安装。
>
> The current public release is v0.1.0. v0.2.0 is in release hardening and can be installed with the same script after the tag is published.

```bash
# 一键安装 / one-line install (Linux/macOS)
curl -fsSL https://raw.githubusercontent.com/a1418507570/capfind/main/scripts/install.sh | sh

# 安装指定版本 / install a specific version
CAPFIND_VERSION=v0.1.0 sh -c "$(curl -fsSL https://raw.githubusercontent.com/a1418507570/capfind/main/scripts/install.sh)"

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
capfind index          # 扫描仓库，生成 .capfind/index.cfi
capfind find mdm query # 查询已有能力
```

English:

```bash
cd path/to/your/repo
capfind init           # creates .capfind/config.toml and .capfindignore
capfind index          # walks the repo and builds .capfind/index.cfi
capfind find mdm query # searches indexed capabilities
```

## 配置 / Configuration

`capfind init` 会生成 `.capfind/config.toml`。当前已生效的配置是 `[search]` 下的 BM25 参数：

`capfind init` creates `.capfind/config.toml`. The currently active settings are BM25 parameters under `[search]`:

```toml
[search]
# k1 必须大于 0 / k1 must be > 0
k1 = 1.2

# b 必须在 0 到 1 之间 / b must be between 0 and 1
b = 0.4
```

如果不配置，`capfind` 会使用默认值。无效配置会让命令直接失败并提示具体字段。

If the file or values are missing, `capfind` uses defaults. Invalid values fail fast with field-level errors.

## 忽略规则 / Ignore rules

`capfind index` 会先遵守 `.gitignore`，再叠加仓库根目录下的 `.capfindignore`。`.capfindignore` 使用 gitignore 语法，适合写 capfind 专用排除规则，例如生成代码、测试代码、vendor 目录等。

`capfind index` respects `.gitignore` first and then applies `.capfindignore` from the repo root. `.capfindignore` uses gitignore syntax and is intended for capfind-specific exclusions such as generated code, test sources, and vendored modules.

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
cargo xtask dist
sh -n scripts/install.sh
```

## 发布打包 / Release packaging

本地发布包可以通过 `cargo xtask dist` 生成。命令会构建 release 版 `capfind`，并在 `target/dist/` 下生成 `.tar.gz` 和 `.sha256`：

Local release artifacts can be generated with `cargo xtask dist`. It builds the release `capfind` binary and writes `.tar.gz` plus `.sha256` files under `target/dist/`:

```bash
cargo xtask dist
ls target/dist/
```

多平台发布由 GitHub Actions Release 工作流负责。推送 `v*` tag 会自动构建 Linux/macOS 包，上传 workflow artifacts，并创建 GitHub Release：

Multi-platform release is handled by the GitHub Actions Release workflow. Pushing a `v*` tag builds Linux/macOS packages, uploads workflow artifacts, and creates a GitHub Release:

```bash
git tag v0.2.0
git push origin v0.2.0
```

## Agent 自动触发 / Agent preflight

`capfind` 目前不会自己常驻后台，也不会像 RTK 一样无条件自动触发；但已经提供 `capfind agent` 作为 Agent Hook / IDE Hook 的前置检查入口。AI Agent 在准备新增接口或 Service 前，可以自动调用它，先查询仓库里是否已有类似能力：

`capfind` does not run as a background daemon and does not auto-trigger by itself like RTK. It now provides `capfind agent` as the preflight entry point for Agent hooks or IDE hooks. Before creating a new endpoint or service, an AI agent can call it to check whether similar capabilities already exist:

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

更多触发时机、JSON schema 和 Agent/IDE 接入建议见 [Agent Hook 集成文档](./docs/HOOKS.md)。

See [Agent Hook Integration](./docs/HOOKS.md) for trigger points, JSON schema, and Agent/IDE integration guidance.

## 常用命令 / Common commands

```bash
capfind init
capfind index
capfind find mdm query
capfind find mdm query --kind endpoint --limit 5
capfind find jackson databind # 查询引用内容：外部 jar / import
capfind find external dependency jackson --json
capfind explain mdm query
capfind agent add mdm query endpoint --auto-index --json
CAPFIND_HOOK_STRICT=1 scripts/capfind-agent-hook.sh add mdm query endpoint
capfind mcp --list-tools
capfind mcp --call capfind_search --args '{"query":"mdm query","limit":5}'
capfind mcp --stdio
capfind show 12
capfind stats
capfind diagnose src/main/java/com/demo/MdmController.java
```

## 路线图 / Roadmap

- **v0.1**：Java parser、准确引用、BM25 搜索、`[search]` 配置、Explain、`.capfindignore`、CLI 集成测试、CI、多平台 Release、安装脚本、基础体验。
- **v0.2**：Go HTTP route parser、Proto/RPC parser、Java 外部 jar/API 扫描、引用内容元数据、增量索引基础版、Agent preflight、Hook 支撑、MCP-compatible 工具 shim 与 stdio server 基础版。
- **v0.3**：更完整的 MCP 协议覆盖、独立 `capfind-mcp` crate、IDE / Agent 配置模板、PR Review 前置检查、大仓库 benchmark 与配置增强。

See [ROADMAP](./docs/ROADMAP.md) for the detailed plan. See [CHANGELOG](./CHANGELOG.md) and [v0.2 Release Hardening](./docs/RELEASE_V0.2.md) for release notes and the release checklist.

## 状态 / Status

v0.2.0 已发布。`capfind` 当前已打通 Java / Go / Proto capability → index → search → citation 主链路，并支持引用内容索引、增量索引基础版、Agent preflight、Hook、MCP-compatible 工具 shim、JSON-RPC stdio server 和集成文档。

v0.2.0 has been released. `capfind` now supports the Java / Go / Proto capability → index → search → citation loop, referenced-content indexing, basic incremental indexing, Agent preflight, Hook support, MCP-compatible tool shim, JSON-RPC stdio server, and integration docs.

## 许可证 / License

本项目采用双许可证，用户可任选其一：

This project is dual-licensed. You may choose either license:

- [MIT](./LICENSE-MIT)
- [Apache-2.0](./LICENSE-APACHE)

SPDX 表达式 / SPDX expression:

```text
MIT OR Apache-2.0
```
