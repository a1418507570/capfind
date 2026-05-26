# Changelog

所有重要变更都会记录在这里。本文档中文优先，并保留英文摘要。

All notable changes are documented here. Chinese comes first, with English summaries where useful.

## [Unreleased]

暂无。

## [0.3.2] - 2026-05-26

### 改进 / Changed

- 优化中文业务 query 排序：短语扩展现在带权重，`获取` 对泛 `query/search` 的权重降低；“法人身份证 / 账号 / 账户信息 / 账号姓名 / 主体名称”会扩展到 `legalpersonid`、`identityno`、`accountinfo`、`accountlist`、`relatedaccount`、`accountname`、`customername`、`enterprisename` 等更贴近业务字段的代码词。
- 当 query 同时表达“法人 + 账号/账户信息”时，包含账号和法人字段的 ServiceMethod / HttpEndpoint 会获得语义排序提升；OCR / 验真 / 校验类候选只有在 query 明确包含“识别 / OCR / 验真 / 校验 / 二要素”等意图时才保持高优先级。

## [0.3.1] - 2026-05-26

### 新增 / Added

- 中文业务 query 会在搜索侧确定性扩展为常见代码字段词，例如“获取法人身份证账号姓名”可扩展到 `legalperson`、`idcard`、`account`、`accountname` 等词，提升 Agent `context` / `agent` preflight 在中文意图下的命中率。

## [0.3.0] - 2026-05-26

### 新增 / Added

- **AI 主入口 / AI context**：`capfind context` / `capfind_context` 输出低 token、机器可消费的 `capfind.context.v1` evidence packet，支持任务级盲搜、候选类型、entrypoint、callability、relationship、confidence 和 `quality_signal`。
- **AI 资产图 / 服务地图**：`capfind map` / `capfind_map` 输出 `capfind.asset_map.v1`，包含模块、包、类、能力节点、服务 registry / SLO 元数据、结构化边、Java 调用边、接口实现解析边、注入依赖边、`exposes` / `wraps` 语义边和模糊目标诊断。
- **显式服务注册表**：`.capfind/config.toml` 支持 `[services."..."]`，可声明 service name、module/package、owner、tier、SLO、runbook 和 tags；dashboard / asset map 会消费这些元数据。
- **本地 dashboard**：`capfind dashboard` 增加服务 registry、服务拓扑、模块 drilldown、SVG service graph、ownership drilldown、query history、eval quality history、metric trend chart、module coverage history 和本地候选结果视图。
- **产品化初始化**：`capfind init --product-config` 安全生成 `.capfind/config.suggested.toml`、通用 MCP 模板和 Agent rules，默认不覆盖已有文件，并尊重 `.capfindignore` / generated / build 目录。
- **候选结果事件 / 拒绝原因**：`record-adoption` / `capfind_record_adoption` 支持 `shown/inspected/adopted/rejected` 阶段和 `rejected_reason` 分类，dashboard 汇总本地结果事件和拒绝原因。
- **Dashboard 指标历史**：`capfind dashboard` 默认追加 `.capfind/dashboard-history.jsonl` 快照，并在 JSON / HTML 中展示最近指标与趋势；可用 `--no-record-history` 只读查看。
- **Agent 结果事件**：`capfind context --record-shown` 与 MCP `capfind_context(record_shown=true)` 可显式记录候选展示；MCP `capfind_show(record_inspected=true)` 可显式记录候选查看。
- **候选结果关联视图**：结果事件可携带 `session_id`，dashboard JSON / HTML 会按 task、candidate、session 汇总 `shown/inspected/adopted/rejected` 路径和最终结果。
- **MCP stale index 自动刷新**：MCP/context 工具在 auto-index 开启时会检测已索引文件、`.capfind/config.toml` 和 `.capfindignore` 的明显变更，并在服务请求前刷新索引。
- **Dashboard 服务拓扑**：`capfind dashboard` 现在输出 `topology.module_links`、`topology.module_drilldowns` 和 `topology.sample_edges`，HTML 增加 Service Topology 表格。
- **服务地图语义边 / 诊断**：`capfind map` / `capfind_map` 现在输出 Java 构造器、`@Autowired`、`@Resource` 注入的 `depends_on` 边，基于调用边派生 `exposes` / `wraps` 语义边，并在 `graph.diagnostics` 中解释多实现接口等模糊目标。
- **Agent / IDE 集成模板**：新增 `docs/AGENT_INTEGRATION.md`，提供 MCP 配置模板、模型使用规则、工具选择规则和非 MCP fallback 命令。
- **自动候选调用检测**：新增 `capfind detect-adoption` 与 `capfind_detect_adoption`，可从 `git diff` 自动识别最终代码是否调用候选，并去重写入 `.capfind/adoptions.jsonl`。
- **MCP 工具**：`capfind mcp --list-tools` / `--call` 现在包含 `capfind_detect_adoption`。
- **服务地图调用边**：`capfind map` / `capfind_map` 现在会在 `graph.edges` 中输出 Java 简单限定调用边，例如 controller -> service -> repository，并能把接口类型调用解析到唯一实现类能力。
- **框架调用边**：Java parser 和 `capfind map` 现在支持 Feign `@FeignClient`、Dubbo `@DubboService`、JAX-RS/Jakarta REST、MyBatis mapper XML statement，以及 sources/jar 方法调用边，并在 edge evidence 中标记 `target_framework` / `target_type`。
- **文件覆盖诊断**：新增 `capfind diagnose-file` 与 `capfind_diagnose_file`，输出 `capfind.file_diagnosis.v1`，用于解释单文件是否在 roots/include/ignore/parser/index 覆盖内。
- **黄金评测**：新增 `capfind eval`，读取 query/file/edge/diagnostic/ownership JSONL fixture 并输出 `capfind.eval.v1`，包含 Recall@K、MRR、known Precision@K、文件诊断通过率、服务地图 edge/diagnostic 通过率、ownership 通过率、suite 分组历史和 fail-under 阈值。
- **Release 安装 smoke**：CI / Release workflow 现在会在 `cargo xtask dist` 后运行 `scripts/smoke-install-release.sh`，本地模拟 release asset 下载、校验和二进制可执行性。

### 改进 / Changed

- MCP/context 默认在缺失或明显陈旧时自动索引/刷新，降低 AI tool 的接入摩擦；需要严格失败时可使用 `--no-auto-index`。
- `capfind_context` 会消费候选质量信号，提升强复用候选，降低噪音或低置信候选的排序优先级，同时保留给模型判断的结构化解释。
- `diagnose-file` 增加 parser support matrix 和 framework coverage hints，帮助 Agent 判断是代码问题、索引问题还是 parser 覆盖问题。
- `dashboard` 增加 `architecture.module_health` / `architecture.edge_counts` / `history`，HTML 输出改为表格化展示模块健康、服务边关系、覆盖类型、候选结果、拒绝原因和指标历史。
- MCP `capfind_record_adoption` 写入的结果事件现在标记为 `source="mcp"`，便于 dashboard 区分 CLI 手动记录与 Agent/MCP 工具调用。
- `scripts/install.sh` 安装完成后会输出项目级 next steps、MCP 启动命令、MCP config 提示和集成文档链接。
- `record-adoption` / `dashboard` 的结果记录现在可以同时消费手动记录和自动检测记录。

### 修复 / Fixed

- 修复 Go parser 在 CJK UTF-8 文本附近切片导致 panic 的问题。
- 修复 `[index].roots` 未限制扫描范围、`.capfindignore` 未在解析前生效、大仓库索引读取解压缓冲不足等 v0.2 阻断问题。
- 大仓库索引流程已并行化并加入回归覆盖，企业级 fixture 的查询/文件/edge/diagnostic/ownership gate 保持 1.0。

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
