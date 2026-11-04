#!/usr/bin/env bash
# The pinned versions of the tools Bootstrap installs outside the tree, read from the one
# place each is pinned, as `name=version` records in $GITHUB_OUTPUT and one line on stdout.
# ci.yml keys each tool's cache on its record, so a pin that moves is a cache miss rather
# than a stale hit.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
release_plz="$(sed -n 's/^readonly RELEASE_PLZ_VERSION=//p' "$ROOT/scripts/scoped-release-plz.sh" | tr -d '\r')"
prune_superseded="$(sed -n 's/^prune_superseded_version := "\([^"]*\)"$/\1/p' "$ROOT/justfile" | tr -d '\r')"
# One version each, so a key is one value and $GITHUB_OUTPUT gains exactly one record.
single_version() {
  case "$2" in
    "" | *[!0-9A-Za-z.+-]*) ;;
    *)
      if printf '%s\n' "$2" | grep -Eqx '(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?'; then
        return 0
      fi
      ;;
  esac
  echo "ci-tool-pins: no single pinned version for $1 where it is pinned (read '$2')." >&2
  echo "ci-tool-pins: next: restore the named exact version in .tool-versions, scripts/scoped-release-plz.sh or the justfile." >&2
  exit 1
}
single_version release-plz "$release_plz"
single_version prune-superseded "$prune_superseded"
pins="release-plz=$release_plz prune-superseded=$prune_superseded"
for tool in just uv cargo-nextest cargo-llvm-cov cargo-machete cargo-deny cargo-binstall; do
  version="$(awk -v tool="$tool" '$1 == tool { if (NF == 2) print $2; else print "invalid tool entry" }' "$ROOT/.tool-versions" | tr -d '\r')"
  single_version "$tool" "$version"
  pins="$pins $tool=$version"
done
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  printf '%s\n' "$pins" | tr ' ' '\n' >>"$GITHUB_OUTPUT"
fi
echo "ci-tool-pins: $pins"
