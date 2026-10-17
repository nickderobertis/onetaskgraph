#!/usr/bin/env bash
# The pinned versions of the tools Bootstrap installs outside the tree, read from the one
# place each is pinned, as `name=version` records in $GITHUB_OUTPUT and one line on stdout.
# ci.yml keys each tool's cache on its record, so a pin that moves is a cache miss rather
# than a stale hit.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
release_plz="$(sed -n 's/^readonly RELEASE_PLZ_VERSION=//p' "$ROOT/scripts/scoped-release-plz.sh" | tr -d '\r')"
prune_superseded="$(cd "$ROOT" && just --evaluate prune_superseded_version | tr -d '\r')"
pins="release-plz=$release_plz prune-superseded=$prune_superseded"
for pin in $pins; do
  version="${pin#*=}"
  # One version, so a key is one value and $GITHUB_OUTPUT gains exactly one record.
  case "$version" in
    "" | *[!0-9A-Za-z.+-]* | [!0-9]* | *.) valid=false ;;
    *.*.*) valid=true ;;
    *) valid=false ;;
  esac
  if ! "$valid"; then
    echo "ci-tool-pins: no single pinned version for ${pin%%=*} where it is pinned (read '$version')." >&2
    echo "ci-tool-pins: next: restore RELEASE_PLZ_VERSION in scripts/scoped-release-plz.sh and prune_superseded_version in the justfile." >&2
    exit 1
  fi
done
[ -z "${GITHUB_OUTPUT:-}" ] || printf '%s\n' $pins >>"$GITHUB_OUTPUT"
echo "ci-tool-pins: $pins"
