#!/usr/bin/env bash
set -euo pipefail
fail() { echo "distribution contract drift: $1" >&2; echo "next: update the release matrix, installer, launcher, and carrier manifests together" >&2; exit 1; }
# The path is assembled from BASH_SOURCE at runtime, so shellcheck cannot resolve it.
# Naming the file has it follow and check read-lines.sh (SC1091) rather than skip it unread.
# shellcheck source=scripts/read-lines.sh
source "$(dirname "${BASH_SOURCE[0]}")/read-lines.sh" || { echo "distribution contract drift: could not load scripts/read-lines.sh, which reads both inventories below into arrays" >&2; echo "next: restore it with 'git checkout -- scripts/read-lines.sh'" >&2; exit 1; }
expected=(darwin-arm64 darwin-x64 linux-arm64 linux-x64 win32-x64)
packages_file="$(mktemp)" || fail "could not create a temporary carrier inventory"
trap 'rm -f "$packages_file"' EXIT
if ! find npm/platforms -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | sort > "$packages_file"; then
  fail "could not inspect npm/platforms"
fi
read_lines packages < "$packages_file"
[[ ${packages[*]} == "${expected[*]}" ]] || fail "npm carriers are '${packages[*]}', expected '${expected[*]}'"
release_tag_pattern=$(sed -n "/invalid release tag:/s/.*grep -Eq '\([^']*\)'.*/\1/p" .github/workflows/release.yml)
installer_tag_pattern=$(sed -n "/unsupported release tag:/s/.*grep -Eq '\([^']*\)'.*/\1/p" scripts/install.sh)
[[ -n $release_tag_pattern && $release_tag_pattern == "$installer_tag_pattern" ]] || fail "release workflow and installer accept different release-tag grammars"
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
wheel_job=$(sed -n '/^  build-wheels:/,/^  build-distributions:/p' .github/workflows/release.yml)
grep -Fq 'runs-on: ${{ matrix.os }}' <<< "$wheel_job" || fail "build-wheels must take its runner from matrix.os"
[[ $(grep -Fc -- '- { os:' <<< "$wheel_job") -eq 5 ]] || fail "build-wheels must contain exactly five target/runner entries"
while read -r os target; do
  grep -Fq -- "- { os: $os, target: $target }" <<< "$wheel_job" || fail "build-wheels does not map $target to $os"
done <<'WHEEL_MAPPINGS'
ubuntu-latest x86_64-unknown-linux-gnu
ubuntu-24.04-arm aarch64-unknown-linux-gnu
macos-latest x86_64-apple-darwin
macos-latest aarch64-apple-darwin
windows-latest x86_64-pc-windows-msvc
WHEEL_MAPPINGS
grep -Fq 'gh release upload "$TAG" "$asset" "$asset.sha256" --clobber' .github/workflows/release.yml || fail "release asset uploads must replace assets left by an earlier attempt"
crate_job=$(sed -n '/^  publish-crates:/,/^  publish-python:/p' .github/workflows/release.yml)
grep -Fq 'publication=$(scripts/crate-publication-status.sh "$crate" "$version") || exit $?' <<< "$crate_job" || fail "crate publication must decide from scripts/crate-publication-status.sh, which identifies the caller to crates.io"
! grep -Eq 'curl|wget' <<< "$crate_job" || fail "the crates.io existence query must stay in scripts/crate-publication-status.sh, where the caller is identified to the registry"
grep -Fq 'published) ;;' <<< "$crate_job" || fail "a crate already on crates.io must be left alone"
grep -Fq 'absent) RUSTFLAGS=' <<< "$crate_job" || fail "a crate absent from crates.io must be published"
grep -Fq -- '--user-agent "$agent"' scripts/crate-publication-status.sh || fail "the crates.io existence query must send an explicit user agent; the registry answers curl's default with 403"
grep -Fq 'agent="onetaskgraph-release (https://github.com/nickderobertis/onetaskgraph)"' scripts/crate-publication-status.sh || fail "the crates.io user agent must name this release and a contact URL for it"
grep -Fq 'NPM_TOKEN is required (received ${#NODE_AUTH_TOKEN} characters)' .github/workflows/release.yml || fail "the npm token guard must report only the received token length"
if ! node <<'NODE'
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
then
  fail "could not reconcile launcher and carrier manifests"
fi
[[ $(grep -Fc 'git_tag_name = "v{{ version }}"' release-plz.toml) -eq 1 ]] || fail "the binary must be the only package using the plain v-prefixed tag"
[[ $(grep -Fc 'git_release_name = "v{{ version }}"' release-plz.toml) -eq 1 ]] || fail "the GitHub Release name must match the binary's plain v-prefixed tag"
if ! python3 <<'PY'
import re
import tomllib

with open("release-plz.toml", "rb") as manifest:
    policy = re.compile(tomllib.load(manifest)["workspace"]["release_commits"])

eligible = ("feat: add source", "fix(cli): handle failure", "perf!: remove bottleneck", "docs: explain\n\nBREAKING CHANGE: remove old API")
ineligible = ("chore(onetaskgraph): release v1.2.3", "docs: explain setup", "test: cover release")
if any(policy.search(commit) is None for commit in eligible):
    raise SystemExit("release-worthy Conventional Commit was rejected")
if any(policy.search(commit) is not None for commit in ineligible):
    raise SystemExit("non-release commit was accepted")
PY
then
  fail "release-plz commit eligibility does not match the release policy"
fi
grep -q 'npm pack ./carrier' .github/workflows/release.yml || fail "release workflow does not build npm carrier tarballs"
grep -q 'cp "$bin"' .github/workflows/release.yml || fail "release workflow does not put native binaries in npm carriers"
grep -q 'pattern: "carrier-\*"' .github/workflows/release.yml || fail "npm publish does not download built carrier tarballs"
read_lines crates < <(for manifest in crates/*/Cargo.toml; do basename "$(dirname "$manifest")"; done | sort)
for crate in "${crates[@]}"; do grep -q "name = \"$crate\"" release-plz.toml || [[ $crate == onetaskgraph ]] || fail "$crate missing from release-plz package inventory"; grep -q "for crate in .*$crate" .github/workflows/release.yml || fail "$crate missing from publish order"; done
