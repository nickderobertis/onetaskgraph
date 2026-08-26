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
set -euo pipefail

fail() {
  echo "prepare-release-pr: $1" >&2
  echo "prepare-release-pr: next: $2" >&2
  exit 1
}

# The git token is read from the environment rather than an argument: an argument list is
# readable by every process on the runner. Nothing here ever prints the value.
[ $# -eq 0 ] || fail \
  "this script takes no arguments and received $#" \
  "pass the token as GIT_TOKEN in the environment, which is where release-plz reads it from"
[ -n "${GIT_TOKEN:-}" ] || fail \
  "GIT_TOKEN is empty, so release-plz could not open a pull request" \
  "set it from this repository's RELEASE_PLZ_TOKEN secret, as .github/workflows/release-plz.yml does"

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd) || fail \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run this from a checkout of this repository, as .github/workflows/release-plz.yml does"
cd "$root" || fail "could not enter $root" "check that the checkout is readable, then rerun"

phase_log="$(mktemp)" || fail \
  "could not create the log each phase below writes to" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -f "$phase_log"' EXIT

# Quiet on success, and everything the tool said when it fails: release-plz narrates every
# crate it considers, which is noise beside the one line that says what went wrong.
quietly() {
  local problem="$1" next="$2"
  shift 2
  if ! "$@" > "$phase_log" 2>&1; then
    cat "$phase_log" >&2
    fail "$problem" "$next"
  fi
}

command -v release-plz >/dev/null 2>&1 || fail \
  "release-plz is not on PATH, so no version can be decided" \
  "install it — the release workflow does, with taiki-e/install-action — then rerun"

quietly \
  "release-plz could not decide the next version" \
  "fix what it reports above; a registry it cannot reach and a manifest it cannot parse both land here" \
  release-plz update

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
