#!/usr/bin/env bash
set -euo pipefail
fail() { echo "distribution contract drift: $1" >&2; echo "next: update the release matrix, installer, launcher, and carrier manifests together" >&2; exit 1; }
expected=(darwin-arm64 darwin-x64 linux-arm64 linux-x64 win32-x64)
mapfile -t packages < <(find npm/platforms -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort)
[[ ${packages[*]} == "${expected[*]}" ]] || fail "npm carriers are '${packages[*]}', expected '${expected[*]}'"
for value in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin x86_64-pc-windows-msvc; do
  grep -q "$value" scripts/install.sh || fail "$value missing from installer"
  grep -q "$value" .github/workflows/release.yml || fail "$value missing from release matrix"
done
for value in linux-x64 linux-arm64 darwin-x64 darwin-arm64 win32-x64; do grep -q "$value" npm/cli/bin/onetaskgraph.js || fail "$value missing from launcher"; done
mapfile -t crates < <(for manifest in crates/*/Cargo.toml; do basename "$(dirname "$manifest")"; done | sort)
for crate in "${crates[@]}"; do grep -q "name = \"$crate\"" release-plz.toml || [[ $crate == onetaskgraph ]] || fail "$crate missing from release-plz package inventory"; grep -q "for crate in .*$crate" .github/workflows/release.yml || fail "$crate missing from publish order"; done
