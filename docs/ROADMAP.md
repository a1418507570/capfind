# capfind Roadmap

`capfind` is a deterministic capability finder for large polyglot repositories.
It helps developers and AI coding agents discover existing endpoints, RPCs,
service methods, DAO methods, and referenced APIs before adding new code.

## Released

### v0.1

- Java Spring endpoint / service / repository parsing.
- Accurate `file:line` citations.
- BM25 search with field weighting and explain output.
- `.capfind/config.toml`, `.capfindignore`, CI, and release packaging.

### v0.2

- Go HTTP route parsing.
- Proto service / RPC parsing with simple HTTP annotations.
- Java referenced-content indexing for Maven/Gradle dependencies, local jars,
  source jars, javadoc jars, and external imports.
- Basic incremental indexing.
- Agent preflight, hook wrapper, and MCP-compatible local tools.

### v0.3

- `capfind context` / `capfind_context` structured evidence packets for AI
  coding agents.
- `capfind map` / `capfind_map` asset-map output with modules, packages,
  capabilities, selected ownership metadata, and call/dependency edges.
- Query and file diagnosis commands.
- Golden evaluation fixtures and threshold checks.
- Local dashboard output for repository coverage and tool feedback.
- Product setup helper: `capfind init --product-config`.

## Next

- Broaden language and framework coverage.
- Improve graph precision on larger repositories.
- Keep release packaging and installation smoke tests reliable.
- Continue reducing noise in AI-facing output.
