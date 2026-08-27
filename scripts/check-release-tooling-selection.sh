#!/usr/bin/env bash
# Drive the production selector against real commits whose only changed file is release
# tooling, and prove both releasable and non-releasable subjects at the public script seam.
# Exit status: 0 when every case passes; 1 for a behavior finding; 2 when the check itself
# cannot construct or drive its scratch fixture.
set -euo pipefail

# The selector's fallback is reconciled against behavior observed from this release-plz.
# The workflow pin check below makes moving the real tool require re-observing the matrix.
readonly RECORDED_RELEASE_PLZ=0.3.160
export RECORDED_RELEASE_PLZ

fatal() {
  echo "check-release-tooling-selection: $1" >&2
  echo "check-release-tooling-selection: next: $2" >&2
  exit 2
}

finding() {
  echo "check-release-tooling-selection: $1" >&2
  echo "check-release-tooling-selection: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve the repository root" "run this from a checkout of this repository"
# Git hooks export GIT_DIR, which overrides every `git -C "$repo"` below. Load the
# repository's shared guard before constructing the scratch repository so this check runs
# against the same fixture both by hand and from the pre-push gate.
# The source path is built from $ROOT at runtime, so ShellCheck cannot resolve it itself.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load the git-environment guard" \
    "restore scripts/scratch-clone.sh and rerun"
fi
scratch_clone_strip_git_env

scratch="$(mktemp -d)" || fatal "could not create a scratch directory" \
  "check temporary-directory permissions and free space"
trap 'rm -rf "$scratch"' EXIT

repo="$scratch/repo"
mkdir -p "$repo" || fatal "could not create $repo" "check free space and rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$repo" || fatal \
  "could not copy the tracked tree" "confirm git ls-files works and check free space"
# Blank and comment entries are supported inventory syntax. Put both in the committed
# baseline so every selector journey below proves they are ignored during real matching.
perl -pi -e 'print "# scratch-only inventory comment\n\n" if $. == 1' "$repo/config/release-tooling-paths.txt" || fatal \
  "could not add ignored inventory entries to the scratch baseline" "check that Perl works and rerun"
git -C "$repo" init --quiet || fatal "could not initialize the scratch repository" "check that git works and rerun"
git -C "$repo" add -A || fatal "could not stage the scratch baseline" "check scratch-directory permissions and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid \
  commit --quiet --no-verify -m baseline || fatal "could not commit the scratch baseline" "check that git works and rerun"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo/crates/onetaskgraph/Cargo.toml" | head -n1)" || fatal \
  "could not read the baseline binary version" "check scratch-directory permissions and rerun"
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fatal "the binary manifest has no plain X.Y.Z version ('$version')" "restore its version and rerun"
git -C "$repo" tag "v$version" || fatal "could not tag the scratch baseline" "check that git works and rerun"

check_release_plz_pin() {
  workflow="$1"
  installed="$(sed -n 's/.*[ ,]release-plz@\([^ ,]*\).*/\1/p' "$workflow" | head -n1)" || return 2
  [ "$installed" = "$RECORDED_RELEASE_PLZ" ]
}
check_release_plz_pin "$repo/.github/workflows/release-plz.yml" || finding \
  "the release workflow's release-plz pin differs from the selector fixture recorded at $RECORDED_RELEASE_PLZ" \
  "re-observe the real tool's Conventional Commit version choices, then move RECORDED_RELEASE_PLZ with the workflow pin"
pin_fixture="$scratch/release-plz-pin-drift.yml"
cp "$repo/.github/workflows/release-plz.yml" "$pin_fixture" || fatal "could not copy the release-plz pin fixture" "check scratch-directory permissions and rerun"
perl -pi -e 's/release-plz\@\Q$ENV{RECORDED_RELEASE_PLZ}\E/release-plz\@0.0.0/' "$pin_fixture" || fatal \
  "could not mutate the release-plz pin fixture" "check that Perl works and rerun"
if check_release_plz_pin "$pin_fixture"; then
  finding "the release-plz drift guard accepted a changed tool pin" \
    "keep the recorded version comparison tied to the workflow's installed release-plz"
fi

# The real selector invokes release-plz first. This deterministic stand-in models its
# observed result for a tooling-only commit: success without changing a Cargo manifest.
mkdir -p "$scratch/bin" || fatal "could not create the stand-in directory" "check scratch-directory permissions and rerun"
cat > "$scratch/bin/release-plz" <<'STUB'
#!/usr/bin/env bash
if [ "${RELEASE_PLZ_STUB_FAIL:-}" = yes ]; then
  echo "release-plz stand-in: selection failed as requested" >&2
  exit 1
fi
if [ "${RELEASE_PLZ_STUB_INVALID:-}" = yes ]; then
  perl -pi -e 's/^version = "[^"]+"/version = "invalid"/' crates/onetaskgraph/Cargo.toml || {
    echo "release-plz stand-in: could not write invalid fixture" >&2
    echo "release-plz stand-in: next: check scratch-directory permissions" >&2
    exit 1
  }
fi
if [ -n "${RELEASE_PLZ_STUB_SELECTED:-}" ]; then
  [[ $RELEASE_PLZ_STUB_SELECTED =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
    echo "release-plz stand-in: selected version is not X.Y.Z" >&2
    echo "release-plz stand-in: next: repair the test case" >&2
    exit 2
  }
  perl -pi -e 's/^version = "[^"]+"/version = "$ENV{RELEASE_PLZ_STUB_SELECTED}"/' crates/onetaskgraph/Cargo.toml || {
    echo "release-plz stand-in: could not write selected fixture" >&2
    echo "release-plz stand-in: next: check scratch-directory permissions" >&2
    exit 1
  }
fi
exit 0
STUB
chmod +x "$scratch/bin/release-plz" || fatal "could not make the stand-in executable" "check scratch-directory permissions and rerun"

read_version() {
  value="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo/crates/onetaskgraph/Cargo.toml" | head -n1)" || fatal \
    "could not read the scratch binary version" "check scratch-directory permissions and rerun"
  printf '%s' "$value"
}
major="${version%%.*}"
rest="${version#*.}"
minor="${rest%%.*}"
patch="${version##*.}"

case_number=0
run_case() {
  subject="$1" path="$2" expected="$3" selected="${4:-}"
  case_number=$((case_number + 1))
  git -C "$repo" checkout --quiet "v$version" -- . || fatal "could not restore the scratch fixture" "check that git works and rerun"
  git -C "$repo" switch --quiet --detach "v$version" || fatal "could not detach the scratch fixture" "check that git works and rerun"
  # Whitespace is valid in every selected fixture (YAML, shell and JSON), so the commit is
  # tooling-only without making the production parser reject the fixture for an unrelated
  # reason.
  printf '\n' >> "$repo/$path" || fatal "could not modify fixture $path" "check scratch-directory permissions and rerun"
  git -C "$repo" add "$path" || fatal "could not stage fixture $path" "check that git works and rerun"
  git -C "$repo" -c user.name=check -c user.email=check@example.invalid \
    commit --quiet --no-verify -m "$subject" || fatal "could not commit fixture $path" "check that git works and rerun"
  (cd "$repo" && PATH="$scratch/bin:$PATH" RELEASE_PLZ_STUB_SELECTED="$selected" scripts/select-release-version.sh >/dev/null) || finding \
    "the selector failed for '$subject' changing $path" "run that case directly and fix its diagnostic"
  actual="$(read_version)"
  [ "$actual" = "$expected" ] || finding \
    "'$subject' changing only $path selected $actual, expected $expected" \
    "repair scripts/select-release-version.sh without widening release_commits eligibility"
}

# With no commit after the release boundary, selection is a successful no-op. This is the
# normal rerun shape after a tag has already been cut, and reaches the empty rev-list path
# rather than a non-releasable commit.
git -C "$repo" switch --quiet --detach "v$version" || fatal \
  "could not detach the release-boundary fixture" "check that git works and rerun"
(cd "$repo" && PATH="$scratch/bin:$PATH" scripts/select-release-version.sh >/dev/null) || finding \
  "the selector failed with HEAD exactly at the release tag" \
  "repair the no-post-tag-commit path and rerun"
[ "$(read_version)" = "$version" ] || finding \
  "the selector bumped a repository with no commit after the release tag" \
  "repair the no-post-tag-commit path and rerun"

# One real commit per entry in config/release-tooling-paths.txt makes the inventory's
# complete contents executable rather than a prose list; removing an entry breaks its case.
run_case "fix: repair release workflow" ".github/workflows/release.yml" \
  "$major.$minor.$((patch + 1))"
run_case "fix: adjust release-tooling inventory" "config/release-tooling-paths.txt" \
  "$major.$minor.$((patch + 1))"
run_case "fix(cli): repair scoped release workflow" ".github/workflows/release.yml" \
  "$major.$minor.$((patch + 1))"
run_case "fix!: repair breaking release contract" ".github/workflows/release.yml" \
  "$major.$((minor + 1)).0"
run_case "feat: extend distribution contract" "scripts/check-distribution-contract.sh" \
  "$major.$((minor + 1)).0"
run_case "perf: streamline npm packaging" "npm/cli/package.json" \
  "$major.$minor.$((patch + 1))"
run_case "docs: explain release workflow" ".github/workflows/release.yml" "$version"
run_case "fix: unrelated documentation" "README.md" "$version"
run_case "fix: adjust release policy" "release-plz.toml" "$major.$minor.$((patch + 1))"
run_case "fix: adjust TypeScript package" "sdks/typescript/package.json" "$major.$minor.$((patch + 1))"
run_case "fix: adjust Python root package" "pyproject.toml" "$major.$minor.$((patch + 1))"
run_case "fix: adjust Python SDK package" "sdks/python/pyproject.toml" "$major.$minor.$((patch + 1))"
run_case "fix: adjust release preparation" "scripts/prepare-release-pr.sh" "$major.$minor.$((patch + 1))"
run_case "fix: adjust version sync" "scripts/set-version.sh" "$major.$minor.$((patch + 1))"
run_case $'chore: describe compatibility\n\nBREAKING CHANGE: release pipeline contract' \
  ".github/workflows/release.yml" "$major.$((minor + 1)).0"
run_case "feat: tooling beside crate bump" ".github/workflows/release.yml" \
  "$major.$minor.$((patch + 1))" "$major.$minor.$((patch + 1))"

# Two real commits prove aggregation: a later feature dominates the earlier patch bump.
git -C "$repo" checkout --quiet "v$version" -- . || fatal "could not restore the aggregation fixture" "check that git works and rerun"
git -C "$repo" switch --quiet --detach "v$version" || fatal "could not detach the aggregation fixture" "check that git works and rerun"
for entry in "fix: first tooling change|release-plz.toml" "feat: second tooling change|scripts/prepare-release-pr.sh"; do
  subject="${entry%%|*}" path="${entry#*|}"
  printf '\n' >> "$repo/$path" || fatal "could not modify aggregation fixture $path" "check scratch-directory permissions and rerun"
  git -C "$repo" add "$path" || fatal "could not stage aggregation fixture $path" "check that git works and rerun"
  git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet --no-verify \
    -m "$subject" || fatal "could not commit aggregation fixture $path" "check that git works and rerun"
done
(cd "$repo" && PATH="$scratch/bin:$PATH" scripts/select-release-version.sh >/dev/null) || finding \
  "the selector failed to aggregate real commits" "run the aggregation case directly and fix its diagnostic"
[ "$(read_version)" = "$major.$((minor + 1)).0" ] || finding \
  "the selector did not give a feature precedence over a patch" "repair bump aggregation and rerun"

expect_refusal() {
  phrase="$1" expected_status="$2"
  shift 2
  output="" status=0
  output="$(cd "$repo" && PATH="$scratch/bin:$PATH" "$@" 2>&1)" || status=$?
  [ "$status" -eq "$expected_status" ] || finding "the selector refusal exited $status, expected $expected_status" "repair its exit contract and rerun"
  grep -qF "$phrase" <<<"$output" || finding "the selector refusal did not name '$phrase'" "repair its diagnostic and rerun"
}

expect_refusal "takes no arguments" 2 scripts/select-release-version.sh unexpected
mv "$scratch/bin/release-plz" "$scratch/release-plz-away" || fatal "could not hide the scratch stand-in" "check scratch-directory permissions and rerun"
mkdir -p "$scratch/no-release-plz" || fatal "could not create the missing-tool PATH" "check scratch-directory permissions and rerun"
for tool in bash dirname; do
  ln -s "$(command -v "$tool")" "$scratch/no-release-plz/$tool" || fatal "could not link $tool into the missing-tool PATH" "check scratch-directory permissions and rerun"
done
expect_refusal "release-plz is not on PATH" 2 env PATH="$scratch/no-release-plz" scripts/select-release-version.sh
mv "$scratch/release-plz-away" "$scratch/bin/release-plz" || fatal "could not restore the scratch stand-in" "check scratch-directory permissions and rerun"
expect_refusal "release-plz could not decide the next version" 1 env RELEASE_PLZ_STUB_FAIL=yes scripts/select-release-version.sh
git -C "$repo" checkout --quiet "v$version" -- . || fatal "could not restore the refusal fixture" "check that git works and rerun"
expect_refusal "no valid semantic version after release-plz update" 1 env RELEASE_PLZ_STUB_INVALID=yes scripts/select-release-version.sh
git -C "$repo" checkout --quiet "v$version" -- . || fatal "could not restore the refusal fixture" "check that git works and rerun"
printf '\ninvalid = [\n' >> "$repo/release-plz.toml" || fatal "could not corrupt the scratch-only policy" "check scratch-directory permissions and rerun"
git -C "$repo" add release-plz.toml || fatal "could not stage the malformed scratch policy" "check that git works and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet --no-verify \
  -m "fix: malformed release policy" || fatal "could not commit the malformed scratch policy" "check that git works and rerun"
expect_refusal "could not load release eligibility" 1 scripts/select-release-version.sh
git -C "$repo" checkout --quiet "v$version" -- . || fatal "could not restore the refusal fixture" "check that git works and rerun"
# The selected path is valid JSON whitespace, but another carrier is deliberately invalid;
# this reaches the production set-version boundary after selection succeeds.
printf '\ninvalid\n' >> "$repo/npm/cli/package.json" || fatal "could not corrupt the scratch-only carrier" "check scratch-directory permissions and rerun"
printf '\n' >> "$repo/.github/workflows/release.yml" || fatal "could not modify the scratch workflow" "check scratch-directory permissions and rerun"
git -C "$repo" add npm/cli/package.json .github/workflows/release.yml || fatal "could not stage the set-version refusal" "check that git works and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet --no-verify \
  -m "fix: exercise version sync refusal" || fatal "could not commit the set-version refusal" "check that git works and rerun"
expect_refusal "could not select release version" 1 scripts/select-release-version.sh
git -C "$repo" checkout --quiet "v$version" -- . || fatal "could not restore the refusal fixture" "check that git works and rerun"
perl -pi -e 's/^version = "[^"]+"/version = "invalid"/' "$repo/crates/onetaskgraph/Cargo.toml" || fatal "could not invalidate the scratch baseline version" "check scratch-directory permissions and rerun"
expect_refusal "has no plain X.Y.Z version" 1 scripts/select-release-version.sh
git -C "$repo" checkout --quiet "v$version" -- . || fatal "could not restore the refusal fixture" "check that git works and rerun"
mv "$repo/config/release-tooling-paths.txt" "$scratch/release-tooling-paths-away" || fatal "could not hide the release-tooling inventory" "check scratch-directory permissions and rerun"
expect_refusal "could not read config/release-tooling-paths.txt" 1 scripts/select-release-version.sh
mv "$scratch/release-tooling-paths-away" "$repo/config/release-tooling-paths.txt" || fatal "could not restore the release-tooling inventory" "check scratch-directory permissions and rerun"
printf '/outside-checkout/*\n' > "$repo/config/release-tooling-paths.txt" || fatal "could not write the unsafe inventory fixture" "check scratch-directory permissions and rerun"
expect_refusal "contains unsafe pattern" 1 scripts/select-release-version.sh
git -C "$repo" checkout --quiet "v$version" -- config/release-tooling-paths.txt || fatal "could not restore the release-tooling inventory" "check that git works and rerun"
printf '../outside-checkout/*\n' > "$repo/config/release-tooling-paths.txt" || fatal "could not write the parent-traversal inventory fixture" "check scratch-directory permissions and rerun"
expect_refusal "contains unsafe pattern" 1 scripts/select-release-version.sh
git -C "$repo" checkout --quiet "v$version" -- config/release-tooling-paths.txt || fatal "could not restore the release-tooling inventory" "check that git works and rerun"
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-release-tooling-selection: unreadable-manifest case skipped on Windows (its permission model does not make chmod 000 unreadable); the Linux and macOS lanes gate this refusal" >&2
    ;;
  *)
    chmod 000 "$repo/crates/onetaskgraph/Cargo.toml" || fatal "could not make the scratch manifest unreadable" "check scratch-directory permissions and rerun"
    expect_refusal "could not read" 1 scripts/select-release-version.sh
    chmod 644 "$repo/crates/onetaskgraph/Cargo.toml" || fatal "could not restore scratch manifest permissions" "check scratch-directory permissions and rerun"
    ;;
esac
git -C "$repo" tag -d "v$version" >/dev/null || fatal "could not remove the scratch-only tag" "check that git works and rerun"
expect_refusal "release boundary is unknown" 1 scripts/select-release-version.sh
