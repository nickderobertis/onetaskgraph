#!/usr/bin/env bash
# Build the reachability-closure prototype: onetaskgraph at the commit this spike measured
# (main at acd090b4) with reach-closure.patch applied, in a clone under the spike's cache, so
# nothing in this checkout changes. `spike/gaps.sh reach-* --build closure` runs what it builds.
#
#   spike/gaps/prototype/build.sh [<base commit>]
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
base="${1:-acd090b4}"
clone="${XDG_CACHE_HOME:-$HOME/.cache}/board-copy-gaps-2026-10-24/prototype"
if [ ! -d "$clone/.git" ]; then
    mkdir -p "$(dirname "$clone")"
    git clone -q --no-checkout "$repo" "$clone"
fi
git -C "$clone" fetch -q origin
git -C "$clone" checkout -q --force "$base"
if ! git -C "$clone" apply "$here/reach-closure.patch"; then
    echo "build.sh: reach-closure.patch does not apply to $base; pass the commit it was cut from (acd090b4) or re-cut the patch there" >&2
    exit 1
fi
(cd "$clone" && cargo build --locked -q -p onetaskgraph --bin onetaskgraph)
