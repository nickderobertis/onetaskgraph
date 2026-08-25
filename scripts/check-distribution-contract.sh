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
grep -Fq 'target: [x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-apple-darwin, aarch64-apple-darwin, x86_64-pc-windows-msvc]' .github/workflows/release.yml || fail "wheel targets disagree with the native release matrix"
node <<'NODE'
const fs = require("fs");
function fail(message) {
  console.error(`distribution contract drift: ${message}`);
  console.error("next: update the launcher mapping and carrier manifests together");
  process.exit(1);
}
const platforms = fs.readdirSync("npm/platforms").sort();
const expectedPackages = Object.fromEntries(platforms.map(platform => [platform, platform]));
const launcher = fs.readFileSync("npm/cli/bin/onetaskgraph.js", "utf8");
const packageMapping = launcher.match(/const packages = (\{[^;]+\});/);
const actualPackages = packageMapping && JSON.parse(packageMapping[1]);
if (!actualPackages || JSON.stringify(Object.entries(actualPackages).sort()) !== JSON.stringify(Object.entries(expectedPackages).sort())) fail("npm/cli/bin/onetaskgraph.js platform mapping disagrees with npm/platforms");
const launcherManifest = JSON.parse(fs.readFileSync("npm/cli/package.json"));
const expectedDependencies = platforms.map(platform => `@onetaskgraph/cli-${platform}`).sort();
if (JSON.stringify(Object.keys(launcherManifest.optionalDependencies || {}).sort()) !== JSON.stringify(expectedDependencies)) fail("npm/cli/package.json optionalDependencies disagree with npm/platforms");
for (const platform of platforms) {
  const path = `npm/platforms/${platform}/package.json`;
  const manifest = JSON.parse(fs.readFileSync(path));
  const separator = platform.lastIndexOf("-");
  const os = platform.slice(0, separator);
  const cpu = platform.slice(separator + 1);
  if (manifest.name !== `@onetaskgraph/cli-${platform}`) fail(`${path} name is ${manifest.name}, expected @onetaskgraph/cli-${platform}`);
  if (String(manifest.os) !== os) fail(`${path} os is ${manifest.os}, expected ${os}`);
  if (String(manifest.cpu) !== cpu) fail(`${path} cpu is ${manifest.cpu}, expected ${cpu}`);
}
NODE
grep -q 'npm pack ./carrier' .github/workflows/release.yml || fail "release workflow does not build npm carrier tarballs"
grep -q 'cp "$bin"' .github/workflows/release.yml || fail "release workflow does not put native binaries in npm carriers"
grep -q 'pattern: "carrier-\*"' .github/workflows/release.yml || fail "npm publish does not download built carrier tarballs"
mapfile -t crates < <(for manifest in crates/*/Cargo.toml; do basename "$(dirname "$manifest")"; done | sort)
for crate in "${crates[@]}"; do grep -q "name = \"$crate\"" release-plz.toml || [[ $crate == onetaskgraph ]] || fail "$crate missing from release-plz package inventory"; grep -q "for crate in .*$crate" .github/workflows/release.yml || fail "$crate missing from publish order"; done
