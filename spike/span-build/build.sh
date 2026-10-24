#!/usr/bin/env bash
# Build the span build: onetaskgraph's v0.7.1 tag with spans-v0.7.1.patch applied and
# nothing else changed, released as the wheel is (`cargo build --release -p onetaskgraph`).
# The tree is a `git archive` of the tag, so no worktree is registered anywhere.
# Usage: build.sh [DIR]  (default: the spike's cache directory, outside every tree)
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
dir="${1:-${XDG_CACHE_HOME:-$HOME/.cache}/board-copy-speed-2026-10-18/span-build}"
patch_sum="$(sha256sum "$here/spans-v0.7.1.patch" | cut -d' ' -f1)"
if [ ! -f "$dir/.spike-patch" ] || [ "$(cat "$dir/.spike-patch")" != "$patch_sum" ]; then
    git -C "$repo" rev-parse -q --verify 'v0.7.1^{commit}' > /dev/null \
        || git -C "$repo" fetch -q origin tag v0.7.1
    rm -rf "$dir.tmp"
    mkdir -p "$dir.tmp"
    git -C "$repo" archive --format=tar -o "$dir.tmp.tar" v0.7.1
    tar -xf "$dir.tmp.tar" -C "$dir.tmp"
    rm "$dir.tmp.tar"
    (cd "$dir.tmp" && git apply "$here/spans-v0.7.1.patch")
    if [ -d "$dir/target" ]; then mv "$dir/target" "$dir.tmp/target"; fi
    rm -rf "$dir"
    mv "$dir.tmp" "$dir"
    echo "$patch_sum" > "$dir/.spike-patch"
fi
(cd "$dir" && cargo build -q --release -p onetaskgraph)
echo "$dir/target/release/onetaskgraph"
