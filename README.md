# capfind

[![CI](https://github.com/a1418507570/capfind/actions/workflows/ci.yml/badge.svg)](https://github.com/a1418507570/capfind/actions/workflows/ci.yml)
[![Release](https://github.com/a1418507570/capfind/actions/workflows/release.yml/badge.svg)](https://github.com/a1418507570/capfind/actions/workflows/release.yml)

> 在大型多语言仓库里先找到已有能力，再决定是否新增实现。  
> Find reusable capabilities in big polyglot repos — before you build a new one.

`capfind` 是一个零 LLM、确定性的代码能力检索工具。它会扫描多模块 Java / Go 代码库，建立一个小型索引，记录仓库里已经存在的能力：HTTP Endpoint、Service 方法、DAO 方法等。你或 AI 编码助手可以直接查询类似“有没有已经查询 MDM 的接口？”这样的问题，并在毫秒级拿到带 `file:line` 引用的结果。

`capfind` is a zero-LLM, deterministic capability finder. It scans a multi-module Java / Go codebase and builds a compact index of existing capabilities such as HTTP endpoints, service methods, and DAO methods. You — or an AI coding agent — can ask questions like “is there already an endpoint that queries MDM?” and get millisecond results with `file:line` citations.

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
- **低成本 / Cheap**：几秒完成中型仓库索引，毫秒级查询。
- **易部署 / Portable**：单个 Rust 二进制文件，无服务端、无 daemon、无 JVM 依赖。
- **Agent 友好 / AI-friendly**：每个结果都带 `file:line`，方便 AI Agent 验证后再行动。

## 当前能力 / Current capabilities

当前稳定能力聚焦 Java Spring 仓库，`main` 分支已开始引入 v0.2 的 Go HTTP route parser 第一版：

The stable path focuses on Java Spring repositories, and `main` has started the first v0.2 slice: Go HTTP route parsing.

- Java Spring `@RestController` / `@Controller` HTTP Endpoint 解析。
- Java `@Service` / `@Repository` / `@Component` 方法解析。
- Go `router.GET/POST/...`、`http.HandleFunc`、`mux.HandleFunc(...).Methods(...)` HTTP route 解析。
- 准确的 `file:line` 引用。
- BM25 + 字段权重 + 层级 boost 搜索。
- `.capfind/config.toml` 中 `[search] k1/b` 评分参数生效。
- `find --lang/--kind/--path` 过滤。
- `find --explain` / `explain` 评分解释。
- `capfind agent` 提供 Agent 自动触发前置检查 JSON 输出。
- `.gitignore` + `.capfindignore` 忽略规则。
- CLI 端到端集成测试覆盖 `init/index/find/show/stats/explain` 主流程。
- GitHub Actions CI 自动执行 `cargo fmt`、`cargo clippy`、`cargo test`。
- `cargo xtask dist` 生成本机 release 包和 SHA-256 校验文件。
- GitHub Release 工作流在 `v*` tag 上自动构建 Linux/macOS 包并发布 Release。
- `scripts/install.sh` 可从 GitHub Release 下载、校验并安装 `capfind`。

Go parser 仍处于 v0.2 早期切片；Proto / RPC 支持计划放在后续 v0.2 迭代。

The Go parser is still an early v0.2 slice; Proto / RPC support is planned for later v0.2 iterations.

## 安装 / Install

> v0.1 仍在快速迭代中，正式安装方式会随首个 release 固化。  
> v0.1 is under active development. Install commands will be finalized with the first release.

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
git tag v0.1.0
git push origin v0.1.0
```

## Agent 自动触发 / Agent preflight

`capfind` 目前不会自己常驻后台，也不会像 RTK 一样无条件自动触发；但已经提供 `capfind agent` 作为 Agent Hook / IDE Hook 的前置检查入口。AI Agent 在准备新增接口或 Service 前，可以自动调用它，先查询仓库里是否已有类似能力：

`capfind` does not run as a background daemon and does not auto-trigger by itself like RTK. It now provides `capfind agent` as the preflight entry point for Agent hooks or IDE hooks. Before creating a new endpoint or service, an AI agent can call it to check whether similar capabilities already exist:

```bash
capfind agent add mdm query endpoint
capfind agent add mdm query endpoint --kind endpoint --json
```

输出是稳定 JSON，包含 `has_candidates`、`recommendation` 和带 `file:line` 的候选能力。Hook 可以根据 `recommendation` 决定先复用、先询问用户，还是继续生成新代码。

The output is stable JSON with `has_candidates`, `recommendation`, and candidate capabilities with `file:line` citations. Hooks can use `recommendation` to decide whether to reuse, ask the user, or continue implementing new code.

## 常用命令 / Common commands

```bash
capfind init
capfind index
capfind find mdm query
capfind find mdm query --kind endpoint --limit 5
capfind explain mdm query
capfind agent add mdm query endpoint
capfind show 12
capfind stats
capfind diagnose src/main/java/com/demo/MdmController.java
```

## 路线图 / Roadmap

- **v0.1**：Java parser、准确引用、BM25 搜索、`[search]` 配置、Explain、`.capfindignore`、CLI 集成测试、CI、多平台 Release、安装脚本、基础体验。
- **v0.2**：Go HTTP route parser、Proto parser、增量索引、完整配置 schema、性能优化。
- **v0.3**：MCP Server、自动触发 Hook、稳定 JSON schema、AI Agent / PR Review 集成。

See [ROADMAP](./docs/ROADMAP.md) for the detailed plan.

## 状态 / Status

v0.1.0 已发布。当前主链路已经打通：Java capability → index → search → citation。`main` 分支正在推进 v0.2 与 Agent 集成前置能力：Go HTTP route parser 第一版和 `capfind agent` 前置检查入口已进入实现；下一步会继续扩展 Go 框架覆盖、Proto/RPC parser 与真正的 Hook/MCP 自动触发。

v0.1.0 has been released. The main loop is working: Java capability → index → search → citation. The `main` branch is now moving toward v0.2 and Agent integration preflight: the first Go HTTP route parser slice and `capfind agent` preflight entry point are implemented; next steps are broader Go framework coverage, Proto/RPC parsing, and real Hook/MCP auto-triggering.

## 许可证 / License

本项目采用双许可证，用户可任选其一：

This project is dual-licensed. You may choose either license:

- [MIT](./LICENSE-MIT)
- [Apache-2.0](./LICENSE-APACHE)

SPDX 表达式 / SPDX expression:

```text
MIT OR Apache-2.0
```
