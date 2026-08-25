#!/usr/bin/env bash
# Answers whether one crate version is already on crates.io, for the release workflow.
# The registry refuses curl's default user agent with 403 — an answer that says nothing
# about the crate — so the query names this release and a contact URL for it.
set -euo pipefail
usage() { echo "usage: scripts/crate-publication-status.sh CRATE VERSION" >&2; echo "next: $1" >&2; exit 64; }
[[ $# -eq 2 ]] || usage "name one crate and its X.Y.Z version, as .github/workflows/release.yml does"
crate=$1
version=$2
base=${ONETASKGRAPH_CRATES_API_BASE_URL:-https://crates.io/api/v1/crates}
agent="onetaskgraph-release (https://github.com/nickderobertis/onetaskgraph)"
# Both arguments become path segments of the query below, so they are checked against the
# grammars crates.io accepts rather than trusted from the caller.
[[ $crate =~ ^[A-Za-z0-9][A-Za-z0-9_-]*$ ]] || usage "invalid crate name: $crate"
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] || usage "invalid version: $version"
[[ $base =~ ^https?://[^[:space:]]+$ ]] || usage "invalid registry base, which must be an http:// or https:// URL: $base"
url="$base/$crate/$version"
status=$(curl -sS -o /dev/null -w '%{http_code}' --user-agent "$agent" "$url") || {
  echo "could not reach crates.io for $crate $version" >&2
  echo "next: confirm the runner can reach $base and rerun the release" >&2
  exit 69
}
case "$status" in
  200) echo published;;
  404) echo absent;;
  403)
    echo "crates.io declined the caller for $crate $version (HTTP 403), so whether it is published is unknown" >&2
    echo "next: check that '$agent' still satisfies the crates.io user-agent policy and rerun the release" >&2
    exit 69;;
  *)
    echo "crates.io answered HTTP $status for $crate $version, which does not say whether it is published" >&2
    echo "next: request $url by hand and rerun the release once the registry answers 200 or 404" >&2
    exit 69;;
esac
