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

  curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
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
  [ -n "$resolved_version" ] || fail "could not resolve release version"
  asset_version=${resolved_version#v}
  base_url="https://github.com/$REPO/releases/download/$resolved_version"

  package="capfind-v$asset_version-$target.tar.gz"
  archive_url="$base_url/$package"
  checksum_url="$archive_url.sha256"

  tmpdir=$(mktemp -d)
  trap 'rm -rf "$tmpdir"' EXIT INT TERM

  archive="$tmpdir/$package"
  checksum="$tmpdir/$package.sha256"

  log "installing capfind ($resolved_version, $target)"
  download "$archive_url" "$archive"
  download "$checksum_url" "$checksum"

  expected=$(awk '{print $1}' "$checksum")
  actual=$(sha256_file "$archive")
  [ "$expected" = "$actual" ] || fail "checksum mismatch for $package"

  tar -xzf "$archive" -C "$tmpdir"
  binary=$(find "$tmpdir" -type f -name "$BINARY_NAME" | head -n 1)
  [ -n "$binary" ] || fail "archive did not contain $BINARY_NAME"

  install_binary "$binary"
  log "installed $INSTALL_DIR/$BINARY_NAME"
  log "run: $BINARY_NAME --help"
}

main "$@"
