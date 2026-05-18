#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
Usage: capfind-agent-hook.sh <task words...>
       echo "add mdm query endpoint" | capfind-agent-hook.sh

Environment:
  CAPFIND_HOOK_STRICT=1  exit with code 2 when similar capabilities are found
EOF
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage
  exit 0
fi

if [ "$#" -gt 0 ]; then
  task="$*"
else
  task="$(cat)"
fi

if [ -z "$task" ]; then
  usage
  exit 64
fi

set -- capfind agent --auto-index --json
case "${CAPFIND_HOOK_STRICT:-0}" in
  1|true|TRUE|yes|YES)
    set -- "$@" --fail-on-candidates
    ;;
esac

exec "$@" "$task"
