# Agent / IDE Integration

This page is the low-noise setup guide for AI coding agents. The goal is simple:
before an agent writes backend code, it should ask capfind what already exists,
then inspect the fastest callable capability before deciding whether to add new
code.

## Recommended Mode

Use MCP stdio when the agent or IDE supports tools:

```bash
capfind mcp --stdio
```

Run the server from the target repository root. `capfind_context` and MCP tool
calls build `.capfind/index.cfi` lazily on first use by default, and refresh it
before serving when indexed files or capfind config files have changed.

Use one-shot CLI calls when MCP is not available:

```bash
capfind context "add mdm query endpoint"
capfind mcp --call capfind_context --args '{"task":"add mdm query endpoint","limit":5}'
```

For a project-local setup bundle, run:

```bash
capfind init --product-config
```

This writes `.capfind/config.suggested.toml` plus MCP / Agent templates under
`.capfind/integrations/` when those files do not already exist. Use `--force`
only when you intentionally want to refresh the generated suggestions.

Use strict hooks only when the product flow should block duplicate capability
creation before the agent continues:

```bash
capfind agent add mdm query endpoint --auto-index --fail-on-candidates --json
```

## Tool Selection Rules

### Search preflight

When the user intent is to find an existing interface, capability, call chain,
or reusable implementation, treat capfind as the first search preflight:

1. Detect capfind availability: use it when the repository has `.capfind/` or
   the `capfind` command is available.
2. Check health before searching: call MCP `capfind_doctor`, or run
   `capfind stats` when `.capfind/index.cfi` exists plus `capfind doctor --json`.
3. Search indexed capabilities first: use `capfind_context` for implementation
   planning, or `capfind_search` / `capfind find` for exploratory lookup.
4. Use `rg` or `git grep` after capfind to verify cited candidates, inspect
   call sites, or supplement empty/surprising results.

Chinese business terms are expanded at search time into common code-field
tokens. For example, "获取法人身份证账号姓名" can match identifiers such as
`legalPerson`, `legalPersonId`, `identityNo`, `accountInfo`, `accountList`,
`relatedAccount`, and `accountName` without the agent knowing those code terms
first. Account/legal-person queries prioritize account-information candidates;
OCR / verification candidates stay high priority only when the query explicitly
asks for OCR, recognition, verification, validation, or two-factor checks. When
project-specific Chinese terms still miss, call
`capfind_diagnose_query` and retry with class, method, path, or field terms from
the local codebase.

Teams can teach capfind local vocabulary through `.capfind/config.toml`:

```toml
[synonyms]
商户资料 = ["merchantAccount", "merchantId"]
"客户主体" = ["customerName", "enterpriseName"]
```

`capfind_diagnose_query` reports matched `configured_synonyms` so agents can
explain whether project vocabulary entered the index vocab.

| Agent intent | Tool | Expected use |
|---|---|---|
| Plan a new capability or API call | `capfind_context` | Blind-search reusable internal capabilities, external APIs, jar methods, and entry paths. |
| Understand service/module structure | `capfind_map` | Read machine-consumable graph nodes, call/dependency/semantic edges, and diagnostics for ambiguous targets. |
| Inspect one candidate | `capfind_show` | Open details after `capfind_context` returns a candidate id; pass `record_inspected=true` with `task` when the agent expands details for a plan. |
| Free-form search | `capfind_search` | Search when the task is exploratory rather than implementation planning. |
| Explain unexpected query results | `capfind_diagnose_query` | Decide whether the issue is query terms, filters, ranking, or index coverage. |
| Explain a missing file/capability | `capfind_diagnose_file` | Check roots, include, ignore, file size, parser support, and stale index state. |
| Check workspace health | `capfind_doctor` | Verify index/config/coverage when results are empty or suspicious. |
| Block duplicate business work | `capfind_agent_preflight` | Use for hook-like flows before creating endpoints/services/DAOs/RPCs. |
| Check generated code usage | `capfind_detect_adoption` | Infer whether generated code called a selected candidate from git diff. |
| Record explicit outcome | `capfind_record_adoption` | Persist an optional local outcome event. |

## Agent Rule Snippet

Add this to project rules, custom instructions, or an agent skill:

```text
When the user intent is to find an existing interface, capability, call chain,
or reusable implementation, use capfind before text search. If this repository
has .capfind/ or the capfind command is available, run capfind_doctor first,
then call capfind_context with a short task description and limit 5. Use
rg/git grep after capfind only to verify cited candidates, inspect call sites,
or supplement empty/surprising results.

Before implementing a backend endpoint, service method, DAO/repository method,
RPC, integration call, or external-library wrapper, call capfind_context with a
short task description and limit 5.

If capfind returns high-confidence callable candidates, inspect the cited
file:line or capfind_show output before writing code. Pass record_inspected=true
and the same task when opening a candidate detail. Prefer calling the candidate
directly, extending it, or using the external API it points to.
Use candidate quality_signal as a secondary signal: high-confidence candidates
can be tried first when the task matches, while low-confidence candidates should
be inspected before use.

If a candidate is not reused, continue only after identifying the reason:
wrong ownership boundary, unsafe abstraction, missing behavior, stale index, or
not callable for this task.

When results are empty but should not be, call capfind_diagnose_query. When a
known file is missing, call capfind_diagnose_file. When service ownership or
dependencies matter, call capfind_map and inspect graph.edges plus
graph.diagnostics. `depends_on` edges explain constructor or annotation
injection, `exposes` edges show entrypoints exposing internal capabilities, and
`wraps` edges show internal services wrapping downstream APIs or mappers.
When available, inspect service metadata on modules and capability nodes for
owner, tier, slo, runbook, and tags before crossing service boundaries.

Outcome recording is optional. When enabled, use one stable session_id for the
same agent run, then call capfind_detect_adoption or capfind_record_adoption
after generation. Search/context/diagnosis calls are logged locally as query
history so the dashboard can show top queries, zero-result queries, and query
trends over time.
```

## MCP Config Templates

MCP clients vary in where they store configuration. The stable contract is the
process command:

```json
{
  "command": "capfind",
  "args": ["mcp", "--stdio"],
  "cwd": "/absolute/path/to/repo"
}
```

If the client supports an `mcpServers` map, use this shape:

```json
{
  "mcpServers": {
    "capfind": {
      "command": "capfind",
      "args": ["mcp", "--stdio"],
      "cwd": "/absolute/path/to/repo"
    }
  }
}
```

If `capfind` is not on `PATH`, point `command` at the installed binary:

```json
{
  "mcpServers": {
    "capfind": {
      "command": "/Users/you/.local/bin/capfind",
      "args": ["mcp", "--stdio"],
      "cwd": "/absolute/path/to/repo"
    }
  }
}
```

If the IDE launches tools from the repository root automatically, omit `cwd`.
If it does not, keep `cwd`; otherwise capfind may index or read the wrong
workspace.

### Generic MCP Clients

Use the same `mcpServers.capfind` process definition in the client's MCP
configuration location, then restart or reload tools so the catalog shows
`capfind_context`, `capfind_map`, and diagnosis tools. Keep `cwd` set to the
repository root for project-local indexing.

## Minimal Non-MCP Workflow

Agents without MCP can still use capfind through shell commands:

```bash
test -f .capfind/index.cfi && capfind stats
capfind doctor --json
capfind context "add mdm query endpoint" --limit 5
capfind context "获取法人身份证账号姓名" --limit 5
capfind find "mdm query" --limit 5
capfind diagnose-query "mdm query"
capfind diagnose-file src/main/java/com/demo/MdmController.java
capfind map --limit 200
```

## Success Signal

The success signal is:

```text
Generated code either reuses an existing capability or has a clear reason not to.
```

The agent should optimize for callable candidates first, then use diagnosis
tools when the result set is empty, noisy, or surprising.
