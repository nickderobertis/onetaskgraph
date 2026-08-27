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

# Preserve successful decisions as well as failure diagnostics in the workflow log.
run_phase() {
  local problem="$1" next="$2" output status
  shift 2
  status=0
  output="$("$@" 2>&1)" || status=$?
  [ -z "$output" ] || printf '%s\n' "$output" >&2
  if [ "$status" -ne 0 ]; then
    [ "$status" -eq 2 ] && fail "$problem" "$next" 2
    fail "$problem" "$next"
  fi
}

command -v release-plz >/dev/null 2>&1 || fail \
  "release-plz is not on PATH, so no version can be decided" \
  "install it — the release workflow does, with taiki-e/install-action — then rerun" 2

selection_output="$(scripts/select-release-version.sh 2>&1)" || {
  status=$?
  printf '%s\n' "$selection_output" >&2
  [ "$status" -eq 2 ] && fail "release version selection failed" "fix what the selector reports above, then rerun" 2
  fail "release version selection failed" "fix what the selector reports above, then rerun"
}
printf '%s\n' "$selection_output" >&2
case "$selection_output" in
  *"release-tooling fallback selected "*) tooling_fallback=yes ;;
  *"release-plz selected "*) tooling_fallback=no ;;
  *"no eligible package or release-tooling commit "*)
    echo "prepare-release-pr: no release pull request proposed: the selector found no eligible change" >&2
    exit 0 ;;
  *) fail "the selector completed without reporting its decision" \
    "repair scripts/select-release-version.sh so success names the selected version or why it selected none" ;;
esac

binary_manifest=crates/onetaskgraph/Cargo.toml
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$binary_manifest" | head -n1)
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] || fail \
  "$binary_manifest has no valid semantic version after the update: '$version'" \
  "restore its X.Y.Z version and rerun; that manifest is where the release's version is read from"

run_phase \
  "could not bring every manifest to $version" \
  "fix the manifest or lockfile it names above, then rerun" \
  scripts/set-version.sh "$version"

# Refuse to open a pull request this repository's own required check would refuse; without
# this the drift reaches CI, which costs a three-platform run to say the same thing.
run_phase \
  "the manifests still disagree after the sync, so the release pull request would fail its own distribution-check" \
  "fix what set-version.sh named above, then rerun" \
  scripts/set-version.sh --check

if [ "$tooling_fallback" = no ]; then
  run_phase "release-plz could not open or update the release pull request" \
    "check that GIT_TOKEN is still authorised to open pull requests, then rerun" \
    release-plz release-pr --allow-dirty --output json
  echo "prepare-release-pr: release-plz proposed the package release pull request for $version" >&2
  exit 0
fi

command -v gh >/dev/null 2>&1 || fail "gh is not on PATH, so the release-tooling pull request cannot be proposed" \
  "install GitHub CLI — GitHub-hosted runners include it — then rerun" 2
base_branch="$(git branch --show-current)"
[ -n "$base_branch" ] || fail "the checkout is detached, so no pull-request base can be named" \
  "check out the repository's default branch and rerun" 2
branch="release-plz-$version"
if git ls-remote --exit-code --heads origin "refs/heads/$branch" >/dev/null 2>&1; then
  run_phase "could not fetch existing release branch $branch" "check the checkout token's contents permission and rerun" \
    git fetch origin "refs/heads/$branch:refs/remotes/origin/$branch"
fi
run_phase "could not create release branch $branch" "restore the checkout and rerun" git switch --force-create "$branch"
run_phase "could not stage the prepared release" "inspect the working tree and rerun" git add -A
run_phase "could not commit the prepared release" "inspect git's diagnostic and rerun" \
  git -c user.name=release-plz -c user.email=release-plz@users.noreply.github.com commit -m "chore: release v$version"
run_phase "could not publish release branch $branch" "check the checkout token's contents permission and rerun" \
  git push --force-with-lease --set-upstream origin "$branch"
pr_number="$(GH_TOKEN="$GIT_TOKEN" gh pr list --head "$branch" --state open --json number --jq '.[0].number // empty')" || fail \
  "could not check for an existing pull request from $branch" "check the token's pull-request permission and rerun"
if [ -z "$pr_number" ]; then
  run_phase "could not open the release pull request" "check the token's pull-request permission and rerun" \
    env GH_TOKEN="$GIT_TOKEN" gh pr create --base "$base_branch" --head "$branch" \
      --title "chore: release v$version" --body "Release v$version selected from an eligible change confined to release tooling."
  echo "prepare-release-pr: proposed release pull request from $branch to $base_branch at v$version" >&2
else
  echo "prepare-release-pr: updated release pull request #$pr_number from $branch to $base_branch at v$version" >&2
fi
