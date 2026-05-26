# capfind Eval Fixtures

This directory documents the JSONL fixture format used by `capfind eval`.

`capfind eval` reads these files from the target repository root by default:

```text
fixtures/eval/queries.jsonl
fixtures/eval/files.jsonl
fixtures/eval/edges.jsonl
fixtures/eval/diagnostics.jsonl
fixtures/eval/ownerships.jsonl
```

Each query case defines a task-level query and one or more expected capability matchers. Each file case defines an expected `capfind.file_diagnosis.v1` diagnosis for one source/reference file. Each edge case defines an expected `capfind.asset_map.v1` graph edge between source and target capability matchers. Each diagnostic case asserts an expected service-map diagnostic, such as an ambiguous interface implementation target. Each ownership case asserts configured or inferred ownership metadata on modules, graph nodes, or external references.

`capfind eval` records `.capfind/eval-history.jsonl` by default so a real project can track quality drift over time. Use `--suite <name>` to group snapshots for a fixture set, or `--no-record-history` for throwaway runs.

Bundled fixture suites:

- `demo-repo`: compact parser / search / map smoke fixture.
- `enterprise-repo`: larger enterprise-style multi-module fixture covering Spring controllers/services, Feign, Dubbo, MyBatis XML, JAX-RS resources, Go routes, Proto RPC, generated/test skips, Maven jar dependencies, external API imports, ambiguous payment gateway resolution, wrapper services, configurable ownership, and service registry/SLO metadata.

To try a bundled fixture without dirtying this checkout:

```bash
tmp="$(mktemp -d)"
cp -R fixtures/eval/enterprise-repo "$tmp/repo"
cd "$tmp/repo"
capfind init
capfind index
capfind eval --suite enterprise-fixture --fail-under-recall 1.0 --fail-under-file-pass 1.0 --fail-under-edge-pass 1.0 --fail-under-diagnostic-pass 1.0 --fail-under-ownership-pass 1.0
```
