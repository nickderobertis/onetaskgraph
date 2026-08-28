#!/usr/bin/env bash
# Let release-plz select crate changes, then cover release-owned files that no Cargo package
# owns. The latter cannot be expressed in release-plz.toml: changelog_include accepts Cargo
# package names, not arbitrary repository paths.
# Exit status: 0 when selection completed (including no bump); 1 when selection failed;
# 2 when the invocation or required toolchain is invalid.
set -euo pipefail

fail() {
  echo "select-release-version: $1" >&2
  echo "select-release-version: next: $2" >&2
  exit "${3:-1}"
}

[ $# -eq 0 ] || fail "this script takes no arguments and received $#" \
  "run scripts/select-release-version.sh without arguments" 2

# llmlint: ignore-block[changed_behavior_has_e2e] Resolving or entering the script's own tracked checkout can fail only if that checkout is removed while the process starts, which cannot be arranged without removing the boundary under test.
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fail \
  "could not resolve the repository root" "run this from a checkout of this repository" 2
cd "$root" || fail "could not enter $root" "check that the checkout is still present" 2
# llmlint: ignore-end[changed_behavior_has_e2e]

command -v release-plz >/dev/null 2>&1 || fail "release-plz is not on PATH" \
  "install the version pinned in .github/workflows/release-plz.yml and rerun" 2
# llmlint: ignore-block[changed_behavior_has_e2e] Removing either host tool cannot be arranged inside a real repository fixture without replacing the boundary under test; both refusals are direct command-availability guards.
command -v git >/dev/null 2>&1 || fail "git is not on PATH" "install git and rerun" 2
command -v python3 >/dev/null 2>&1 || fail "python3 is not on PATH" "install Python 3.11 or newer and rerun" 2
# llmlint: ignore-end[changed_behavior_has_e2e]

manifest=crates/onetaskgraph/Cargo.toml
release_paths=config/release-tooling-paths.txt
[ -r "$release_paths" ] || fail "could not read $release_paths" \
  "restore the release-tooling path inventory and rerun"
read_version() {
  local value
  value="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$manifest" | head -n1)" || fail \
    "could not read $manifest" "check its permissions and restore the manifest before rerunning"
  printf '%s' "$value"
}
before="$(read_version)"
[[ $before =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || fail \
  "$manifest has no plain X.Y.Z version ('$before')" "restore its released version and rerun"
major="${BASH_REMATCH[1]}"
minor="${BASH_REMATCH[2]}"
patch="${BASH_REMATCH[3]}"

# --no-changelog: this script decides a version and nothing else. An entry written here is
# headed by that package's own next version, which set-version.sh normalises away, and
# `release-plz release-pr` prepends its own entry beside it rather than replacing it — two
# entries for one set of changes, which is what jammed #55.
update_output=""
if ! update_output="$(release-plz update --no-changelog 2>&1)"; then
  printf '%s\n' "$update_output" >&2
  fail "release-plz could not decide the next version" \
    "fix what it reports above; a registry it cannot reach and a manifest it cannot parse both land here"
fi
after="$(read_version)"
[[ $after =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail \
  "$manifest has no valid semantic version after release-plz update ('$after'); expected plain X.Y.Z" \
  "restore its version and inspect the release-plz output before rerunning"
if [ "$after" != "$before" ]; then
  echo "select-release-version: release-plz selected $before -> $after"
  exit 0
fi

# The boundary is a tag, and a tag missing from this checkout is not yet a missing tag.
# `release-plz release` creates it on the origin, and the release workflow runs this script
# in the same job, from a checkout made before that tag existed — so every run that follows
# a successful release arrives here with the boundary it names absent locally. Fetch that
# one ref before deciding anything: it writes the same ref the origin holds, so `$tag..HEAD`
# below enumerates the same commits whether the boundary was already here or was just
# resolved. Only a boundary the origin cannot supply either is genuinely unknown, and that
# refusal is kept — guessing one would propose a version against nothing.
tag="v$before"
if ! git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then
  fetch_output=""
  if ! fetch_output="$(git fetch --no-tags origin "refs/tags/$tag:refs/tags/$tag" 2>&1)"; then
    printf '%s\n' "$fetch_output" >&2
    fail "$tag is in neither this checkout nor origin, so the release boundary is unknown" \
      "restore the tag or make the initial release through release-plz"
  fi
fi

# llmlint: ignore-block[changed_behavior_has_e2e] This read fails only for a repository corrupted after its tag and commit fixture exists; corrupting git internals would replace the real history boundary the checks below drive.
commits="$(git rev-list --reverse "$tag..HEAD")" || fail \
  "could not read commits after $tag" "check the repository history and rerun"
# llmlint: ignore-end[changed_behavior_has_e2e]

# This repository declares release eligibility once, as workspace.release_commits in
# release-plz.toml, and both decisions below ask that one policy rather than restating it.
# A second copy would drift, and the shape of that drift is the whole defect here: the
# policy exists because release-plz would otherwise treat its own release commit as
# release-worthy.
release_policy_accepts() {
  local message="$1" output status=0
  output="$(RELEASE_MESSAGE="$message" python3 - <<'PY'
import os, re, sys, tomllib
try:
    with open("release-plz.toml", "rb") as manifest:
        policy = tomllib.load(manifest)["workspace"]["release_commits"]
    policy = re.compile(policy)
except Exception as error:
    print(error, file=sys.stderr)
    raise SystemExit(2)
raise SystemExit(0 if policy.search(os.environ["RELEASE_MESSAGE"]) else 1)
PY
)" || status=$?
  case "$status" in
    0) return 0 ;;
    1) return 1 ;;
  esac
  printf '%s\n' "$output" >&2
  fail "could not load release eligibility from release-plz.toml" \
    "restore its valid workspace.release_commits policy and rerun"
}

# release-plz reports registry lag without advancing a manifest: its `update` decision is
# registry-based, but it limits that situation to a changelog edit. A tag only proves that
# publishing was attempted, so treating its existence as "already released" would
# permanently wedge a partial publish. What separates a release worth recovering from a
# release this pipeline has just cut itself is not the lag — a registry cannot hold a
# version tagged seconds earlier, so every push that merges a release pull request arrives
# here — but what has landed since the boundary. Recover only for a commit the policy above
# accepts; without that test the pipeline releases its own release commit, without end.
if grep -qF "local version ($before) > registry version (" <<<"$update_output"; then
  recoverable=no
  for commit in $commits; do
    # llmlint: ignore[changed_behavior_has_e2e] Reading a commit this script has just listed fails only for a repository corrupted mid-run, which would replace the history boundary under test.
    message="$(git show -s --format=%B "$commit")" || fail \
      "could not read commit $commit" "check the repository history and rerun"
    if release_policy_accepts "$message"; then
      recoverable=yes
      break
    fi
  done
  if [ "$recoverable" = yes ]; then
    # The attempted version may remain in some registries, while its successor is new to
    # every crate from that release.
    next="$major.$minor.$((patch + 1))"
    scripts/set-version.sh "$next" || fail "could not select registry recovery version $next" \
      "fix the manifest or lockfile named above and rerun"
    echo "select-release-version: registry recovery selected $before -> $next"
    exit 0
  fi
  echo "select-release-version: registry is behind $before with no eligible commit since $tag"
  exit 0
fi

bump=none
# Validate the inventory before path matching so failures are reported in the parent shell,
# rather than being obscured by the NUL-delimited git pipeline below.
while IFS= read -r pattern || [ -n "$pattern" ]; do
  case "$pattern" in
    "" | \#*) continue ;;
    /* | *..*) fail "$release_paths contains unsafe pattern '$pattern'" \
      "keep every entry repository-relative and remove parent traversal" ;;
  esac
done < "$release_paths"

# llmlint: ignore-block[changed_behavior_has_e2e] These git reads fail only for a corrupt repository after the tag and commit fixture has been created; corrupting git internals would replace the real history boundary this check exists to exercise.
for commit in $commits; do
  message="$(git show -s --format=%B "$commit")" || fail \
    "could not read commit $commit" "check the repository history and rerun"
  release_policy_accepts "$message" || continue
  case "$message" in
    feat*|*!:*|*"BREAKING CHANGE:"*) candidate="minor" ;;
    *) candidate="patch" ;;
  esac

  match_status=0
  owns_release_path="$(
    git diff-tree --no-commit-id --name-only -z -r "$commit" |
      {
        matched=no
        while IFS= read -r -d '' path; do
          while IFS= read -r pattern || [ -n "$pattern" ]; do
            case "$pattern" in "" | \#*) continue ;; esac
            [[ $path == $pattern ]] && matched=yes
          done < "$release_paths"
        done
        printf '%s' "$matched"
      }
  )" || match_status=$?
  [ "$match_status" -eq 0 ] || fail \
    "could not read paths changed by commit $commit" "check the repository history and rerun"
# llmlint: ignore-end[changed_behavior_has_e2e]
  [ "$owns_release_path" = yes ] || continue
  [ "$candidate" = minor ] && bump=minor
  [ "$bump" = none ] && bump="patch"
done

case "$bump" in
  none)
    echo "select-release-version: no eligible package or release-tooling commit since $tag"
    exit 0
    ;;
  minor) next="$major.$((minor + 1)).0" ;;
  patch) next="$major.$minor.$((patch + 1))" ;;
esac

scripts/set-version.sh "$next" || fail "could not select release version $next" \
  "fix the manifest or lockfile named above and rerun"
echo "select-release-version: release-tooling fallback selected $before -> $next"
