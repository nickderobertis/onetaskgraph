#!/usr/bin/env bash
# Open (or update) the release pull request with EVERY version manifest in agreement.
#
# release-plz writes the Cargo manifests and the changelogs. It writes no package.json, no
# pyproject.toml and no workspace.package version, and it has no hook that could run
# something that would: `release-plz generate-schema` for 0.3.160 lists no hook key at all,
# and its config refuses one by name — `unknown field 'pre_release_hook'` — because both
# config tables are declared with additionalProperties false. So the release pull request
# release-plz opens on its own carries npm/, sdks/ and pyproject.toml at the PREVIOUS
# version, and `just distribution-check` — part of the required `check` on all three
# platforms — refuses it with "npm/cli/package.json has 0.1.0; expected 0.2.0". That is not
# a hypothetical: it is what blocked v0.2.0's release pull request (#26), and it blocks
# every bump after it, because v0.1.0 was the only release this repository could cut
# without one.
#
# So the bump and the sync happen here, before the pull request exists:
#
#   1. `release-plz update` writes the Cargo side and decides the next version,
#   2. `scripts/set-version.sh` brings every other manifest and lockfile to that version,
#   3. `release-plz release-pr --allow-dirty` carries that whole tree into the release
#      commit — "the uncommitted changes will be part of the update", in its own words.
#
# Step 3 is the part that has to be true of the installed tool rather than assumed, so it
# was observed rather than read: driven against a forge stand-in on 2026-08-26, release-plz
# 0.3.160 sent all 26 files — npm/cli/package.json, npm/platforms/*/package.json,
# pyproject.toml, sdks/python/pyproject.toml, sdks/typescript/package.json and uv.lock among
# them, each at the new version — in the createCommitOnBranch mutation that becomes the
# release commit. Re-deciding the version over the already-bumped tree in step 3 is
# idempotent by the tool's own rule, which it states as it applies it: "local version
# (0.2.0) > registry version (0.1.0). Only changelog will be updated."
#
# scripts/check-release-pr-sync.sh drives this script end to end against a stand-in for
# release-plz on every `just check`, and refuses a workflow that goes around it.
set -euo pipefail

fail() {
  echo "prepare-release-pr: $1" >&2
  echo "prepare-release-pr: next: $2" >&2
  exit 1
}

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd) || fail \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run this from a checkout of this repository, as .github/workflows/release-plz.yml does"
cd "$root" || fail \
  "could not enter $root" \
  "check that the checkout is readable, then rerun"

command -v release-plz >/dev/null 2>&1 || fail \
  "release-plz is not on PATH, so no version can be decided" \
  "install it — the release-plz workflow does, with taiki-e/install-action — then rerun"

release-plz update || fail \
  "release-plz could not decide the next version (its own diagnostic is above)" \
  "fix what it reports; a registry it cannot reach and a manifest it cannot parse both land here"

binary_manifest=crates/onetaskgraph/Cargo.toml
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$binary_manifest" | head -n1)
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] || fail \
  "$binary_manifest has no valid semantic version after the update: '$version'" \
  "restore its X.Y.Z version and rerun; that manifest is where the release's version is read from"

scripts/set-version.sh "$version" || fail \
  "could not bring every manifest to $version (its own diagnostic is above)" \
  "fix the manifest or lockfile it names, then rerun"

# Refuse to open a pull request this repository's own required check would refuse. Without
# this the drift reaches CI, where it costs a full three-platform run to say the same thing.
scripts/set-version.sh --check || fail \
  "the manifests still disagree after the sync, so the release pull request would fail its own distribution-check" \
  "fix what set-version.sh named above, then rerun"

# --allow-dirty is what carries the sync above into the release commit; without it
# release-plz refuses to run at all on a tree it did not write itself.
release-plz release-pr --allow-dirty "$@" || fail \
  "release-plz could not open or update the release pull request (its own diagnostic is above)" \
  "check that the git token is present and still authorises pull requests, then rerun"
