# capfind

> Find reusable capabilities in big polyglot repos — before you build a new one.

`capfind` scans a multi-module Java + Go codebase and builds a small deterministic
index of the capabilities that already exist (HTTP endpoints, RPC methods, services,
DAOs). You — or an AI coding agent — can then ask questions like "is there already
an endpoint that queries MDM?" and get back an answer with `file:line` citations in
milliseconds, with no LLM in the loop.

```
$ capfind find mdm query
POST /mdm/query  ·  HttpEndpoint  ·  score 18.4
  MdmController#queryMdm(MdmQueryRequest)
  java/adq-biz/.../MdmController.java:57
```

## Why

Large polyglot repos pay a hidden tax: developers (and AI agents generating code)
don't know what already exists. They build duplicates. Code review catches it.
Everyone rolls back.

`capfind` is the deterministic, zero-dependency tool that closes the gap:

- **Zero LLM.** Same query + same index → same answer, always.
- **Cheap.** Seconds to index a ~5 000-capability repo, milliseconds to query.
- **Portable.** A single Rust binary. No services, no daemons, no JVM.
- **AI-friendly.** Every result carries `file:line` — agents can verify before
  acting.

## Install

*(v0.1 is under active development. Install instructions below ship with the first release.)*

```bash
# one-line install (linux/macos)
curl -fsSL https://capfind.sh | sh

# homebrew
brew install capfind-ai/capfind/capfind

# from source
cargo install --git https://github.com/capfind-ai/capfind capfind-cli
```

## Quickstart

```bash
cd path/to/your/repo
capfind init           # creates .capfind/{config.toml, .capfindignore}
capfind index          # walks the repo, builds .capfind/index.cfi
capfind find mdm query # searches the index
```

## Status

v0.1 (in progress) — Java parser, BM25 search, CLI, Linux/macOS binaries.

See [ROADMAP](./docs/ROADMAP.md) for what's landing when.

## License

MIT OR Apache-2.0
