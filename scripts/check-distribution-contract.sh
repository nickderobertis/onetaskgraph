#!/usr/bin/env bash
set -euo pipefail
fail() { echo "distribution contract drift: $1" >&2; echo "next: update the release platform table, installer, launcher, and carrier manifests together" >&2; exit 1; }
# The path is assembled from BASH_SOURCE at runtime, so shellcheck cannot resolve it.
# Naming the file has it follow and check read-lines.sh (SC1091) rather than skip it unread.
# shellcheck source=scripts/read-lines.sh
# Tested before it is sourced, not merely guarded after: bash 3.2 ends the shell where
# `source` cannot find its file, so the handler a later bash takes never runs there — and
# macos-latest is a 3.2 runner. Without this the reader gets bash's own "No such file or
# directory", which names the sourcing line rather than the file to put back.
read_lines_path="$(dirname "${BASH_SOURCE[0]}")/read-lines.sh"
if [ ! -r "$read_lines_path" ] || ! source "$read_lines_path"; then echo "distribution contract drift: could not load scripts/read-lines.sh, which reads both inventories below into arrays" >&2; echo "next: restore it with 'git checkout -- scripts/read-lines.sh'" >&2; exit 1; fi
expected=(darwin-arm64 linux-x64 win32-x64)
packages_file="$(mktemp)" || fail "could not create a temporary carrier inventory"
trap 'rm -f "$packages_file"' EXIT
if ! find npm/platforms -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | sort > "$packages_file"; then
  fail "could not inspect npm/platforms"
fi
read_lines packages < "$packages_file"
[[ ${packages[*]} == "${expected[*]}" ]] || fail "npm carriers are '${packages[*]}', expected '${expected[*]}'"
# What a release builds is scripts/release_products.py's platform table, and every CD job
# reads it through scripts/release_cd.py; the installer, the carriers and each build job's
# runner are held to it here. The tag grammar lives there too, and the installer carries a copy.
release_table="$(python3 -c '
import sys
sys.path.insert(0, "scripts")
from release_products import PLATFORM_TARGETS, RELEASE_TAG_PATTERN
print(RELEASE_TAG_PATTERN)
for row in PLATFORM_TARGETS:
    print(row.platform, row.runner, row.target, row.extension, row.npm_platform)
' | tr -d '\r')" || fail "could not read the release platform table from scripts/release_products.py"
release_tag_pattern=$(sed -n 1p <<< "$release_table")
installer_tag_pattern=$(sed -n "/unsupported release tag:/s/.*grep -Eq '\([^']*\)'.*/\1/p" scripts/install.sh)
[[ -n $release_tag_pattern && $release_tag_pattern == "$installer_tag_pattern" ]] || fail "the release platform table and installer accept different release-tag grammars"
mappings=$(sed 1d <<< "$release_table")
[[ $mappings == "linux ubuntu-latest x86_64-unknown-linux-gnu tar.gz linux-x64
macos macos-latest aarch64-apple-darwin tar.gz darwin-arm64
windows windows-latest x86_64-pc-windows-msvc zip win32-x64" ]] || fail "the release platform table builds '${mappings//$'\n'/; }', not x86_64 Linux, Apple Silicon and x86_64 Windows, one job each"
# Read before it is judged: under `set -e` a sed that cannot open the workflow would end the
# script here on sed's own diagnostic, and the refusal below — which is where the next action
# lives — would never run.
workflow=$(sed -n '1,$p' .github/workflows/release.yml) || { echo "distribution contract drift: could not read .github/workflows/release.yml, which carries the build jobs" >&2; echo "next: restore it with 'git checkout -- .github/workflows/release.yml'" >&2; exit 1; }
while read -r platform runner target ext npm; do
  grep -Fq "$target; ext=$ext" scripts/install.sh || fail "$target is not mapped to $ext in the installer"
  job=$(sed -n "/^  build-$platform:\$/,/^  [a-z-]*:\$/p" <<< "$workflow")
  [[ -n $job ]] || fail "the release workflow has no build-$platform job"
  grep -Fq "runs-on: $runner" <<< "$job" || fail "the build-$platform job must run on $runner"
  grep -Fq "with: { target: $target }" <<< "$job" || fail "the build-$platform job must install the $target Rust target"
  grep -Fq "run: just release-$platform build --tag" <<< "$job" || fail "the build-$platform job must build, package and test-install through 'just release-$platform build'"
done <<< "$mappings"
! grep -RqiE 'x86_64-apple-darwin|darwin-x64|rosetta' .github/workflows release-targets.toml npm/platforms npm/cli/package.json scripts/product_versions.py scripts/verify-release-products.py scripts/release_products.py scripts/release_cd.py || fail "an Intel macOS target, carrier or Rosetta step is still declared; Intel macOS is no longer released"
grep -Fq 'Darwin:x86_64) die 69 "Intel macOS is no longer supported' scripts/install.sh || fail "the installer must refuse Intel macOS by name"
! grep -RqiE 'aarch64-unknown-linux-gnu|linux-arm64|ubuntu-24.04-arm' .github/workflows release-targets.toml npm/platforms npm/cli/package.json scripts/product_versions.py scripts/release_products.py scripts/release_cd.py justfile || fail "a Linux arm64 target, carrier or runner is still declared; Linux arm64 is no longer released"
grep -Fq "Linux:aarch64|Linux:arm64) die 69 \"Linux arm64 is no longer supported" scripts/install.sh || fail "the installer must refuse Linux arm64 by name"
# Maturin owns the canonical bytes, and every other form is staged from them: the wheel is
# built, its binary taken out, the archive and carrier packed from that, and all of it
# installed and run — in that order, inside the build phase, before any publication.
python3 - <<'PY' || fail "scripts/release_cd.py must build the wheel, take its binary out, pack the archive and carrier from it, and install and run them, in that order, before publishing"
import re
from pathlib import Path

driver = Path("scripts/release_cd.py").read_text(encoding="utf-8")
build = re.search(r"(?ms)^def build_target\(.*?(?=^def )", driver)
package = re.search(r"(?ms)^def package_target\(.*?(?=^def )", driver)
if not build or not package:
    raise SystemExit(1)
in_order = lambda body, steps: all(body.find(step) >= 0 for step in steps) and [body.find(step) for step in steps] == sorted(body.find(step) for step in steps)
built = in_order(build.group(0), ['"maturin", "build"', "package_target(row, tag, dist)", "scripts/verify-release-products.py", '"--install"'])
packed = in_order(package.group(0), ["scripts/release-wheel-binary.py", "tarfile.open(archive", '"pack"'])
raise SystemExit(0 if built and packed else 1)
PY
crate_body=$(sed -n '/^def publish_crates(/,/^def /p' scripts/release_cd.py)
grep -Fq '"scripts/crate-publication-status.sh", crate, version_of(tag)' <<< "$crate_body" || fail "crate publication must decide from scripts/crate-publication-status.sh, which identifies the caller to crates.io"
! grep -Eq 'curl|wget' scripts/release_cd.py || fail "the crates.io existence query must stay in scripts/crate-publication-status.sh, where the caller is identified to the registry"
grep -Fq 'if status == "published":' <<< "$crate_body" || fail "a crate already on crates.io must be left alone"
python3 - <<'PY' || fail "a crate absent from crates.io must be published, and only after scripts/crate-sibling-resolution.sh finds every sibling it requires resolving to the version released with it"
import re
from pathlib import Path

body = re.search(r"(?ms)^def publish_crates\(.*?(?=^def |\Z)", Path("scripts/release_cd.py").read_text(encoding="utf-8"))
text = body.group(0) if body else ""
resolution, publication = text.find('"scripts/crate-sibling-resolution.sh", crate'), text.find('"publish", "-p", crate')
raise SystemExit(0 if 0 <= resolution < publication else 1)
PY
grep -Fq -- '--user-agent "$agent"' scripts/crate-publication-status.sh || fail "the crates.io existence query must send an explicit user agent; the registry answers curl's default with 403"
grep -Fq 'agent="onetaskgraph-release (https://github.com/nickderobertis/onetaskgraph)"' scripts/crate-publication-status.sh || fail "the crates.io user agent must name this release and a contact URL for it"
grep -Fq 'NPM_TOKEN is required (received ${#token} characters)' scripts/publish-npm.sh || fail "the npm token guard must report only the received token length"
grep -Fq 'token=${NODE_AUTH_TOKEN:-}' scripts/publish-npm.sh || fail "the npm token must be read through a default before its length is taken; under 'set -u' an unset one ends the script on bash's own diagnostic instead of the guard's message"
# npm authentication drift is repaired in the publication path rather than in the
# manifests the generic next action names, so it reports its own.
fail_npm_auth() { echo "distribution contract drift: $1" >&2; echo "next: restore the npm registry authentication in .github/workflows/release.yml, scripts/publish-npm.sh and scripts/npm-registry-auth.sh together" >&2; exit 1; }
npm_steps=$(grep -F 'publish --to npm' <<< "$workflow" || true)
[[ -n $npm_steps ]] || fail_npm_auth "the release workflow has no npm publication step to authenticate"
# The publication itself lives in scripts/publish-npm.sh, which scripts/check-npm-publish.sh
# drives against a registry of its own on every gate run and scripts/release_cd.py runs for
# each job — so every step publishing to npm goes through a recipe, never a copy inline.
[[ $(grep -Fc 'run: just release-' <<< "$npm_steps") -eq $(wc -l <<< "$npm_steps") ]] || fail_npm_auth "every npm publication step must run 'just release-<job> publish --to npm', which runs scripts/publish-npm.sh — the publication scripts/check-npm-publish.sh exercises"
! grep -Fq 'npm publish' <<< "$workflow" || fail_npm_auth "the release workflow must not publish inline; scripts/publish-npm.sh is the one publication path anything can drive"
grep -Fq '[bash, "scripts/publish-npm.sh"]' scripts/release_cd.py || fail_npm_auth "scripts/release_cd.py must publish to npm through scripts/publish-npm.sh"
! grep -E 'tool\("npm"\)|npm_command\(' scripts/release_cd.py | grep -Fq '"publish"' || fail_npm_auth "scripts/release_cd.py must not run npm publish itself; scripts/publish-npm.sh is the one publication path anything can drive"
# The public registry is the default and nothing in the workflow may choose another: the
# override exists so the checks and journeys can point one run at their own stub.
! grep -Fq 'NPM_REGISTRY' <<< "$workflow" || fail_npm_auth "the release workflow must not set NPM_REGISTRY; the public registry is scripts/publish-npm.sh's default"
grep -Fq 'registry=${NPM_REGISTRY:-https://registry.npmjs.org/}' scripts/publish-npm.sh || fail_npm_auth "scripts/publish-npm.sh must default to the public npm registry"
grep -Fq 'NPM_CONFIG_USERCONFIG=$(scripts/npm-registry-auth.sh "$registry")' scripts/publish-npm.sh || fail_npm_auth "npm publication must configure registry authentication with scripts/npm-registry-auth.sh; NODE_AUTH_TOKEN alone leaves the npm client logged out"
grep -Fq 'export NPM_CONFIG_USERCONFIG' scripts/publish-npm.sh || fail_npm_auth "the npm configuration must be exported as NPM_CONFIG_USERCONFIG, which is how the npm client finds it"
grep -Fq ':_authToken=${NODE_AUTH_TOKEN}' scripts/npm-registry-auth.sh || fail_npm_auth "the npm configuration must name NODE_AUTH_TOKEN rather than carry a token value"
if ! python3 <<'PY'
import pathlib
import shlex

workflow = pathlib.Path("scripts/publish-npm.sh").read_text(encoding="utf-8")
expected = {
    "@onetaskgraph/cli@$cli_version": pathlib.Path("./npm/cli"),
    "@onetaskgraph/sdk@$sdk_version": pathlib.Path("./sdks/typescript"),
}
found = {}
for line in workflow.splitlines():
    stripped = line.strip()
    if not stripped.startswith("publish_if_absent "):
        continue
    arguments = shlex.split(stripped)
    if len(arguments) == 3 and arguments[1] in expected:
        found[arguments[1]] = arguments[2]

for spec, expected_path in expected.items():
    argument = found.get(spec)
    if argument is None:
        raise SystemExit(f"{spec} has no publish_if_absent call")
    if not argument.startswith("./"):
        raise SystemExit(
            f"{spec} publish argument {argument!r} is not an explicit checkout-local path"
        )
    path = pathlib.Path(argument)
    if path != expected_path or not path.is_dir():
        raise SystemExit(
            f"{spec} publish argument {argument!r} is not its local package directory"
        )
PY
then
  echo "distribution contract drift: installable npm packages must publish from explicit local directories in this checkout" >&2
  echo "next: restore the ./npm/cli and ./sdks/typescript publish operands in scripts/publish-npm.sh" >&2
  exit 1
fi
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
crate_order=$(python3 -c 'import sys; sys.path.insert(0, "scripts"); from release_cd import CRATE_PUBLICATION_ORDER; print(" ".join(CRATE_PUBLICATION_ORDER))' | tr -d '\r') || fail "could not read the crate publication order from scripts/release_cd.py"
read_lines crates < <(for manifest in crates/*/Cargo.toml; do basename "$(dirname "$manifest")"; done | sort)
# Both ways. A crate this workspace publishes has to be in release-plz's inventory and in
# the publish order, or a release ships without it; a crate marked `publish = false` — the
# workspace's own test-support crates, which nothing installs — has to be in NEITHER, or a
# release job would try to push a crate crates.io was never told about and fail the whole
# publish. The manifest is what says which a crate is, so nothing here transcribes it.
for crate in "${crates[@]}"; do
  if grep -Fxq 'publish = false' "crates/$crate/Cargo.toml"; then
    if grep -q "name = \"$crate\"" release-plz.toml; then fail "$crate is publish = false but is in the release-plz package inventory"; fi
    if [[ " $crate_order " == *" $crate "* ]]; then fail "$crate is publish = false but is in the publish order"; fi
    continue
  fi
  grep -q "name = \"$crate\"" release-plz.toml || [[ $crate == onetaskgraph ]] || fail "$crate missing from release-plz package inventory"
  [[ " $crate_order " == *" $crate "* ]] || fail "$crate missing from publish order"
done
# Every edge between two crates this workspace publishes is an exact requirement on the
# version the other is released at. These pre-1.0 crates are lock-step, and a caret lets a
# consumer resolve one against another's newer, incompatible patch with no lockfile error.
# Last, after the inventory above: this is the one read that asks cargo to load the whole
# workspace, and a crate directory cargo cannot load is named by the inventory's own
# diagnostic rather than by cargo's manifest error — scripts/check-line-reads.sh plants
# exactly such a directory and reads the inventory's answer.
# Neither failure is repaired in the matrices and manifests the generic next action names,
# so each reports its own.
fail_siblings() { echo "distribution contract drift: $1" >&2; echo "next: $2" >&2; exit 1; }
if ! inexact="$(cargo metadata --no-deps --format-version 1 | python3 -c '
import json, sys
packages = {package["name"]: package for package in json.load(sys.stdin)["packages"]}
for package in packages.values():
    if package.get("publish") == []:
        continue
    for dependency in package["dependencies"]:
        sibling = packages.get(dependency["name"])
        if sibling is None or sibling.get("publish") == []:
            continue
        wanted = "=" + sibling["version"]
        if dependency["req"] != wanted:
            print(package["name"], "requires", dependency["name"], "as", dependency["req"], "rather than", wanted)
' | tr -d '\r')"; then
  fail_siblings "could not read the workspace's sibling requirements from cargo metadata" \
    "fix the manifest error cargo printed above, then rerun"
fi
[[ -z $inexact ]] || fail_siblings "sibling requirements must be exact, '=<workspace version>', in the root Cargo.toml's [workspace.dependencies]: ${inexact//$'\n'/; }" \
  "run scripts/set-version.sh <workspace version>, which writes every sibling pin, then rerun"
