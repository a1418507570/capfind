#!/usr/bin/env sh
set -eu

REPO="${CAPFIND_REPO:-a1418507570/capfind}"
WORKFLOW="${CAPFIND_RELEASE_WORKFLOW:-release.yml}"
DIST_DIR="${CAPFIND_DIST_DIR:-target/dist}"
WAIT_SECONDS="${CAPFIND_RELEASE_WAIT_SECONDS:-120}"
POLL_SECONDS="${CAPFIND_RELEASE_POLL_SECONDS:-10}"

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

sha256_check() {
  checksum=$1
  dir=$(dirname "$checksum")
  file=$(basename "$checksum")
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$dir" && sha256sum -c "$file")
  elif command -v shasum >/dev/null 2>&1; then
    (cd "$dir" && shasum -a 256 -c "$file")
  else
    fail "sha256sum or shasum is required"
  fi
}

resolve_tag() {
  if [ "${1:-}" ]; then
    printf '%s' "$1"
    return
  fi
  if [ "${CAPFIND_RELEASE_TAG:-}" ]; then
    printf '%s' "$CAPFIND_RELEASE_TAG"
    return
  fi
  git describe --tags --exact-match 2>/dev/null || {
    fail "pass a v* tag or run from an exact release tag"
  }
}

find_release_run() {
  gh run list \
    --repo "$REPO" \
    --workflow "$WORKFLOW" \
    --branch "$TAG" \
    --event push \
    --limit 1 \
    --json databaseId \
    --jq '.[0].databaseId // ""' 2>/dev/null || true
}

wait_for_release_workflow() {
  elapsed=0
  while [ "$elapsed" -le "$WAIT_SECONDS" ]; do
    run_id=$(find_release_run)
    if [ -n "$run_id" ]; then
      log "found release workflow run: $run_id"
      gh run watch "$run_id" --repo "$REPO" --exit-status
      return
    fi
    sleep "$POLL_SECONDS"
    elapsed=$((elapsed + POLL_SECONDS))
  done
  return 1
}

verify_dist_assets() {
  version=${TAG#v}
  archives=$(find "$DIST_DIR" -maxdepth 1 -type f -name "capfind-v$version-*.tar.gz" | sort)
  [ -n "$archives" ] || fail "no release archives found in $DIST_DIR for $TAG"

  for archive in $archives; do
    [ -f "$archive.sha256" ] || fail "missing checksum for $archive"
    sha256_check "$archive.sha256"
  done
}

manual_release() {
  version=${TAG#v}
  tmpdir=$(mktemp -d)
  trap 'rm -rf "$tmpdir"' EXIT INT TERM
  notes_file="${CAPFIND_RELEASE_NOTES_FILE:-$tmpdir/notes.md}"
  if [ ! -f "$notes_file" ]; then
    cat >"$notes_file" <<EOF
## $TAG

Automated fallback release for $TAG.

The GitHub Actions release workflow did not publish this tag in time, so this
release was created from locally built and checksum-verified dist assets.
EOF
  fi

  assets=$(find "$DIST_DIR" -maxdepth 1 -type f \
    \( -name "capfind-v$version-*.tar.gz" -o -name "capfind-v$version-*.tar.gz.sha256" \) \
    | sort)
  [ -n "$assets" ] || fail "no upload assets found in $DIST_DIR for $TAG"

  if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    log "release exists; uploading assets with --clobber"
    gh release upload "$TAG" $assets scripts/install.sh --repo "$REPO" --clobber
  else
    log "creating fallback GitHub Release for $TAG"
    gh release create "$TAG" $assets scripts/install.sh \
      --repo "$REPO" \
      --title "$TAG" \
      --notes-file "$notes_file" \
      --latest
  fi
}

verify_release() {
  gh release view "$TAG" \
    --repo "$REPO" \
    --json tagName,url,assets,publishedAt,isDraft,isPrerelease \
    --jq '{tagName,url,publishedAt,isDraft,isPrerelease,assets:[.assets[].name]}'
}

need_cmd git
need_cmd gh
need_cmd find
need_cmd sort
need_cmd mktemp

TAG=$(resolve_tag "${1:-}")
case "$TAG" in
  v*) ;;
  *) fail "release tag must start with v: $TAG" ;;
esac

git rev-parse --verify "refs/tags/$TAG" >/dev/null || fail "local tag not found: $TAG"

if [ "${CAPFIND_RELEASE_DRY_RUN:-0}" = "1" ]; then
  log "dry run for $TAG"
  if [ -d "$DIST_DIR" ]; then
    verify_dist_assets
  else
    log "dry run: $DIST_DIR does not exist; skipping asset verification"
  fi
  log "dry run: would ensure remote tag, wait for $WORKFLOW, then publish fallback assets if needed"
  exit 0
fi

if ! git ls-remote --exit-code origin "refs/tags/$TAG" >/dev/null 2>&1; then
  log "pushing tag $TAG"
  git push origin "refs/tags/$TAG"
else
  log "remote tag already exists: $TAG"
fi

if [ "${CAPFIND_RELEASE_MANUAL:-0}" = "1" ]; then
  verify_dist_assets
  manual_release
else
  if wait_for_release_workflow; then
    log "release workflow completed"
  else
    log "release workflow did not complete from tag push; using manual fallback"
    verify_dist_assets
    manual_release
  fi
fi

verify_release
