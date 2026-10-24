#!/usr/bin/env bash
# Build the reachability-closure prototype: onetaskgraph at the commit this spike measured
# (main at acd090b4) with reach-closure.patch applied, in a clone under the spike's cache, so
# nothing in this checkout changes. `run.py reach-* --build closure` runs what it builds.
#
#   spike/prototype/build.sh [<base commit>]
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
base="${1:-acd090b4}"
cache="${XDG_CACHE_HOME:-$HOME/.cache}/${SPIKE_CACHE_NAME:-board-copy-gaps-2026-10-24}/prototype"
if [ ! -d "$cache/.git" ]; then
    mkdir -p "$(dirname "$cache")"
    git clone -q --no-checkout "$repo" "$cache"
fi
git -C "$cache" fetch -q origin
git -C "$cache" checkout -q --force "$base"
git -C "$cache" apply "$here/reach-closure.patch"
(cd "$cache" && cargo build --locked -q -p onetaskgraph --bin onetaskgraph)
echo "built $cache/target/debug/onetaskgraph ($("$cache/target/debug/onetaskgraph" --version))"
