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
pinned="$(sed -n 's/.*release-plz@\([^ ,]*\).*/\1/p' "$ROOT/.github/workflows/release-plz.yml" | head -n1)"
[[ $pinned =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "the release workflow has no exact X.Y.Z release-plz pin ('$pinned')" "restore its exact tool pin and rerun"
[ "$(release-plz --version 2>/dev/null || true)" = "release-plz $pinned" ] || fail \
  "release-plz $pinned is not installed, so the real preparation cannot be exercised" \
  "run 'just bootstrap', which installs the workflow's pinned tool, then rerun"
for tool in git gh perl python3 uv; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH" "run 'just bootstrap', then rerun"
done

scratch="$(mktemp -d)" || fail "could not create a scratch directory" "check temporary-directory permissions"
trap 'rm -rf "$scratch"' EXIT
repo="$scratch/repo"
remote="$scratch/origin.git"
source_branch="$(git -C "$ROOT" branch --show-current)"
[ -n "$source_branch" ] || fail "the checkout is detached, so no branch can be cloned" "check out the branch under review and rerun"
git clone --quiet --branch "$source_branch" "$ROOT" "$repo" || fail \
  "could not clone the finished tree" "check the current branch and rerun"
# Exercise the working tree under review, then give release-plz the tooling-only commit the
# workflow receives after merge.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$repo" || fail \
  "could not copy the tracked working tree into the checkout" "check git, tar and free space, then rerun"
# Semver compatibility is gated independently. Disabling it in this scratch-only config
# keeps this journey focused on selection and proposal rather than compiling every crate
# twice before either decision can be observed.
perl -pi -e 's/^semver_check = true$/semver_check = false/' "$repo/release-plz.toml" || fail \
  "could not disable the unrelated scratch semver pass" "check Perl and rerun"
git -C "$repo" add -A
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet --no-verify \
  -m "fix(release): prepare tooling-only releases" || fail \
  "could not commit the tooling-only fixture" "check the scratch repository and rerun"
git init --quiet --bare "$remote" || fail "could not create the local origin" "check scratch-directory permissions"
git -C "$repo" remote set-url origin "$remote" || fail "could not point the fixture at its local origin" "check git and rerun"
git -C "$repo" push --quiet --set-upstream origin HEAD || fail "could not seed the local origin" "check git and rerun"
fixture_base="$(git -C "$repo" branch --show-current)"

mkdir -p "$scratch/bin" "$scratch/state" || fail "could not create fixture state directories" "check scratch-directory permissions"
cat > "$scratch/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-} ${2:-}" in
  "pr list") [ -s "$GH_FIXTURE_STATE/proposals" ] && echo 41 || true ;;
  "pr create") printf '%s\n' "$*" >> "$GH_FIXTURE_STATE/proposals"; echo "http://example.invalid/pull/41" ;;
  *) echo "gh fixture: unexpected call: $*" >&2; exit 2 ;;
esac
GH
chmod +x "$scratch/bin/gh" || fail "could not make the gh fixture executable" "check scratch-directory permissions"
export GH_FIXTURE_STATE="$scratch/state"
export GIT_TOKEN=fixture-token
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

git -C "$repo" switch --quiet "$fixture_base" || fail "could not restore $fixture_base before the update case" "check the scratch repository and rerun"
case_log="$scratch/update.log"
if ! (cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1; then
  sed 's/^/    /' "$case_log" >&2
  fail "the real preparation failed while updating the existing proposal" "fix the phase named above and rerun"
fi
grep -qF "updated release pull request #41" "$case_log" || fail \
  "the existing proposal was not updated visibly" "repair the existing-branch and existing-PR path and rerun"
[ "$(wc -l < "$scratch/state/proposals")" -eq 1 ] || fail \
  "updating the existing proposal created a duplicate" "reuse the release branch and open pull request"

git -C "$repo" switch --quiet "$fixture_base" || fail "could not restore $fixture_base before the ineligible case" "check the scratch repository and rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$repo" || fail \
  "could not restore the tracked working tree for the ineligible case" "check git, tar and free space, then rerun"
perl -pi -e 's/^semver_check = true$/semver_check = false/' "$repo/release-plz.toml" || fail \
  "could not disable the ineligible fixture's semver pass" "check Perl and rerun"
git -C "$repo" tag --force v0.2.1 >/dev/null || fail "could not set the ineligible release boundary" "check the scratch repository and rerun"
printf '\n' >> "$repo/README.md" || fail "could not modify the ineligible README fixture" "check scratch-directory permissions"
git -C "$repo" add README.md || fail "could not stage the ineligible fixture" "check the scratch repository and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet --no-verify -m "docs: ineligible fixture" || fail \
  "could not commit the ineligible fixture" "check the scratch repository and rerun"
case_log="$scratch/ineligible.log"
if ! (cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1; then
  sed 's/^/    /' "$case_log" >&2
  fail "the real preparation failed for the ineligible head" "fix the phase named above and rerun"
fi
grep -qF "no release pull request proposed: the selector found no eligible change" "$case_log" || fail \
  "the ineligible run did not expose why it proposed nothing" "repair the successful no-release diagnostic"
[ "$(wc -l < "$scratch/state/proposals")" -eq 1 ] || fail \
  "the ineligible run proposed another pull request" "keep fallback eligibility confined to the inventoried release paths"
