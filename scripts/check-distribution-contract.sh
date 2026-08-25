#!/usr/bin/env bash
set -euo pipefail
fail() { echo "distribution contract drift: $1" >&2; echo "next: update the release matrix, installer, launcher, and carrier manifests together" >&2; exit 1; }
expected=(darwin-arm64 darwin-x64 linux-arm64 linux-x64 win32-x64)
mapfile -t packages < <(find npm/platforms -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort)
[[ ${packages[*]} == "${expected[*]}" ]] || fail "npm carriers are '${packages[*]}', expected '${expected[*]}'"
while read -r os target ext npm; do
  grep -Fq -- "- { os: $os, target: $target, ext: $ext, npm: $npm }" .github/workflows/release.yml || fail "$target is not mapped to $os/$ext/$npm in the release matrix"
  grep -Fq "$target; ext=$ext" scripts/install.sh || fail "$target is not mapped to $ext in the installer"
done <<'MAPPINGS'
ubuntu-latest x86_64-unknown-linux-gnu tar.gz linux-x64
ubuntu-24.04-arm aarch64-unknown-linux-gnu tar.gz linux-arm64
macos-latest x86_64-apple-darwin tar.gz darwin-x64
macos-latest aarch64-apple-darwin tar.gz darwin-arm64
windows-latest x86_64-pc-windows-msvc zip win32-x64
MAPPINGS
for value in linux-x64 linux-arm64 darwin-x64 darwin-arm64 win32-x64; do grep -q "$value" npm/cli/bin/onetaskgraph.js || fail "$value missing from launcher"; done
node <<'NODE' || fail "an npm carrier manifest disagrees with its directory"
const fs = require("fs");
for (const platform of fs.readdirSync("npm/platforms")) {
  const manifest = JSON.parse(fs.readFileSync(`npm/platforms/${platform}/package.json`));
  const separator = platform.lastIndexOf("-");
  const os = platform.slice(0, separator);
  const cpu = platform.slice(separator + 1);
  if (manifest.name !== `@onetaskgraph/cli-${platform}` || String(manifest.os) !== os || String(manifest.cpu) !== cpu) process.exit(1);
}
NODE
grep -q 'npm pack ./carrier' .github/workflows/release.yml || fail "release workflow does not build npm carrier tarballs"
grep -q 'cp "$bin"' .github/workflows/release.yml || fail "release workflow does not put native binaries in npm carriers"
grep -q 'pattern: "carrier-\*"' .github/workflows/release.yml || fail "npm publish does not download built carrier tarballs"
mapfile -t crates < <(for manifest in crates/*/Cargo.toml; do basename "$(dirname "$manifest")"; done | sort)
for crate in "${crates[@]}"; do grep -q "name = \"$crate\"" release-plz.toml || [[ $crate == onetaskgraph ]] || fail "$crate missing from release-plz package inventory"; grep -q "for crate in .*$crate" .github/workflows/release.yml || fail "$crate missing from publish order"; done
