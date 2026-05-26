#!/usr/bin/env sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dist_dir="${CAPFIND_DIST_DIR:-$repo_root/target/dist}"

archive=$(find "$dist_dir" -maxdepth 1 -type f -name 'capfind-v*.tar.gz' | sort | tail -n 1)
[ -n "$archive" ] || {
  echo "error: no capfind release archive found in $dist_dir" >&2
  exit 1
}
[ -f "$archive.sha256" ] || {
  echo "error: missing checksum for $archive" >&2
  exit 1
}

package=$(basename "$archive")
version=${package#capfind-v}
version=${version%%-*}

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT INT TERM
mkdir -p "$tmpdir/bin" "$tmpdir/install"

cat >"$tmpdir/bin/curl" <<'EOF'
#!/usr/bin/env sh
set -eu

output=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      output=$2
      shift 2
      ;;
    http://* | https://*)
      url=$1
      shift
      ;;
    *)
      shift
      ;;
  esac
done

case "$url" in
  *"/releases/latest")
    printf '{"tag_name":"v%s"}\n' "$CAPFIND_SMOKE_VERSION"
    ;;
  *".tar.gz.sha256")
    [ -n "$output" ] || exit 2
    cp "$CAPFIND_SMOKE_ARCHIVE.sha256" "$output"
    ;;
  *".tar.gz")
    [ -n "$output" ] || exit 2
    cp "$CAPFIND_SMOKE_ARCHIVE" "$output"
    ;;
  *)
    exit 22
    ;;
esac
EOF
chmod 755 "$tmpdir/bin/curl"

PATH="$tmpdir/bin:$PATH" \
CAPFIND_SMOKE_ARCHIVE="$archive" \
CAPFIND_SMOKE_VERSION="$version" \
CAPFIND_INSTALL_DIR="$tmpdir/install" \
CAPFIND_VERSION=latest \
sh "$repo_root/scripts/install.sh"

"$tmpdir/install/capfind" --version >/dev/null
echo "release install smoke passed"
