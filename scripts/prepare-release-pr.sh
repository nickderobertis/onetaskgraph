#!/usr/bin/env bash
# Open or update the release pull request with every version manifest in agreement.
#
# release-plz writes the Cargo manifests and nothing else — no package.json, no
# pyproject.toml, not even the version under [workspace.package] — and no hook can be asked
# to: it refuses `pre_release_hook` as an unknown field and its own `generate-schema` lists
# no other. A pull request it opens alone carries the rest at the previous version and
# fails the `distribution-check` its own merge waits on, which is what blocked v0.2.0 (#26).
#
# So the sync happens before the pull request exists, and `--allow-dirty` is what carries it
# into the release commit: release-plz builds that commit from what differs from HEAD, and
# refuses the tree outright without the flag. scripts/check-release-pr-sync.sh drives all of
# this end to end on every `just check`.
#
# Exit status, which the release workflow reads and a reader of its log has to be able to
# tell apart: 0, the release pull request is open and its manifests agree; 2, this script
# was called wrongly or the environment it needs is missing, and nothing was attempted; 1, a
# phase failed, with everything that phase printed above the diagnostic. The split follows
# scripts/set-version.sh, where 2 is likewise "the call was wrong".
set -euo pipefail

# fail <problem> <next action> [exit status, 1 by default]
fail() {
  echo "prepare-release-pr: $1" >&2
  echo "prepare-release-pr: next: $2" >&2
  exit "${3:-1}"
}

# The git token is read from the environment rather than an argument: an argument list is
# readable by every process on the runner. Nothing here ever prints the value.
[ $# -eq 0 ] || fail \
  "this script takes no arguments and received $#" \
  "pass the token as GIT_TOKEN in the environment, which is where release-plz reads it from" 2
[ -n "${GIT_TOKEN:-}" ] || fail \
  "GIT_TOKEN is empty, so release-plz could not open a pull request" \
  "set it from this repository's RELEASE_PLZ_TOKEN secret, as .github/workflows/release-plz.yml does" 2

# llmlint: ignore[changed_behavior_has_e2e] Driving this refusal means a checkout whose own
# directory cannot be entered, which no case can arrange without breaking the runner it runs on.
cd "$(dirname "${BASH_SOURCE[0]}")/.." || fail \
  "could not enter this repository's root from ${BASH_SOURCE[0]}" \
  "run this from a checkout of this repository, as .github/workflows/release-plz.yml does" 2

# Quiet on success, and everything the tool said when it fails: release-plz narrates every
# crate it considers, which is noise beside the one line that says what went wrong. A
# variable rather than a temporary file, so there is no log to fail to create or clean up.
quietly() {
  local problem="$1" next="$2" output
  shift 2
  if ! output="$("$@" 2>&1)"; then
    printf '%s\n' "$output" >&2
    fail "$problem" "$next"
  fi
}

command -v release-plz >/dev/null 2>&1 || fail \
  "release-plz is not on PATH, so no version can be decided" \
  "install it — the release workflow does, with taiki-e/install-action — then rerun" 2

quietly \
  "release-plz could not decide the next version" \
  "fix what it reports above; a registry it cannot reach and a manifest it cannot parse both land here" \
  scripts/select-release-version.sh

binary_manifest=crates/onetaskgraph/Cargo.toml
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$binary_manifest" | head -n1)
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] || fail \
  "$binary_manifest has no valid semantic version after the update: '$version'" \
  "restore its X.Y.Z version and rerun; that manifest is where the release's version is read from"

quietly \
  "could not bring every manifest to $version" \
  "fix the manifest or lockfile it names above, then rerun" \
  scripts/set-version.sh "$version"

# Refuse to open a pull request this repository's own required check would refuse; without
# this the drift reaches CI, which costs a three-platform run to say the same thing.
quietly \
  "the manifests still disagree after the sync, so the release pull request would fail its own distribution-check" \
  "fix what set-version.sh named above, then rerun" \
  scripts/set-version.sh --check

quietly \
  "release-plz could not open or update the release pull request" \
  "check that GIT_TOKEN is still authorised to open pull requests, then rerun" \
  release-plz release-pr --allow-dirty
