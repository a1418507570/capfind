#!/usr/bin/env sh
set -eu

REPO="${CAPFIND_REPO:-a1418507570/capfind}"
VERSION="${CAPFIND_VERSION:-latest}"
INSTALL_DIR="${CAPFIND_INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="capfind"

usage() {
  cat <<'EOF'
Install capfind from GitHub Releases.

Usage:
  curl -fsSL https://raw.githubusercontent.com/a1418507570/capfind/main/scripts/install.sh | sh

Environment:
  CAPFIND_VERSION      Release tag to install, e.g. v0.1.0. Default: latest
  CAPFIND_INSTALL_DIR  Install directory. Default: $HOME/.local/bin
  CAPFIND_REPO         GitHub repo. Default: a1418507570/capfind

If a GitHub release cannot be resolved or downloaded, the script falls back to
`cargo install --git` when cargo is available.
EOF
}

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

resolve_version() {
  if [ "$VERSION" != "latest" ]; then
    printf '%s' "$VERSION"
    return
  fi

  tag=$(
    curl -fsSL \
      -H "Accept: application/vnd.github+json" \
      -H "User-Agent: capfind-install" \
      "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -n 1
  ) || tag=""

  if [ -n "$tag" ]; then
    printf '%s' "$tag"
    return
  fi

  effective_url=$(
    curl -fsSIL \
      -H "User-Agent: capfind-install" \
      -o /dev/null \
      -w '%{url_effective}' \
      "https://github.com/$REPO/releases/latest" 2>/dev/null
  ) || effective_url=""
  printf '%s' "$effective_url" \
    | sed -n 's#.*/tag/\([^/?#]*\).*#\1#p' \
    | head -n 1
}

detect_target() {
  os=$(uname -s | tr '[:upper:]' '[:lower:]')
  arch=$(uname -m)

  case "$os" in
    linux) os_part="unknown-linux-gnu" ;;
    darwin) os_part="apple-darwin" ;;
    *) fail "unsupported OS: $os" ;;
  esac

  case "$arch" in
    x86_64 | amd64) arch_part="x86_64" ;;
    aarch64 | arm64) arch_part="aarch64" ;;
    *) fail "unsupported architecture: $arch" ;;
  esac

  printf '%s-%s' "$arch_part" "$os_part"
}

sha256_file() {
  file=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    fail "sha256sum or shasum is required for checksum verification"
  fi
}

download() {
  url=$1
  output=$2
  log "downloading $url"
  curl -fL --retry 3 --retry-delay 2 -o "$output" "$url"
}

install_binary() {
  src=$1
  mkdir -p "$INSTALL_DIR"
  cp "$src" "$INSTALL_DIR/$BINARY_NAME"
  chmod 755 "$INSTALL_DIR/$BINARY_NAME"
}

install_from_source() {
  tmpdir=$1
  tag=${2:-}
  if ! command -v cargo >/dev/null 2>&1; then
    fail "could not resolve/download a GitHub release and cargo is not available for source install"
  fi

  log "falling back to source install via cargo"
  cargo_root="$tmpdir/cargo-root"
  if [ -n "$tag" ]; then
    cargo install \
      --git "https://github.com/$REPO" \
      --tag "$tag" \
      capfind-cli \
      --bin "$BINARY_NAME" \
      --root "$cargo_root"
  else
    cargo install \
      --git "https://github.com/$REPO" \
      capfind-cli \
      --bin "$BINARY_NAME" \
      --root "$cargo_root"
  fi
  install_binary "$cargo_root/bin/$BINARY_NAME"
}

post_install_message() {
  installed="$INSTALL_DIR/$BINARY_NAME"

  log "installed $installed"
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) log "path note: add $INSTALL_DIR to PATH or use $installed directly" ;;
  esac

  log ""
  log "next steps:"
  log "  1. cd <repo> && $installed init --product-config"
  log "  2. $installed context \"add mdm query endpoint\""
  log "  3. MCP command: $installed mcp --stdio"
  log "  4. MCP config: command=\"$installed\", args=[\"mcp\",\"--stdio\"]"
  log "  5. Docs: https://github.com/$REPO/blob/main/docs/AGENT_INTEGRATION.md"
  log "note: context and MCP auto-build .capfind/index.cfi on first use by default"
}

main() {
  case "${1:-}" in
    -h | --help)
      usage
      exit 0
      ;;
  esac

  need_cmd uname
  need_cmd tr
  need_cmd awk
  need_cmd sed
  need_cmd curl
  need_cmd tar
  need_cmd mkdir
  need_cmd cp
  need_cmd chmod
  need_cmd mktemp
  need_cmd find

  target=$(detect_target)
  resolved_version=$(resolve_version)
  tmpdir=$(mktemp -d)
  trap 'rm -rf "$tmpdir"' EXIT INT TERM

  if [ -z "$resolved_version" ]; then
    install_from_source "$tmpdir" ""
    post_install_message
    exit 0
  fi

  asset_version=${resolved_version#v}
  base_url="https://github.com/$REPO/releases/download/$resolved_version"

  package="capfind-v$asset_version-$target.tar.gz"
  archive_url="$base_url/$package"
  checksum_url="$archive_url.sha256"

  archive="$tmpdir/$package"
  checksum="$tmpdir/$package.sha256"

  log "installing capfind ($resolved_version, $target)"
  if ! download "$archive_url" "$archive" || ! download "$checksum_url" "$checksum"; then
    install_from_source "$tmpdir" "$resolved_version"
    post_install_message
    exit 0
  fi

  expected=$(awk '{print $1}' "$checksum")
  actual=$(sha256_file "$archive")
  [ "$expected" = "$actual" ] || fail "checksum mismatch for $package"

  tar -xzf "$archive" -C "$tmpdir"
  binary=$(find "$tmpdir" -type f -name "$BINARY_NAME" | head -n 1)
  [ -n "$binary" ] || fail "archive did not contain $BINARY_NAME"

  install_binary "$binary"
  post_install_message
}

main "$@"
