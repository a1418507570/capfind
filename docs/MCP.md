# MCP / Agent 工具集成 / MCP & Agent Tool Integration

`capfind mcp` 提供 MCP-compatible 的工具目录、一次性本地工具调用入口，以及 JSON-RPC stdio MCP Server。

`capfind mcp` provides an MCP-compatible tool catalog, one-shot local tool-call shim, and a JSON-RPC stdio MCP server.

Agent / IDE setup templates and model-use rules live in [Agent / IDE Integration](./AGENT_INTEGRATION.md).

## 工具列表 / Tool catalog

```bash
capfind mcp --list-tools
```

输出包含：

Output includes:

- `capfind_search`：搜索工程内能力与引用内容。
- `capfind_context`：主入口，返回低 token AI evidence packet。
- `capfind_show`：按 ID 展示单个能力详情。
- `capfind_agent_preflight`：编码前复用/重复能力前置检查。
- `capfind_doctor`：返回索引、配置和覆盖率诊断。
- `capfind_diagnose_query`：解释 query 预期不好时是词表、过滤器、索引覆盖还是排序问题。
- `capfind_diagnose_file`：解释单文件索引/parser 覆盖，定位为什么没有产出能力。
- `capfind_map`：返回机器可消费的代码资产/服务地图。
- `capfind_record_adoption`：记录可选的本地候选结果事件。
- `capfind_detect_adoption`：从 git diff 自动识别候选调用并写入本地日志。

## 一次性工具调用 / One-shot tool calls

### `capfind_context`

```bash
capfind mcp --call capfind_context --args '{"task":"use jackson object mapper","limit":5}'
capfind mcp --call capfind_context --args '{"task":"use jackson object mapper","limit":5,"record_shown":true,"session_id":"agent-run-42"}'
```

返回 `capfind.context.v1`，面向模型消费，字段包括 `decision`、`candidates[].type`、`entrypoints`、`relationship`、`evidence`、`callability`、`confidence` 和 jar 方法 `meta`。

Returns `capfind.context.v1` for model consumption, including `decision`, `candidates[].type`, `entrypoints`, `relationship`, `evidence`, `callability`, `confidence`, and jar method `meta`.

`record_shown=true` 会为本次返回的候选写入 `shown` 漏斗事件；默认不记录，避免普通只读搜索产生噪音。

`record_shown=true` records `shown` funnel events for returned candidates. It is off by default to keep read-only searches noise-free.

### `capfind_search`

```bash
capfind mcp --call capfind_search --args '{"query":"mdm query","limit":5}'
```

### `capfind_show`

```bash
capfind mcp --call capfind_show --args '{"id":12}'
capfind mcp --call capfind_show --args '{"id":12,"task":"add mdm query endpoint","record_inspected":true,"session_id":"agent-run-42"}'
```

`record_inspected=true` 需要同时传 `task`，会写入 `inspected` 漏斗事件，表示模型展开过候选详情。

`record_inspected=true` requires `task` and records an `inspected` funnel event, meaning the model expanded candidate details.

### `capfind_agent_preflight`

```bash
capfind mcp --call capfind_agent_preflight --args '{"task":"add mdm query endpoint","limit":5}'
```

### `capfind_doctor`

```bash
capfind mcp --call capfind_doctor --args '{}'
```

### `capfind_diagnose_query`

```bash
capfind mcp --call capfind_diagnose_query --args '{"query":"mdm query","limit":5}'
capfind mcp --call capfind_diagnose_query --args '{"query":"mdm query","lang":"java","kind":"endpoint"}'
```

返回 `capfind.query_diagnosis.v1`，包含 query token 是否进入词表、同义词命中、raw top hits、过滤后的 top hits、过滤损失和下一步建议。它用于回答“为什么结果不符合预期”：是代码没有被索引、parser 没覆盖、query 词不在 vocab、过滤器过窄，还是已有候选排在后面。

Returns `capfind.query_diagnosis.v1`, including query-token vocab presence, synonym hits, raw top hits, filtered top hits, filter loss, and next actions. It answers why results are unexpected: unindexed code, parser coverage gap, query terms missing from vocab, overly narrow filters, or candidates ranked lower than expected.

### `capfind_diagnose_file`

```bash
capfind mcp --call capfind_diagnose_file --args '{"file":"src/main/java/com/demo/MdmController.java"}'
```

返回 `capfind.file_diagnosis.v1`，包含 roots/include/.capfindignore/内置跳过/文件大小/parser 类型/index 命中/陈旧状态/parser 预览能力和下一步建议。它用于回答“为什么这个文件没有成为可复用资产”：路径不在 roots、被忽略、类型不支持、文件过大、索引缺失、索引陈旧，还是 parser 识别不到能力。

Returns `capfind.file_diagnosis.v1`, including roots/include/.capfindignore/builtin skip/file size/parser kind/index hit/staleness/parser preview and next actions. It answers why a file did not become a reusable asset: outside roots, ignored, unsupported type, too large, missing index, stale index, or parser produced no capabilities.

### `capfind_map`

```bash
capfind mcp --call capfind_map --args '{"limit":200}'
capfind mcp --call capfind_map --args '{"module":"services/api","limit":100}'
```

返回 `capfind.asset_map.v1`，包含 `modules`、`external`、`ownership` 和 `graph.nodes/edges/diagnostics`，用于让模型或可视化工具理解模块、包、类、能力节点之间的结构关系。模块、包、能力和外部引用节点会带有从代码路径 / 符号推断或由 `[ownership.modules]`、`[ownership.packages]`、`[ownership.external]` 配置的 ownership 元数据。当前边包含结构化 containment 关系、Java 简单限定调用边、接口类型到唯一实现类的调用解析、构造器 / `@Autowired` / `@Resource` 注入依赖边、`exposes` / `wraps` 语义边，以及 Feign / Dubbo / MyBatis XML / jar 方法调用边。无法唯一解析的多实现接口调用会进入 `graph.diagnostics`。

Returns `capfind.asset_map.v1` with `modules`, `external`, `ownership`, and `graph.nodes/edges/diagnostics`, so models and dashboards can understand relationships between modules, packages, classes, and capabilities. Module, package, capability, and external reference nodes include `ownership` metadata inferred from code paths/symbols or configured through `[ownership.modules]`, `[ownership.packages]`, and `[ownership.external]`. Current edges include structural containment, simple qualified Java call edges, interface-type calls resolved to a unique implementation, constructor / `@Autowired` / `@Resource` dependency edges, `exposes` / `wraps` semantic edges, and Feign / Dubbo / MyBatis XML / jar method call edges. Ambiguous multi-implementation calls are reported in `graph.diagnostics`.

### `capfind_record_adoption`

```bash
capfind mcp --call capfind_record_adoption --args '{"task":"add mdm query endpoint","candidate_id":12,"adopted":true,"files":["src/main/java/com/demo/Foo.java"]}'
capfind mcp --call capfind_record_adoption --args '{"task":"add mdm query endpoint","candidate_id":12,"stage":"rejected","rejected_reason":"wrong_ownership_boundary","note":"must call external owner directly","session_id":"agent-run-42"}'
```

`stage` 可选值为 `shown`、`inspected`、`adopted`、`rejected`。`rejected_reason` 使用稳定分类：`wrong_ownership_boundary`、`not_callable`、`missing_behavior`、`unsafe_abstraction`、`external_api_required`、`low_confidence`、`stale_index`、`user_requested_new`、`other`、`unspecified`。dashboard 会汇总这些本地事件，帮助定位候选质量和拒绝原因。

`stage` can be `shown`, `inspected`, `adopted`, or `rejected`. `rejected_reason` uses stable categories: `wrong_ownership_boundary`, `not_callable`, `missing_behavior`, `unsafe_abstraction`, `external_api_required`, `low_confidence`, `stale_index`, `user_requested_new`, `other`, or `unspecified`. The dashboard aggregates these local events to help explain candidate quality and rejection reasons.

`session_id` is optional on `capfind_context`, `capfind_show`, `capfind_record_adoption`, and `capfind_detect_adoption`. Passing the same value through one agent run lets the dashboard correlate shown, inspected, adopted, and rejected events by task, candidate, and session.

### `capfind_detect_adoption`

```bash
capfind mcp --call capfind_detect_adoption --args '{"since":"HEAD","task":"reuse mdm query","candidate_ids":[12],"session_id":"agent-run-42"}'
```

返回 `capfind.adoption_detection.v1`，包含 `detections`、`recorded_events`、`skipped_duplicates`、`session_id` 和本地汇总。默认会把匹配结果写入 `.capfind/adoptions.jsonl`，用于 dashboard 分析候选调用情况。

## stdio MCP Server

```bash
capfind mcp --stdio
# auto-builds missing index and refreshes stale/new files by default
capfind mcp --stdio --auto-index
# opt out explicitly when a caller requires a read-only existing index
capfind mcp --stdio --no-auto-index
```

When auto-index is enabled, tool calls refresh the index before serving if an already indexed file was modified/deleted, if a newly added supported source appears under configured roots/include rules, or if `.capfind/config.toml` / `.capfindignore` is newer than the index. This keeps long-running MCP sessions from serving obviously stale results while preserving `--no-auto-index` as the strict read-only mode.

支持的 JSON-RPC 方法：

Supported JSON-RPC methods:

- `initialize`
- `tools/list`
- `tools/call`
- `notifications/*`（静默忽略 / ignored silently）

示例：

Example:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"capfind_search","arguments":{"query":"mdm query","limit":5}}}
```

## Agent 接入建议 / Agent integration guidance

完整接入模板、模型规则和非 MCP fallback 命令见 [Agent / IDE Integration](./AGENT_INTEGRATION.md)。

See [Agent / IDE Integration](./AGENT_INTEGRATION.md) for full setup templates, model rules, and non-MCP fallback commands.

- IDE / Agent 可以先用 `capfind mcp --list-tools` 发现工具 schema。
- 当用户意图是“找现有接口 / 能力 / 调用链 / 可复用实现”时，如果仓库存在 `.capfind/` 或 `capfind` 可用，先调用 `capfind_doctor`，再调用 `capfind_context` 或 `capfind_search`；`rg` / `git grep` 只作为候选验证、调用点展开或空结果补充。
- 自动生成方案时优先调用 `capfind_context`。
- 需要阻断重复业务能力时调用 `capfind_agent_preflight`。
- 需要展开候选细节时调用 `capfind_show`。
- 查询结果不符合预期时调用 `capfind_diagnose_query`。
- 怀疑某个文件没被索引或 parser 未覆盖时调用 `capfind_diagnose_file`。
- 需要理解服务/资产结构时调用 `capfind_map`。
- 需要自由检索工程能力或引用内容时调用 `capfind_search`。
- 调用 `capfind_context` 生成方案候选时，可显式传 `record_shown=true` 记录候选展示。
- 调用 `capfind_show` 展开候选详情时，可显式传 `record_inspected=true` 和 `task` 记录候选查看。
- 可选调用 `capfind_record_adoption` 记录候选结果；未采用时传 `stage="rejected"` 和 `rejected_reason`。
- 需要从 git diff 自动识别候选调用时调用 `capfind_detect_adoption`。
- 如果结果为空且不符合预期，调用 `capfind_doctor` 检查覆盖率。

- IDEs / agents can use `capfind mcp --list-tools` to discover tool schemas.
- When the user asks for an existing interface, capability, call chain, or reusable implementation, use capfind first when `.capfind/` exists or `capfind` is available: call `capfind_doctor`, then `capfind_context` or `capfind_search`; use `rg` / `git grep` afterward only for candidate verification, call-site inspection, or empty-result supplementation.
- Prefer `capfind_context` while planning generated code.
- Call `capfind_agent_preflight` when duplicate business capability blocking is needed.
- Call `capfind_show` to inspect a candidate by id.
- Call `capfind_diagnose_query` when search results are unexpected.
- Call `capfind_diagnose_file` when a specific file may be unindexed or uncovered by parsers.
- Call `capfind_map` to understand service/asset structure.
- Call `capfind_search` for free-form capability or referenced-content search.
- Pass `record_shown=true` to `capfind_context` when the agent wants to record displayed candidates.
- Pass `record_inspected=true` and `task` to `capfind_show` when the agent expands candidate details.
- Optionally call `capfind_record_adoption` after generation to record candidate outcomes; when not adopted, pass `stage="rejected"` and `rejected_reason`.
- Call `capfind_detect_adoption` to infer candidate calls from git diff and persist matched events.
- Call `capfind_doctor` when empty results are unexpected.

## 当前状态 / Current status

当前实现已支持本地一次性工具调用、JSON-RPC stdio server 和 Agent / IDE 配置模板。stdio server 覆盖 MCP 初始化、工具列表和工具调用三条主链路；后续可以继续补更完整的协议能力。

The current implementation supports one-shot local tool calls, a JSON-RPC stdio server, and Agent / IDE configuration templates. The stdio server covers MCP initialization, tool listing, and tool calls; future work can add more protocol features.

## IDE 配置模板 / IDE configuration templates

不同 IDE / Agent 的 MCP 配置字段可能不同，下面模板只表达核心启动命令。更多接入建议见 [Agent / IDE Integration](./AGENT_INTEGRATION.md)。

MCP configuration fields vary by IDE / agent. The template below only captures the core launch command. See [Agent / IDE Integration](./AGENT_INTEGRATION.md) for integration guidance:

```json
{
  "mcpServers": {
    "capfind": {
      "command": "capfind",
      "args": ["mcp", "--stdio"]
    }
  }
}
```

如果 IDE 要求指定工作目录，请把 `cwd` 指向目标仓库根目录，确保能找到 `.capfind/index.cfi`。首次运行可把 `args` 改为 `["mcp", "--stdio", "--auto-index"]`。

If the IDE requires a working directory, set `cwd` to the target repository root so `.capfind/index.cfi` can be found. For first-run setups, use `["mcp", "--stdio", "--auto-index"]`.
