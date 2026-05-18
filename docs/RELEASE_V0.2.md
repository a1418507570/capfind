# v0.2 发布收敛计划 / v0.2 Release Hardening Plan

本文档用于 v0.2.0 发布前收敛：冻结范围、列出必跑验证、明确发布步骤和风险项。

This document hardens the v0.2.0 release: freeze scope, list required validation, define release steps, and track risks.

## 1. 发布目标 / Release goal

v0.2.0 的目标是把 `capfind` 从 Java 单语言能力检索，推进到“多语言 + 引用内容 + Agent 自动触发 + MCP 接入”的可发布版本。

v0.2.0 turns `capfind` from a Java-only capability finder into a publishable polyglot, referenced-content-aware, Agent/MCP-ready release.

## 2. 范围冻结 / Scope freeze

v0.2.0 发布前不再继续扩新功能，除非是修复编译、测试、文档或明显阻断问题。

No new feature work before v0.2.0 unless it fixes compilation, tests, documentation, or release-blocking issues.

### 已纳入范围 / In scope

- Go HTTP route parser。
- Proto/RPC parser。
- 引用内容索引：Maven / Gradle / jar / sources jar / javadoc jar / Java import。
- Agent preflight：`capfind agent`、`capfind.agent.v1`。
- Hook 支撑：`--auto-index`、`--fail-on-candidates`、`scripts/capfind-agent-hook.sh`。
- 增量索引基础版：`file_stats`、mtime/size/hash、`--rehash`。
- MCP / Agent 工具接入：`capfind mcp --list-tools`、`--call`、`--stdio`。
- README、HOOKS、MCP、ROADMAP、CHANGELOG 文档。

### 不纳入 v0.2 / Out of scope

- 完整 Java class 方法体解析。
- 完整 Javadoc 站点解析与复杂泛型签名恢复。
- 完整生产级 MCP 协议覆盖。
- GUI / IDE 插件原生 UI。
- 远程仓库索引服务。

## 3. 必跑验证 / Required validation

在具备 Rust 工具链的环境执行：

Run in an environment with Rust toolchain:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo xtask dist
sh -n scripts/install.sh
sh -n scripts/capfind-agent-hook.sh
```

### 当前环境可执行预检 / Preflight checks available in limited environments

如果当前机器暂时没有 `cargo` / `rustc` / `rustfmt`，至少先执行不依赖 Rust 工具链的预检；但这些不能替代完整 Rust 验证：

If the current machine does not have `cargo`, `rustc`, or `rustfmt`, run the non-Rust preflight checks first. They do not replace full Rust validation:

```bash
git diff --check
sh -n scripts/install.sh
sh -n scripts/capfind-agent-hook.sh
```

### 重点观察 / Watch closely

- `crates/capfind-cli/src/indexer.rs`：引用内容索引、jar/classfile、sources/javadoc 解析、增量索引。
- MCP stdio 测试是否阻塞。
- 增量索引是否因为 mtime/hash 造成测试不稳定。
- `zip` 依赖是否正确进入 lockfile。
- `cargo clippy` 对大函数、复杂分支、无用 clone 的提示。

## 4. 手动冒烟 / Manual smoke tests

在任意 Java / Go / Proto 混合仓库内：

Inside a mixed Java / Go / Proto repository:

```bash
capfind init
capfind index
capfind index
capfind index --rehash
capfind find mdm query --json
capfind find external dependency jackson --json
capfind agent add mdm query endpoint --auto-index --json
CAPFIND_HOOK_STRICT=1 scripts/capfind-agent-hook.sh add mdm query endpoint
capfind mcp --list-tools
capfind mcp --call capfind_search --args '{"query":"mdm query","limit":5}'
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | capfind mcp --stdio
```

期望：

Expected:

- 第二次 `capfind index` 显示复用未变化文件。
- 引用内容命中包含 `is_reference: true` 与 `external` tag。
- `agent --fail-on-candidates` 找到候选时退出码为 `2` 且 stdout 是 JSON。
- `mcp --stdio` 返回 JSON-RPC `2.0` 响应。

## 5. 发布步骤 / Release steps

验证通过后：

After validation passes:

1. 更新版本号到 `0.2.0`：
   - workspace `Cargo.toml`
   - workspace local dependency versions
2. 确认 `CHANGELOG.md` 中 `[0.2.0] - planned` 改为实际日期。
3. 更新 README 状态为 `v0.2.0 已发布 / released`。
4. 提交并推送 main。
5. 打 tag 并发布：

```bash
git tag v0.2.0
git push origin main
git push origin v0.2.0
```

6. 等待 GitHub Release workflow 产物。
7. 指定版本安装验证：

```bash
CAPFIND_VERSION=v0.2.0 sh -c "$(curl -fsSL https://raw.githubusercontent.com/a1418507570/capfind/main/scripts/install.sh)"
capfind --version
```

8. 同步 GitWoa。

## 6. 风险与处置 / Risks and mitigations

| 风险 / Risk | 处置 / Mitigation |
| --- | --- |
| `indexer.rs` 过大，clippy 或维护成本高 | v0.2 发布后拆分 `external/jar/incremental` 模块 |
| sources/javadoc 解析启发式不完整 | 保持已知限制，后续增强 HTML/泛型解析 |
| MCP stdio 协议覆盖不完整 | 明确当前为基础版，先满足 initialize/tools/list/tools/call |
| 本地缺 Rust 工具链未验证 | 发布前必须在可用环境跑完整验证 |
| `Cargo.lock` 未同步新增依赖 | 在有 Rust 工具链环境运行 `cargo generate-lockfile` 或 `cargo check -p capfind-cli` 并提交 lockfile |
| 版本号仍为 `0.1.0` | 完整验证通过后统一 bump workspace 与 local dependency version 到 `0.2.0` |
| 引用内容索引噪声过多 | 后续增加 include/exclude、dependency scope 配置 |

## 7. 发布检查清单 / Release checklist

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets`
- [ ] `cargo test --workspace`
- [ ] `cargo xtask dist`
- [ ] `sh -n scripts/install.sh`
- [ ] `sh -n scripts/capfind-agent-hook.sh`
- [ ] `Cargo.lock` 已同步 `zip` / `blake3` 等新增依赖
- [ ] 手动冒烟通过
- [ ] `CHANGELOG.md` 日期更新
- [ ] README 状态更新
- [ ] 版本号更新为 `0.2.0`
- [ ] GitHub main 推送
- [ ] `v0.2.0` tag 推送
- [ ] Release artifacts 验证
- [ ] GitWoa 同步
