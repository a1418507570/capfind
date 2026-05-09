# capfind

> 在大型多语言仓库里先找到已有能力，再决定是否新增实现。  
> Find reusable capabilities in big polyglot repos — before you build a new one.

`capfind` 是一个零 LLM、确定性的代码能力检索工具。它会扫描多模块 Java 代码库，建立一个小型索引，记录仓库里已经存在的能力：HTTP Endpoint、Service 方法、DAO 方法等。你或 AI 编码助手可以直接查询类似“有没有已经查询 MDM 的接口？”这样的问题，并在毫秒级拿到带 `file:line` 引用的结果。

`capfind` is a zero-LLM, deterministic capability finder. It scans a multi-module Java codebase and builds a compact index of existing capabilities such as HTTP endpoints, service methods, and DAO methods. You — or an AI coding agent — can ask questions like “is there already an endpoint that queries MDM?” and get millisecond results with `file:line` citations.

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

v0.1 当前聚焦 Java Spring 仓库：

v0.1 currently focuses on Java Spring repositories:

- Java Spring `@RestController` / `@Controller` HTTP Endpoint 解析。
- Java `@Service` / `@Repository` / `@Component` 方法解析。
- 准确的 `file:line` 引用。
- BM25 + 字段权重 + 层级 boost 搜索。
- `find --lang/--kind/--path` 过滤。
- `find --explain` / `explain` 评分解释。
- `.gitignore` + `.capfindignore` 忽略规则。

Go / Proto / RPC 支持计划放在 v0.2。

Go / Proto / RPC support is planned for v0.2.

## 安装 / Install

> v0.1 仍在快速迭代中，正式安装方式会随首个 release 固化。  
> v0.1 is under active development. Install commands will be finalized with the first release.

```bash
# 一键安装 / one-line install (Linux/macOS)
curl -fsSL https://capfind.sh | sh

# Homebrew
brew install capfind-ai/capfind/capfind

# 从源码安装 / from source
cargo install --git https://github.com/capfind-ai/capfind capfind-cli
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

## 忽略规则 / Ignore rules

`capfind index` 会先遵守 `.gitignore`，再叠加仓库根目录下的 `.capfindignore`。`.capfindignore` 使用 gitignore 语法，适合写 capfind 专用排除规则，例如生成代码、测试代码、vendor 目录等。

`capfind index` respects `.gitignore` first and then applies `.capfindignore` from the repo root. `.capfindignore` uses gitignore syntax and is intended for capfind-specific exclusions such as generated code, test sources, and vendored modules.

```gitignore
/generated
**/*Test.java
**/src/test/**
```

## 常用命令 / Common commands

```bash
capfind init
capfind index
capfind find mdm query
capfind find mdm query --kind endpoint --limit 5
capfind explain mdm query
capfind show 12
capfind stats
capfind diagnose src/main/java/com/demo/MdmController.java
```

## 路线图 / Roadmap

- **v0.1**：Java parser、准确引用、BM25 搜索、Explain、`.capfindignore`、CLI 基础体验。
- **v0.2**：Go / Proto parser、增量索引、配置文件生效、性能优化。
- **v0.3**：MCP Server、稳定 JSON schema、AI Agent / PR Review 集成。

See [ROADMAP](./docs/ROADMAP.md) for the detailed plan.

## 状态 / Status

v0.1 正在开发中。当前主链路已经打通：Java capability → index → search → citation。下一步会继续补齐配置文件生效、CLI 集成测试、CI 和发布脚本。

v0.1 is in progress. The main loop is already working: Java capability → index → search → citation. Next steps include config loading, CLI integration tests, CI, and release tooling.

## 许可证 / License

MIT OR Apache-2.0
