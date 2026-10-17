#!/usr/bin/env bash
# The pinned versions of the tools Bootstrap installs outside the tree, read from the one
# place each is pinned, as `name=version` lines on stdout and in $GITHUB_OUTPUT. ci.yml keys
# each tool's cache on its line, so a pin that moves is a cache miss rather than a stale hit.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
release_plz="$(sed -n 's/^readonly RELEASE_PLZ_VERSION=//p' "$ROOT/scripts/scoped-release-plz.sh" | tr -d '\r')"
prune_superseded="$(cd "$ROOT" && just --evaluate prune_superseded_version | tr -d '\r')"
for pin in "release-plz=$release_plz" "prune-superseded=$prune_superseded"; do
  if [ -z "${pin#*=}" ]; then
    echo "ci-tool-pins: no pinned version for ${pin%%=*} where it is pinned." >&2
    echo "ci-tool-pins: next: restore RELEASE_PLZ_VERSION in scripts/scoped-release-plz.sh and prune_superseded_version in the justfile." >&2
    exit 1
  fi
  echo "$pin"
  [ -z "${GITHUB_OUTPUT:-}" ] || echo "$pin" >>"$GITHUB_OUTPUT"
done
