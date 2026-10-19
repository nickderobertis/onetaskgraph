#!/usr/bin/env bash
# The pinned versions of the tools Bootstrap installs outside the tree, read from the one
# place each is pinned, as `name=version` records in $GITHUB_OUTPUT and one line on stdout.
# ci.yml keys each tool's cache on its record, so a pin that moves is a cache miss rather
# than a stale hit.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
release_plz="$(sed -n 's/^readonly RELEASE_PLZ_VERSION=//p' "$ROOT/scripts/scoped-release-plz.sh" | tr -d '\r')"
prune_superseded="$(cd "$ROOT" && just --evaluate prune_superseded_version | tr -d '\r')"
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
  echo "ci-tool-pins: next: restore RELEASE_PLZ_VERSION in scripts/scoped-release-plz.sh and prune_superseded_version in the justfile." >&2
  exit 1
}
single_version release-plz "$release_plz"
single_version prune-superseded "$prune_superseded"
pins="release-plz=$release_plz prune-superseded=$prune_superseded"
[ -z "${GITHUB_OUTPUT:-}" ] || printf '%s\n' $pins >>"$GITHUB_OUTPUT"
echo "ci-tool-pins: $pins"
