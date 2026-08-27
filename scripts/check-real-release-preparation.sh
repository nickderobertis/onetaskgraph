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
[ "$(release-plz --version 2>/dev/null || true)" = "release-plz 0.3.160" ] || fail \
  "release-plz 0.3.160 is not installed, so the real preparation cannot be exercised" \
  "run 'just bootstrap', which installs the workflow's pinned tool, then rerun"
for tool in git gh perl python3 uv; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH" "run 'just bootstrap', then rerun"
done

scratch="$(mktemp -d)" || fail "could not create a scratch directory" "check temporary-directory permissions"
trap 'rm -rf "$scratch"' EXIT
repo="$scratch/repo"
remote="$scratch/origin.git"
git clone --quiet --branch "$(git -C "$ROOT" branch --show-current)" "$ROOT" "$repo" || fail \
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
git -C "$repo" remote set-url origin "$remote"
git -C "$repo" push --quiet --set-upstream origin HEAD || fail "could not seed the local origin" "check git and rerun"

mkdir -p "$scratch/bin" "$scratch/state"
cat > "$scratch/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-} ${2:-}" in
  "pr list") exit 0 ;;
  "pr create") printf '%s\n' "$*" >> "$GH_FIXTURE_STATE/proposals"; echo "http://example.invalid/pull/1" ;;
  *) echo "gh fixture: unexpected call: $*" >&2; exit 2 ;;
esac
GH
chmod +x "$scratch/bin/gh"
export GH_FIXTURE_STATE="$scratch/state"
export GIT_TOKEN=fixture-token
export PATH="$scratch/bin:$PATH"

case_log="$scratch/tooling.log"
if ! (cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1; then
  sed 's/^/    /' "$case_log" >&2
  fail "the real preparation failed for the tooling-only head" "fix the phase named above and rerun"
fi
grep -qF "release-tooling fallback selected 0.2.1 -> 0.2.2" "$case_log" || fail \
  "the real tool did not expose the tooling fallback decision" "inspect $case_log and repair the selector diagnostic"
grep -qF "proposed release pull request" "$case_log" || fail \
  "the tooling-only run did not report a proposal" "repair the fallback proposal path and rerun"
[ -s "$scratch/state/proposals" ] || fail "the tooling-only run never proposed a pull request" \
  "repair the fallback proposal path and rerun"
(cd "$repo" && scripts/set-version.sh --check) || fail "the proposed tree has version drift" \
  "run scripts/set-version.sh with the selected version and carry every changed manifest"

git -C "$repo" switch --quiet main
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$repo" || fail \
  "could not restore the tracked working tree for the ineligible case" "check git, tar and free space, then rerun"
perl -pi -e 's/^semver_check = true$/semver_check = false/' "$repo/release-plz.toml"
git -C "$repo" add -A
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet --no-verify \
  -m "test: disable semver in fixture"
git -C "$repo" tag --force v0.2.1 >/dev/null
printf '\n' >> "$repo/README.md"
git -C "$repo" add README.md
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet --no-verify -m "docs: ineligible fixture"
case_log="$scratch/ineligible.log"
if ! (cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1; then
  sed 's/^/    /' "$case_log" >&2
  fail "the real preparation failed for the ineligible head" "fix the phase named above and rerun"
fi
grep -qF "no release pull request proposed: the selector found no eligible change" "$case_log" || fail \
  "the ineligible run did not expose why it proposed nothing" "repair the successful no-release diagnostic"
[ "$(wc -l < "$scratch/state/proposals")" -eq 1 ] || fail \
  "the ineligible run proposed another pull request" "keep fallback eligibility confined to the inventoried release paths"
