# Agent Hook 集成 / Agent Hook Integration

`capfind agent` 是给 AI coding agent、IDE 或 CI Hook 调用的稳定前置检查入口。它不会常驻后台；自动触发由外部 Hook、脚本、MCP 或 Agent 工作流负责。

`capfind agent` is the stable preflight entry point for AI coding agents, IDEs, or CI hooks. It does not run as a background daemon; auto-triggering is owned by an external hook, script, MCP integration, or agent workflow.

## 推荐触发时机 / Recommended trigger points

在 Agent 准备新增以下能力前触发：

Trigger before an agent creates one of these capabilities:

- 新 HTTP Endpoint / new HTTP endpoint
- 新 Service 方法 / new service method
- 新 DAO / repository 方法 / new DAO or repository method
- 新 RPC / new RPC method

当 Agent 只是要查找现有接口、能力、调用链或可复用实现时，也应把 capfind 放在文本检索前：先用 `capfind doctor --json` 观察索引覆盖，再用 `capfind context` / `capfind find` 找候选，最后才用 `rg` / `git grep` 验证候选或补查空结果。

When an agent is only looking for an existing interface, capability, call chain, or reusable implementation, put capfind before text search too: use `capfind doctor --json` to check index coverage, then `capfind context` / `capfind find` for candidates, and only then use `rg` / `git grep` for verification or empty-result supplementation.

## 直接调用 / Direct call

```bash
capfind agent add mdm query endpoint --auto-index --json
```

严格模式会在找到候选能力时返回退出码 `2`，适合阻断式 Hook：

Strict mode exits with code `2` when candidates are found, which is useful for blocking hooks:

```bash
capfind agent add mdm query endpoint --auto-index --fail-on-candidates --json
```

## 通用包装脚本 / Generic wrapper

```bash
# 提示模式：输出候选，但不阻断 / advisory mode
scripts/capfind-agent-hook.sh add mdm query endpoint

# 阻断模式：命中候选能力时退出 2 / strict blocking mode
CAPFIND_HOOK_STRICT=1 scripts/capfind-agent-hook.sh add mdm query endpoint

# 从 stdin 接收任务 / read task from stdin
echo "add mdm query endpoint" | scripts/capfind-agent-hook.sh
```

## JSON schema v1

`capfind agent` 输出稳定 JSON。当前 schema 版本为 `capfind.agent.v1`：

`capfind agent` emits stable JSON. The current schema version is `capfind.agent.v1`:

```json
{
  "schema_version": "capfind.agent.v1",
  "task": "add mdm query endpoint",
  "took_ms": 1,
  "has_candidates": true,
  "recommendation": "review_existing_capability_before_implementing",
  "agent_hint": "Review the candidates and cited file:line locations before creating new code. Prefer reuse or extension when appropriate.",
  "exit_policy": {
    "fail_on_candidates": true,
    "candidate_exit_code": 2,
    "no_candidate_exit_code": 0
  },
  "next_actions": [
    "open_candidate_file_lines",
    "prefer_reuse_or_extension",
    "ask_user_before_creating_duplicate_capability"
  ],
  "candidates": []
}
```

字段语义：

Field semantics:

- `schema_version`：Agent 输出 schema 版本；消费者应按版本兼容解析。
- `has_candidates`：是否找到相似已有能力。
- `recommendation`：机器可读建议。
- `agent_hint`：给 Agent 或日志展示的人类可读提示。
- `exit_policy`：当前调用的退出策略，尤其是阻断模式下的退出码。
- `next_actions`：Agent 接下来应该优先执行的动作。
- `candidates`：候选能力列表，每项包含 `file` 和 `line`，便于继续打开源码验证。

## 集成示例 / Integration examples

### Shell preflight

```bash
#!/usr/bin/env sh
set -eu

task="$*"
CAPFIND_HOOK_STRICT=1 scripts/capfind-agent-hook.sh "$task"
```

### CI advisory check

CI 中建议先使用提示模式，只归档 JSON，不直接失败：

In CI, start with advisory mode and archive the JSON without failing the build:

```bash
capfind agent "$CAPFIND_TASK" --auto-index --json > capfind-agent.json
```

### Agent instruction

如果当前 IDE / Agent 没有原生 Hook，可以先把以下规则写入项目级规则或自定义指令：

If the current IDE or agent has no native hook, add this rule to project-level rules or custom instructions first:

```text
When the task is to find an existing interface, capability, call chain, or reusable implementation, run capfind before rg/git grep:
capfind doctor --json
capfind context <short task description> --limit 5
capfind find <short query> --limit 5

Before creating a new endpoint, service, DAO, or RPC method, run:
capfind agent <short task description> --auto-index --json
If has_candidates is true, inspect the cited file:line results and prefer reuse or extension before writing new code.
```

### Client note

不同 IDE / Agent 产品的 Hook 配置格式可能变化。不要假设固定字段；建议优先通过 MCP、自定义命令、Rules、Skills 或自定义 Agent 工作流触发 `capfind agent`。

Hook configuration formats vary across agent products. Keep `scripts/capfind-agent-hook.sh` as the stable entry point, and let the product-specific side pass a short task text before creating a new capability.
