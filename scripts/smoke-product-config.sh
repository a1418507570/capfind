#!/usr/bin/env sh
set -eu

CAPFIND_BIN="${CAPFIND_BIN:-capfind}"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

need_file() {
  [ -f "$1" ] || fail "expected file: $1"
}

need_contains() {
  grep -F "$2" "$1" >/dev/null 2>&1 || fail "expected $1 to contain: $2"
}

need_not_contains() {
  if grep -F "$2" "$1" >/dev/null 2>&1; then
    fail "expected $1 not to contain: $2"
  fi
}

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT INT TERM

repo="$tmpdir/repo"
mkdir -p "$repo/src/main/java/com/acme/order"
mkdir -p "$repo/server/api"
mkdir -p "$repo/target/generated/com/noisy"

cat >"$repo/pom.xml" <<'EOF'
<project>
  <dependencies>
    <dependency>
      <groupId>com.fasterxml.jackson.core</groupId>
      <artifactId>jackson-databind</artifactId>
      <version>2.17.0</version>
    </dependency>
  </dependencies>
</project>
EOF

cat >"$repo/src/main/java/com/acme/order/OrderController.java" <<'EOF'
package com.acme.order;
public class OrderController {}
EOF

cat >"$repo/server/api/routes.go" <<'EOF'
package api
func register() {}
EOF

cat >"$repo/target/generated/com/noisy/Generated.java" <<'EOF'
package com.noisy.generated;
public class Generated {}
EOF

printf 'server/**\n' >"$repo/.capfindignore"

(cd "$repo" && "$CAPFIND_BIN" init --product-config >/dev/null)

need_file "$repo/.capfind/config.toml"
need_file "$repo/.capfind/config.suggested.toml"
need_file "$repo/.capfind/integrations/mcp.generic.json"
need_file "$repo/.capfind/integrations/agent-rules.md"
need_file "$repo/.capfind/integrations/mcp-client.md"

need_contains "$repo/.capfind/config.suggested.toml" '"**/*.java"'
need_contains "$repo/.capfind/config.suggested.toml" 'com.acme.order'
need_contains "$repo/.capfind/config.suggested.toml" 'com.fasterxml.jackson.core'
need_not_contains "$repo/.capfind/config.suggested.toml" '"**/*.go"'
need_not_contains "$repo/.capfind/config.suggested.toml" 'com.noisy'
need_contains "$repo/.capfind/integrations/mcp.generic.json" '"mcpServers"'
need_contains "$repo/.capfind/integrations/agent-rules.md" 'capfind_context'
need_contains "$repo/.capfind/integrations/agent-rules.md" 'rg/git grep after capfind'
need_contains "$repo/.capfind/integrations/mcp-client.md" 'before rg/git grep'

printf 'sentinel\n' >"$repo/.capfind/config.suggested.toml"
(cd "$repo" && "$CAPFIND_BIN" init --product-config >/dev/null)
need_contains "$repo/.capfind/config.suggested.toml" 'sentinel'

(cd "$repo" && "$CAPFIND_BIN" init --product-config --force >/dev/null)
need_contains "$repo/.capfind/config.suggested.toml" 'Suggested capfind product config'

printf 'product config smoke passed\n'
