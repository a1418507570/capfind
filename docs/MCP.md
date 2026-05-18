# MCP / Agent 工具集成 / MCP & Agent Tool Integration

`capfind mcp` 提供 MCP-compatible 的工具目录、一次性本地工具调用入口，以及 JSON-RPC stdio MCP Server。

`capfind mcp` provides an MCP-compatible tool catalog, one-shot local tool-call shim, and a JSON-RPC stdio MCP server.

## 工具列表 / Tool catalog

```bash
capfind mcp --list-tools
```

输出包含：

Output includes:

- `capfind_search`：搜索工程内能力与引用内容。
- `capfind_show`：按 ID 展示单个能力详情。
- `capfind_agent_preflight`：编码前复用/重复能力前置检查。

## 一次性工具调用 / One-shot tool calls

### `capfind_search`

```bash
capfind mcp --call capfind_search --args '{"query":"mdm query","limit":5}'
```

### `capfind_show`

```bash
capfind mcp --call capfind_show --args '{"id":12}'
```

### `capfind_agent_preflight`

```bash
capfind mcp --call capfind_agent_preflight --args '{"task":"add mdm query endpoint","limit":5}'
```

## stdio MCP Server

```bash
capfind mcp --stdio
```

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

- IDE / Agent 可以先用 `capfind mcp --list-tools` 发现工具 schema。
- 自动生成代码前调用 `capfind_agent_preflight`。
- 需要展开候选细节时调用 `capfind_show`。
- 需要自由检索工程能力或引用内容时调用 `capfind_search`。

- IDEs / agents can use `capfind mcp --list-tools` to discover tool schemas.
- Call `capfind_agent_preflight` before generating new code.
- Call `capfind_show` to inspect a candidate by id.
- Call `capfind_search` for free-form capability or referenced-content search.

## 当前状态 / Current status

当前实现已支持本地一次性工具调用和 JSON-RPC stdio server。stdio server 覆盖 MCP 初始化、工具列表和工具调用三条主链路；后续可以继续补更完整的协议能力和 IDE 配置模板。

The current implementation supports both one-shot local tool calls and a JSON-RPC stdio server. The stdio server covers MCP initialization, tool listing, and tool calls; future work can add more protocol features and IDE configuration templates.

## IDE 配置模板 / IDE configuration templates

不同 IDE / Agent 的 MCP 配置字段可能不同，下面模板只表达核心启动命令：

MCP configuration fields vary by IDE / agent. The templates below only capture the core launch command:

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

如果 IDE 要求指定工作目录，请把 `cwd` 指向目标仓库根目录，确保能找到 `.capfind/index.cfi`。

If the IDE requires a working directory, set `cwd` to the target repository root so `.capfind/index.cfi` can be found.
