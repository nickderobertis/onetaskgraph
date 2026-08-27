#!/usr/bin/env bash
# Drive the real pinned release-plz through both decisions without contacting GitHub.
set -euo pipefail

fail() {
  echo "check-real-release-preparation: $1" >&2
  echo "check-real-release-preparation: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fail \
  "could not resolve the repository root" "run this from a checkout of the repository"

# Both production jobs that run release preparation use ubuntu-latest. On Windows, Git
# applies this hermetic fixture's insteadOf transport rewrite before release-plz chooses a
# forge, so the pinned tool sees the local bare repository rather than the deliberately
# GitHub-shaped public origin and refuses it. Linux and macOS retain the real-checkout,
# real-release-plz journey below; Windows has no production release path for it to model.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-real-release-preparation: skipped on Windows (release preparation runs only on ubuntu-latest, and Git rewrites this hermetic fixture's GitHub-shaped origin before release-plz detects its forge there); the Linux and macOS lanes gate both real release decisions" >&2
    exit 0
    ;;
esac

[ -f "$ROOT/scripts/scratch-clone.sh" ] || fail \
  "scripts/scratch-clone.sh is missing, so hook-exported git state cannot be cleared" \
  "restore scripts/scratch-clone.sh and rerun"
# shellcheck source=scripts/scratch-clone.sh
source "$ROOT/scripts/scratch-clone.sh"
# Git exports repository-routing variables to hooks, and they override every later `git
# -C`. Clear them before the first git command so this check addresses its scratch clone
# even when distribution-check is running inside pre-push.
scratch_clone_strip_git_env
pinned="$(sed -n 's/.*release-plz@\([^ ,]*\).*/\1/p' "$ROOT/.github/workflows/release-plz.yml" | head -n1)" || fail \
  "could not read the workflow's release-plz pin" "restore the readable workflow and rerun"
[[ $pinned =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "the release workflow has no exact X.Y.Z release-plz pin ('$pinned')" "restore its exact tool pin and rerun"
[ "$(release-plz --version 2>/dev/null || true)" = "release-plz $pinned" ] || fail \
  "release-plz $pinned is not installed, so the real preparation cannot be exercised" \
  "run 'just bootstrap', which installs the workflow's pinned tool, then rerun"
release_plz_bin="$(command -v release-plz)" || fail \
  "could not resolve the installed release-plz" "run 'just bootstrap', then rerun"
for tool in git gh perl python3 uv; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH" "run 'just bootstrap', then rerun"
done

scratch="$(mktemp -d)" || fail "could not create a scratch directory" "check temporary-directory permissions"
trap 'rm -rf "$scratch"' EXIT
repo="$scratch/repo"
remote="$scratch/origin.git"
hooks="$scratch/hooks"
fixture_base=fixture-main
scratch_clone "$ROOT" "$repo" || fail \
  "could not clone the finished tree" "fix what scratch-clone reported above and rerun"
git -C "$repo" switch --quiet --create "$fixture_base" || fail \
  "could not create the fixture branch $fixture_base" "check the cloned commit and rerun"
# The user's hooks belong to the checkout under review, not to fixture setup. Point this
# scratch repository at an empty hook directory so its synthetic commits and local pushes
# cannot recursively launch the complete gate when the check itself runs from a hook.
mkdir -p "$hooks" || fail "could not create the empty fixture hook directory" "check scratch-directory permissions"
git -C "$repo" config core.hooksPath "$hooks" || fail \
  "could not isolate the fixture from repository hooks" "check the scratch repository and rerun"
# Exercise the working tree under review, then give release-plz the tooling-only commit the
# workflow receives after merge.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$repo" || fail \
  "could not copy the tracked working tree into the checkout" "check git, tar and free space, then rerun"
# Semver compatibility is gated independently. Disabling it in this scratch-only config
# keeps this journey focused on selection and proposal rather than compiling every crate
# twice before either decision can be observed.
perl -pi -e 's/^semver_check = true$/semver_check = false/' "$repo/release-plz.toml" || fail \
  "could not disable the unrelated scratch semver pass" "check Perl and rerun"
git -C "$repo" add -A || fail "could not stage the tooling-only fixture" "check the scratch repository and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet \
  -m "fix(release): prepare tooling-only releases" || fail \
  "could not commit the tooling-only fixture" "check the scratch repository and rerun"
git init --quiet --bare "$remote" || fail "could not create the local origin" "check scratch-directory permissions"
# release-plz chooses its forge from the configured remote URL. Keep that public seam
# GitHub-shaped, while Git's URL rewrite routes every transport operation to the local
# bare repository so this journey remains hermetic.
fixture_origin=https://github.com/check/onetaskgraph-release-fixture.git
git -C "$repo" config "url.$remote.insteadOf" "$fixture_origin" || fail \
  "could not route the fixture origin to its local repository" "check git and rerun"
git -C "$repo" remote set-url origin "$fixture_origin" || fail \
  "could not give the fixture a GitHub-shaped origin" "check git and rerun"
git -C "$repo" push --quiet --set-upstream origin HEAD || fail "could not seed the local origin" "check git and rerun"
released_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo/crates/onetaskgraph/Cargo.toml" | head -n1)" || fail \
  "could not read the fixture's released version" "restore the binary manifest and rerun"
[[ $released_version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail \
  "the fixture has no plain X.Y.Z released version ('$released_version')" "restore the binary manifest and rerun"
# The checkout under test can be either an ordinary main commit, whose manifest version
# has already been tagged, or a release pull request, whose version has not. Neither shape
# should determine this fixture's history: discard every copied tag and establish the
# tooling-only scenario's release boundary on the commit immediately before its eligible
# release-tooling change.
git -C "$repo" for-each-ref --format='delete %(refname)' refs/tags | \
  git -C "$repo" update-ref --stdin || fail \
  "could not clear inherited tags from the tooling-only fixture" "check the scratch repository and rerun"
git -C "$repo" tag "v$released_version" HEAD^ || fail \
  "could not set the tooling-only release boundary" "check the scratch repository and rerun"
git --git-dir="$remote" symbolic-ref HEAD "refs/heads/$fixture_base" || fail \
  "could not set the local origin's default branch" "check the scratch repository and rerun"
git -C "$repo" remote set-head origin "$fixture_base" || fail \
  "could not align the clone's default remote branch with $fixture_base" "check the scratch repository and rerun"
git -C "$repo" switch --quiet --detach "$fixture_base" || fail \
  "could not detach the tooling-only checkout" "check the scratch repository and rerun"

mkdir -p "$scratch/bin" "$scratch/state" || fail "could not create fixture state directories" "check scratch-directory permissions"
if ! cat > "$scratch/bin/release-plz" <<'RELEASE_PLZ'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = update ]; then
  exec "$REAL_RELEASE_PLZ" "$@" --forge github
fi
exec "$REAL_RELEASE_PLZ" "$@"
RELEASE_PLZ
then
  fail "could not create the release-plz fixture launcher" "check scratch-directory permissions and free space"
fi
if ! cat > "$scratch/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-} ${2:-}" in
  "pr list") [ -s "$GH_FIXTURE_STATE/proposals" ] && echo 41 || true ;;
  "pr create") printf '%s\n' "$*" >> "$GH_FIXTURE_STATE/proposals"; echo "http://example.invalid/pull/41" ;;
  *) echo "gh fixture: unexpected call: $*" >&2; exit 2 ;;
esac
GH
then
  fail "could not create the gh fixture" "check scratch-directory permissions and free space"
fi
chmod +x "$scratch/bin/release-plz" "$scratch/bin/gh" || fail \
  "could not make the command fixtures executable" "check scratch-directory permissions"
export GH_FIXTURE_STATE="$scratch/state"
export GIT_TOKEN=fixture-token
export GITHUB_REF_NAME="$fixture_base"
# Git's insteadOf routing can be applied before release-plz detects the forge on Windows.
# State the forge at this hermetic fixture boundary while still delegating the update to
# the exact pinned binary verified above.
export REAL_RELEASE_PLZ="$release_plz_bin"
export PATH="$scratch/bin:$PATH"

case_log="$scratch/tooling.log"
if ! (cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1; then
  sed 's/^/    /' "$case_log" >&2
  fail "the real preparation failed for the tooling-only head" "fix the phase named above and rerun"
fi
grep -qF "proposed release pull request" "$case_log" || fail \
  "the tooling-only run did not report a proposal" "repair the fallback proposal path and rerun"
[ -s "$scratch/state/proposals" ] || fail "the tooling-only run never proposed a pull request" \
  "repair the fallback proposal path and rerun"
(cd "$repo" && scripts/set-version.sh --check) || fail "the proposed tree has version drift" \
  "run scripts/set-version.sh with the selected version and carry every changed manifest"

git -C "$repo" switch --quiet --detach "$fixture_base" || fail "could not restore a detached $fixture_base before the update case" "check the scratch repository and rerun"
unset GITHUB_REF_NAME
case_log="$scratch/update.log"
if ! (cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1; then
  sed 's/^/    /' "$case_log" >&2
  fail "the real preparation failed while updating the existing proposal" "fix the phase named above and rerun"
fi
grep -qF "updated release pull request #41" "$case_log" || fail \
  "the existing proposal was not updated visibly" "repair the existing-branch and existing-PR path and rerun"
[ "$(wc -l < "$scratch/state/proposals")" -eq 1 ] || fail \
  "updating the existing proposal created a duplicate" "reuse the release branch and open pull request"

git -C "$repo" switch --quiet "$fixture_base" || fail "could not restore $fixture_base before the partial-publish case" "check the scratch repository and rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$repo" || fail \
  "could not restore the tracked working tree for the partial-publish case" "check git, tar and free space, then rerun"
perl -pi -e 's/^semver_check = true$/semver_check = false/' "$repo/release-plz.toml" || fail \
  "could not disable the partial-publish fixture's semver pass" "check Perl and rerun"
git -C "$repo" tag --force "v$released_version" >/dev/null || fail "could not set the partial-publish release boundary" "check the scratch repository and rerun"
package_names="$(cd "$repo" && cargo metadata --no-deps --format-version 1 | python3 -c \
  'import json, sys; print(" ".join(package["name"] for package in json.load(sys.stdin)["packages"]))')" || fail \
  "could not read the partial-publish package inventory" "repair Cargo metadata and rerun"
for crate in $package_names; do
  [ "$crate" = onetaskgraph ] && continue
  git -C "$repo" tag --force "$crate-v$released_version" >/dev/null || fail \
    "could not set the partial-publish tag for $crate" "check the scratch repository and rerun"
done
printf '\n' >> "$repo/crates/onetaskgraph-core/src/lib.rs" || fail "could not modify the partial-publish fixture" "check scratch-directory permissions"
git -C "$repo" add crates/onetaskgraph-core/src/lib.rs || fail "could not stage the partial-publish fixture" "check the scratch repository and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet -m "fix(core): partial-publish recovery fixture" || fail \
  "could not commit the partial-publish fixture" "check the scratch repository and rerun"
# The earlier cases leave an open fixture PR. Remove only that scratch state so registry
# recovery proves its fresh-proposal branch as well as the update branch exercised above.
: > "$scratch/state/proposals" || fail "could not clear the scratch proposal state" "check scratch-directory permissions and rerun"
case_log="$scratch/partial-publish.log"
if ! (cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1; then
  sed 's/^/    /' "$case_log" >&2
  fail "the real preparation failed for the partly published head" "fix the phase named above and rerun"
fi
grep -qF "proposed release pull request" "$case_log" || fail \
  "the partly published run did not create a registry-recovery proposal" "advance beyond the tagged version and open its release pull request"
recovery_version="${released_version%.*}.$((${released_version##*.} + 1))"
[ "$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo/crates/onetaskgraph/Cargo.toml" | head -n1)" = "$recovery_version" ] || fail \
  "the partly published run did not select $recovery_version" "advance every crate to the patch after the attempted release"
[ "$(wc -l < "$scratch/state/proposals")" -eq 1 ] || fail \
  "the partly published run created a duplicate pull request" "reuse the existing release pull request during recovery"
