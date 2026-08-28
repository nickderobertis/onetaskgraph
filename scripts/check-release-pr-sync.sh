#!/usr/bin/env bash
# Drive scripts/prepare-release-pr.sh over a real version bump and prove the release pull
# request it prepares carries version manifests that agree.
#
# The bump is the only moment they can part, and v0.1.0 needed none, so nothing before #26
# had ever run this path. release-plz itself is stood in for below rather than installed:
# putting it in the gate would put crates.io on the critical path of a required check. Every
# behaviour the stand-in reproduces was observed from the pinned release-plz, driven against
# this repository; the pin and this file are reconciled in case 12. What is under test is
# this repository's own scripts, which are real here.
#
# Exit status: 0, the preparation carries what it must; 1, a case below failed and the
# release pull request would not pass its own checks; 2, this check could not be run at all
# — a scratch tree it could not build, a toolchain it could not find — which is a different
# thing from a finding and reads differently in a gate log.
set -euo pipefail

# The release-plz the stand-in below was recorded from. The workflow installs exactly this
# version, and case 12 fails when the two part: a stand-in models one version's behaviour.
readonly RECORDED_RELEASE_PLZ=0.3.160

# The journey could not be run. Distinct from a case failing, which is a finding about the
# tree and exits 1 at the end.
fatal() {
  echo "check-release-pr-sync: $1" >&2
  echo "check-release-pr-sync: next: $2" >&2
  exit 2
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just distribution-check' does"
readonly ROOT

for tool in cargo git node perl python3 uv; do
  command -v "$tool" >/dev/null 2>&1 || fatal \
    "$tool is not on PATH, and the journey below cannot bump a version without it" \
    "install $tool — 'just bootstrap' provisions the toolchain this repository needs — then rerun"
done

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this journey bumps" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT
readonly scratch

# Git exports GIT_DIR to every hook and GIT_DIR overrides `git -C`; the gate runs from
# .githooks/pre-push, so every git command below has to be asked in a stripped environment
# or it answers about — and commits into — the repository the hook was invoked for.
#
# The path below is built from $ROOT at run time, so ShellCheck cannot follow it to decide
# whether scratch_clone_strip_git_env is defined; the directive names the file it resolves to.
# shellcheck source=scripts/scratch-clone.sh
# Tested before it is sourced, not merely guarded after: bash 3.2 ends the shell where
# `source` cannot find its file, so the handler a later bash takes never runs there — and
# macos-latest is a 3.2 runner.
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal \
    "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore that file with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi
scratch_clone_strip_git_env

# The WORKING tree's tracked files, not HEAD's: what is under test is the release-PR
# preparation as it is right now, so an author repairing it does not watch this journey keep
# failing against the version they just replaced.
repo="$scratch/repo"
mkdir -p "$repo" || fatal \
  "could not create the scratch tree at $repo" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$repo" || fatal \
  "could not copy $ROOT's tracked files into $repo (see the tar or git output above)" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

# A repository rather than a directory: release-plz decides what a release commit carries
# from what differs from HEAD, and the stand-in answers the same question the same way.
git -C "$repo" init --quiet || fatal \
  "could not initialise the scratch repository at $repo" \
  "check that git works ('git --version') and 'df -h' for free space, then rerun"
git -C "$repo" add -A >/dev/null 2>&1 && \
  git -C "$repo" -c user.email=check@example.invalid -c user.name=check \
    commit --quiet --no-verify -m "baseline" >/dev/null 2>&1 || fatal \
  "could not commit the baseline in $repo, so nothing below could tell a bumped file from an unchanged one" \
  "check that git works ('git --version') and 'df -h' for free space, then rerun"

baseline_commit="$(git -C "$repo" rev-parse HEAD)" || fatal \
  "could not read the scratch repository's baseline commit" \
  "check that git works ('git --version') and rerun"
readonly baseline_commit

restore_scratch() {
  git -C "$repo" checkout --quiet -- . || fatal \
    "could not restore the scratch tree after a case, so every case after it would run against a half-bumped tree" \
    "rerun; if it persists, check 'df -h' for a full disk"
}

read_toml_version() { sed -n 's/^version = "\([^"]*\)"/\1/p' "$1" | head -n1; }
read_json_version() { sed -n 's/.*"version": "\([^"]*\)".*/\1/p' "$1" | head -n1; }
manifest_version() {
  case "$1" in
    *.json) read_json_version "$repo/$1" ;;
    *) read_toml_version "$repo/$1" ;;
  esac
}

old_version="$(read_toml_version "$repo/crates/onetaskgraph/Cargo.toml")"
[[ $old_version =~ ^([0-9]+)\.([0-9]+)\.[0-9]+$ ]] || fatal \
  "crates/onetaskgraph/Cargo.toml has no plain X.Y.Z version ('$old_version'), so this journey cannot bump it" \
  "restore that manifest's version, or teach this journey the version grammar that replaced it"
# A minor bump, which is what this repository's policy makes of a `feat` before 1.0 — the
# same bump release-plz computed for v0.2.0.
new_version="${BASH_REMATCH[1]}.$(( BASH_REMATCH[2] + 1 )).0"
# The version one crate takes on its own, because release-plz decides each package
# separately: #55's run selected the patch for the binary and the minor for
# onetaskgraph-plugin-api, and the sync below has to reconcile exactly that.
divergent_version="${BASH_REMATCH[1]}.$(( BASH_REMATCH[2] + 2 )).0"
divergent_crate=onetaskgraph-plugin-api
readonly old_version new_version divergent_version divergent_crate
[ -f "$repo/crates/$divergent_crate/Cargo.toml" ] || fatal \
  "crates/$divergent_crate/Cargo.toml is missing, so no crate can take a bump of its own here" \
  "point divergent_crate at a crate this workspace still has, then rerun"

# Every version manifest release-plz does NOT write. Each is a way the release pull request
# fails its own gate, and the root Cargo.toml is here because the version under
# [workspace.package] is one release-plz leaves behind too.
carriers="Cargo.toml
pyproject.toml
sdks/python/pyproject.toml
sdks/typescript/package.json
npm/cli/package.json
npm/platforms/darwin-arm64/package.json
npm/platforms/darwin-x64/package.json
npm/platforms/linux-arm64/package.json
npm/platforms/linux-x64/package.json
npm/platforms/win32-x64/package.json"

state="$scratch/state"
mkdir -p "$state/seen" || fatal \
  "could not create the recording directory at $state/seen" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

# The stand-in, and the seven behaviours this repository depends on, every one of them
# observed from the pinned release-plz against this workspace:
#
#   `update` rewrites crates/*/Cargo.toml and the [workspace.dependencies] path pins, and
#   refreshes Cargo.lock. It leaves every manifest in `carriers` above untouched — the real
#   run left all ten at 0.1.0 while every crate went to 0.2.0.
#
#   `update` bumps each package on its own, so one crate's next version is not another's:
#   #55 selected 0.2.9 for the binary and 0.3.0 for onetaskgraph-plugin-api out of one run.
#   Which crate diverges does not matter here; that one can is what the sync has to survive.
#
#   `update` prepends one changelog entry per updated package, headed by that package's own
#   next version and placed above the newest versioned heading.
#
#   `update --no-changelog` bumps the manifests and writes no changelog at all.
#
#   `update` and `release-pr` both refuse a checkout with uncommitted changes unless
#   --allow-dirty, exiting 1 with the diagnostic below (observed from `release-plz update`
#   over a dirty fixture checkout on 2026-08-28).
#
#   `release-pr --allow-dirty` builds the release commit from what differs from HEAD: the
#   real run sent exactly those files, contents and all, in the createCommitOnBranch mutation
#   that makes the commit. So what it records here is what the pull request would carry.
#
#   `release-pr --allow-dirty` copies the project — uncommitted changes included — and runs
#   its own update over that copy before building that commit, so the entry it writes lands
#   beside any entry already in the tree rather than replacing it. That is what put two
#   entries for one set of changes into #55, and it reaches the release commit alone: the
#   checkout it was run from keeps whatever it had.
cat > "$scratch/release-plz" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
version="$RELEASE_PLZ_STUB_VERSION"
divergent_crate="$RELEASE_PLZ_STUB_DIVERGENT_CRATE"
divergent_version="$RELEASE_PLZ_STUB_DIVERGENT_VERSION"
entry_date="$RELEASE_PLZ_STUB_ENTRY_DATE"
state="$RELEASE_PLZ_STUB_STATE"
subcommand="${1:-}"
shift || true
allow_dirty=no
no_changelog=no
for argument in "$@"; do
  [ "$argument" = --allow-dirty ] && allow_dirty=yes
  [ "$argument" = --no-changelog ] && no_changelog=yes
done
refuse_dirty() {
  [ "$allow_dirty" = no ] || return 0
  [ -n "$(git status --porcelain)" ] || return 0
  echo "the working directory of this project has uncommitted changes. If these files are both committed and in .gitignore, either delete them or remove them from .gitignore." >&2
  echo "Otherwise, please commit or stash these changes:" >&2
  git status --porcelain | sed 's/^...//' >&2
  exit 1
}
# The entry goes above the newest versioned heading, which is where the real tool puts it:
# an `## [Unreleased]` heading carries no digit and stays on top.
prepend_entry() {
  local changelog="$1" entry_version="$2"
  [ -f "$changelog" ] || return 0
  CHANGELOG_ENTRY="## [$entry_version] - $entry_date

### Added

- a change this release describes
" perl -0pi -e 's/^(## \[\d)/$ENV{CHANGELOG_ENTRY}\n$1/m' "$changelog"
}
crate_version() { sed -n 's/^version = "\([^"]*\)"/\1/p' "$1" | head -n1; }
# The cases that drive a failing release-plz set this to the phase that should fail.
if [ "${RELEASE_PLZ_STUB_FAIL:-}" = "$subcommand" ]; then
  echo "release-plz stand-in: $subcommand was asked to fail" >&2
  fail_status="${RELEASE_PLZ_STUB_FAIL_STATUS:-1}"
  if ! [[ $fail_status =~ ^[1-9][0-9]*$ ]] || [ "$fail_status" -gt 255 ]; then
    echo "release-plz stand-in: failure status must be an integer from 1 through 255" >&2
    exit 2
  fi
  exit "$fail_status"
fi
case "$subcommand" in
  update)
    refuse_dirty
    perl -pi -e "s/^version = \"[^\"]+\"/version = \"$version\"/" crates/*/Cargo.toml
    perl -pi -e "s|(path = \"crates/[^\" ]+\", version = \")[^\"]+|\${1}$version|g" Cargo.toml
    # The package that takes a bump of its own, manifest and path pin together — cargo
    # refuses a path dependency whose pin cannot match the crate beside it.
    perl -pi -e "s/^version = \"[^\"]+\"/version = \"$divergent_version\"/" "crates/$divergent_crate/Cargo.toml"
    perl -pi -e "s|(path = \"crates/$divergent_crate\", version = \")[^\"]+|\${1}$divergent_version|g" Cargo.toml
    # One case asks for a bump that leaves no version to read: cargo accepts an inherited
    # one, so this is a refactor the workspace could really take, and the reader of the
    # release's version would silently get an empty string.
    if [ "${RELEASE_PLZ_STUB_INHERIT_VERSION:-}" = yes ]; then
      perl -pi -e 's/^version = "[^"]+"/version.workspace = true/' crates/onetaskgraph/Cargo.toml
    fi
    if [ "$no_changelog" = no ]; then
      for manifest in crates/*/Cargo.toml; do
        prepend_entry "${manifest%/Cargo.toml}/CHANGELOG.md" "$(crate_version "$manifest")"
      done
    fi
    # release-plz runs `cargo update --workspace`; this is the same refresh, and the one
    # scripts/set-version.sh uses to keep Cargo.lock in step with a version change.
    cargo metadata --format-version 1 >/dev/null
    echo "update $*" >> "$state/calls"
    ;;
  release-pr)
    echo "release-pr $*" >> "$state/calls"
    echo "release-plz stand-in: proposed package release"
    refuse_dirty
    git status --porcelain | sed 's/^...//' > "$state/carried"
    # The changelogs its own update writes are carried too, whether or not the tree had
    # already changed them.
    for manifest in crates/*/Cargo.toml; do
      changelog="${manifest%/Cargo.toml}/CHANGELOG.md"
      [ -f "$changelog" ] || continue
      grep -qxF "$changelog" "$state/carried" || echo "$changelog" >> "$state/carried"
    done
    while IFS= read -r carried; do
      [ -f "$carried" ] || continue
      cp "$carried" "$state/seen/${carried//\//__}"
    done < "$state/carried"
    # That update runs over the copy, so the entry lands in what the commit carries and the
    # checkout it was run from keeps whatever it had.
    for manifest in crates/*/Cargo.toml; do
      changelog="${manifest%/Cargo.toml}/CHANGELOG.md"
      prepend_entry "$state/seen/${changelog//\//__}" "$(crate_version "$manifest")"
    done
    ;;
  *)
    echo "release-plz stand-in: no behaviour recorded for '$subcommand'" >&2
    echo "next: record what release-plz does for it, from a real run, before depending on it" >&2
    exit 2
    ;;
esac
STUB
chmod +x "$scratch/release-plz" || fatal \
  "could not make the release-plz stand-in executable" \
  "check the permissions of \$TMPDIR, then rerun"

# The PATH the "no release-plz installed" case runs under. The ambient PATH will not do:
# a machine with release-plz installed would run the real tool against the scratch tree and
# the case would prove the opposite of what it says. So every directory carrying one is
# dropped, and the result is put to the question it is about to be trusted for.
path_without_tool() {
  local tool="$1" entry result=""
  local IFS=:
  for entry in $PATH; do
    [ -n "$entry" ] || continue
    ( PATH="$entry"; hash -r 2>/dev/null; command -v "$tool" >/dev/null 2>&1 ) && continue
    result="${result:+$result:}$entry"
  done
  printf '%s' "$result"
}
PATH_WITHOUT_RELEASE_PLZ="$(path_without_tool release-plz)"
readonly PATH_WITHOUT_RELEASE_PLZ
if ( PATH="$PATH_WITHOUT_RELEASE_PLZ"; hash -r 2>/dev/null; command -v release-plz >/dev/null 2>&1 ); then
  fatal \
    "release-plz is still reachable after dropping every directory that carries one, so the case for a machine without it would drive the real tool against the scratch tree" \
    "run 'command -v release-plz' and take it off PATH — a shell function or an alias reaches past the directory scan above"
fi

# Subtract only the entries that expose python3. Keeping every other PATH entry is
# load-bearing on Windows, where Git Bash needs DLLs beside its own installation and a
# whitelist of executable symlinks leaves bash itself unable to start. The links retain
# commands that share Python's directory on Unix; the remaining PATH retains their runtime
# libraries on Windows.
PATH_WITHOUT_PYTHON="$(path_without_tool python3)"
readonly PATH_WITHOUT_PYTHON
if ( PATH="$PATH_WITHOUT_PYTHON"; hash -r 2>/dev/null; command -v python3 >/dev/null 2>&1 ); then
  fatal \
    "python3 is still reachable after dropping every directory that carries it, so the missing-tool case would not reach the selector's refusal" \
    "run 'command -v python3' and remove any shell function or alias that reaches past the PATH scan"
fi
path_without_python="$scratch/path-without-python"
mkdir -p "$path_without_python" || fatal \
  "could not create the missing-python fixture" "check scratch-directory permissions and rerun"
for tool in bash dirname git head sed; do
  ln -s "$(command -v "$tool")" "$path_without_python/$tool" || fatal \
    "could not link $tool into the missing-python fixture" "check scratch-directory permissions and rerun"
done
readonly path_without_python
export PATH="$scratch:$PATH"
export RELEASE_PLZ_STUB_VERSION="$new_version"
export RELEASE_PLZ_STUB_DIVERGENT_CRATE="$divergent_crate"
export RELEASE_PLZ_STUB_DIVERGENT_VERSION="$divergent_version"
# Fixed rather than today's date: two preparations of the same base have to be comparable
# byte for byte, and the real tool's own entry date is the one thing about them that moves.
export RELEASE_PLZ_STUB_ENTRY_DATE=2026-08-28
export RELEASE_PLZ_STUB_STATE="$state"
# Not a credential: the stand-in never sends it anywhere. It is here because the script
# refuses to run without one, which is case 3.
export GIT_TOKEN="unused-by-the-release-plz-stand-in"

failures=0
report() {
  failures=$((failures + 1))
  echo "check-release-pr-sync: $1" >&2
}

case_log="$scratch/case.log"
quote_case_log() { sed 's/^/    /' "$case_log" >&2; }

# Run the preparation script and require it to refuse with the status its own contract gives
# that class of failure, saying the named thing. The command is given in full so a case can
# vary the environment it runs under.
expect_refusal() {
  local case_name="$1" phrase="$2" expected_status="$3" status=0
  shift 3
  (cd "$repo" && "$@") > "$case_log" 2>&1 || status=$?
  if [ "$status" -eq 0 ]; then
    report "$case_name — the preparation ACCEPTED it, so a release pull request would be opened over a tree nothing has checked. It said:"
    quote_case_log
  elif [ "$status" -ne "$expected_status" ]; then
    report "$case_name — it refused with status $status, but its own contract gives that failure $expected_status, so a workflow reading the status cannot tell a wrong call from a failed phase. It said:"
    quote_case_log
  elif ! grep -qF "$phrase" "$case_log"; then
    report "$case_name — it refused, but its diagnostic never says '$phrase', so it does not say what to go and fix. It said:"
    quote_case_log
  fi
  restore_scratch
}

# 1. The regression itself, watched failing. A release pull request carrying only what
#    release-plz writes is the tree that failed #26, and this repository's own version check
#    has to be the thing that says so — if it ever stops, every case below proves nothing.
if ! (cd "$repo" && release-plz update) > "$case_log" 2>&1; then
  quote_case_log
  fatal \
    "the release-plz stand-in could not bump the scratch tree, so this journey never reached the sync" \
    "read the output above; a cargo that cannot resolve the workspace lands here"
fi
if (cd "$repo" && scripts/set-version.sh --check) > "$case_log" 2>&1; then
  fatal \
    "scripts/set-version.sh --check ACCEPTED a tree bumped the way release-plz bumps it, so it would not have caught #26 either" \
    "repair that check first — until it refuses this tree, nothing below can prove the sync fixes anything"
fi
if ! grep -qF "npm/cli/package.json has $old_version; expected $new_version" "$case_log"; then
  report "the version check refused the release-plz-only tree, but never named npm/cli/package.json and the two versions, so it does not say what to go and fix. It said:"
  quote_case_log
fi
restore_scratch
: > "$state/calls"

# 2. The journey. The same bump, prepared the way the workflow prepares it.
if ! (cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1; then
  quote_case_log
  fatal \
    "scripts/prepare-release-pr.sh failed on a release-plz-shaped bump, so no release pull request can be prepared at all" \
    "read its diagnostic above and fix what it names"
fi
if ! grep -qF "release-plz proposed the package release pull request for $new_version" "$case_log"; then
  report "the preparation succeeded without saying what it proposed, so its workflow log cannot distinguish a release from no release. It said:"
  quote_case_log
fi

# 3. What the required check would say about the tree it prepared.
if ! (cd "$repo" && scripts/set-version.sh --check) > "$case_log" 2>&1; then
  report "the prepared tree still fails scripts/set-version.sh --check, which is what 'just distribution-check' runs — so the release pull request would still fail its own required check. It said:"
  quote_case_log
fi

# 4. The pull request is opened over the synced tree, not beside it: without --allow-dirty
#    release-plz refuses the tree outright, and a release-pr that ran before the sync would
#    carry the drift into the commit however green the tree ended up afterwards.
if ! grep -q -- '^release-pr .*--allow-dirty' "$state/calls"; then
  report "release-plz release-pr was not invoked with --allow-dirty, so the synced manifests never reach the release commit. The calls were:"
  sed 's/^/    /' "$state/calls" >&2
fi
while IFS= read -r carrier; do
  if ! grep -qxF "$carrier" "$state/carried"; then
    report "$carrier is not among the files the release commit would carry, so it stays at $old_version on the release branch"
    continue
  fi
  seen="$state/seen/${carrier//\//__}"
  case "$carrier" in
    *.json) seen_version="$(read_json_version "$seen")" ;;
    *) seen_version="$(read_toml_version "$seen")" ;;
  esac
  [ "$seen_version" = "$new_version" ] || report \
    "$carrier read $seen_version when the release pull request was opened, expected $new_version"
  tree_version="$(manifest_version "$carrier")"
  [ "$tree_version" = "$new_version" ] || report \
    "$carrier reads $tree_version in the prepared tree, expected $new_version"
done <<< "$carriers"
restore_scratch

# 5. One release, described once. `release-pr` runs its own update over the tree it is
#    handed, so an entry written while the version was being selected is kept above the one
#    that update writes, under a version scripts/set-version.sh has since normalised away.
#    Two entries for one set of changes is what jammed #55.
base_changelog="$scratch/base-changelog"
seen_headings="$scratch/seen-headings"
base_headings="$scratch/base-headings"
for changelog_path in "$repo"/crates/*/CHANGELOG.md; do
  changelog="${changelog_path#"$repo"/}"
  seen="$state/seen/${changelog//\//__}"
  if [ ! -f "$seen" ]; then
    report "$changelog is not among the files the release commit would carry, so the release it proposes is described nowhere"
    continue
  fi
  git -C "$repo" show "HEAD:$changelog" > "$base_changelog" || fatal \
    "could not read $changelog as the baseline commit has it" \
    "check the scratch repository and rerun"
  grep '^## \[' "$seen" > "$seen_headings" || : > "$seen_headings"
  grep '^## \[' "$base_changelog" > "$base_headings" || : > "$base_headings"
  added=0
  while IFS= read -r heading; do
    grep -qxF "$heading" "$base_headings" && continue
    added=$((added + 1))
    case "$heading" in
      "## [$new_version]"*) ;;
      *) report "$changelog would reach the release branch holding '$heading', which describes this release under a version this pull request does not propose — the manifests it carries say $new_version" ;;
    esac
  done < "$seen_headings"
  case "$added" in
    0) report "$changelog reaches the release branch with no entry for $new_version, so the release it proposes is described nowhere" ;;
    1) ;;
    *) report "$changelog reaches the release branch holding $added new entries for one set of changes; a release is described once" ;;
  esac
done

# 6. Preparing again over the same base supersedes the last preparation rather than adding
#    to it. Every push to the default branch runs this path, and three of them updated #55's
#    branch inside half an hour — so what the second run leaves has to be what the first one
#    would have left on its own, whichever version it computes.
restore_scratch
rm -rf "$state/seen" "$state/first-seen" || fatal \
  "could not clear the recording directory before the idempotence case" \
  "check the permissions of \$TMPDIR and rerun"
mkdir -p "$state/seen" || fatal \
  "could not recreate the recording directory before the idempotence case" \
  "check the permissions of \$TMPDIR and rerun"
: > "$state/calls"
if ! (cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1; then
  quote_case_log
  fatal "the first of two preparations failed, so this case never reached the second" \
    "read its diagnostic above and fix what it names"
fi
cp -R "$state/seen" "$state/first-seen" && cp "$state/carried" "$state/first-carried" || fatal \
  "could not record what the first preparation would carry" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
rm -rf "$state/seen" && mkdir -p "$state/seen" || fatal \
  "could not clear the recording directory between the two preparations" \
  "check the permissions of \$TMPDIR and rerun"
# Deliberately no restore between them: a rerun finds the tree the last run left, and
# regenerating from the base is what this case is about.
second_status=0
(cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1 || second_status=$?
if [ "$second_status" -ne 0 ]; then
  report "preparing the release pull request a second time over the same base failed with status $second_status, so a rerun cannot supersede what the last one left. It said:"
  quote_case_log
elif ! diff -r "$state/first-seen" "$state/seen" > "$case_log" 2>&1 || \
  ! diff "$state/first-carried" "$state/carried" >> "$case_log" 2>&1; then
  report "the second preparation would put a different release commit on the branch than the first, so the two compound instead of superseding each other:"
  quote_case_log
fi
restore_scratch

# 7. Every way the preparation can fail is a way a release stalls, so each one has to say
#    which phase stopped and what to do — and none may end with a pull request opened.
expect_refusal "release-plz missing from PATH" \
  "release-plz is not on PATH" 2 \
  env PATH="$PATH_WITHOUT_RELEASE_PLZ" scripts/prepare-release-pr.sh
expect_refusal "no git token in the environment" \
  "GIT_TOKEN is empty" 2 \
  env GIT_TOKEN= scripts/prepare-release-pr.sh
expect_refusal "a token passed as an argument, where every process on the runner can read it" \
  "takes no arguments" 2 \
  scripts/prepare-release-pr.sh --git-token from-the-command-line
expect_refusal "release-plz failing to decide the next version" \
  "release version selection failed" 1 \
  env RELEASE_PLZ_STUB_FAIL=update scripts/prepare-release-pr.sh
expect_refusal "release-plz returning an undefined phase status" \
  "release version selection failed" 1 \
  env RELEASE_PLZ_STUB_FAIL=update RELEASE_PLZ_STUB_FAIL_STATUS=7 scripts/prepare-release-pr.sh
expect_refusal "the selector missing its required Python toolchain" \
  "python3 is not on PATH" 2 \
  env PATH="$path_without_python:$scratch:$PATH_WITHOUT_PYTHON" scripts/prepare-release-pr.sh
expect_refusal "an update that leaves no version to read in the binary's manifest" \
  "no valid semantic version" 1 \
  env RELEASE_PLZ_STUB_INHERIT_VERSION=yes scripts/prepare-release-pr.sh
expect_refusal "release-plz failing to open the pull request" \
  "could not open or update the release pull request" 1 \
  env RELEASE_PLZ_STUB_FAIL=release-pr scripts/prepare-release-pr.sh

# A failing phase has to arrive with what the tool itself said, or the diagnostic names a
# phase and nothing else.
if ! grep -qF "release-plz stand-in: release-pr was asked to fail" "$case_log"; then
  report "the preparation refused without passing on what release-plz said, so the reader is told which phase failed and nothing about why. It said:"
  quote_case_log
fi

#    And the way that stalls before any of them: a checkout the preparation cannot restore.
#    It is the first phase, so nothing has been decided when it fails, and the run must stop
#    rather than prepare a release over the tree the last one left. Induced with the index
#    lock a killed run leaves behind — which is exactly the interruption the restore is for,
#    and, unlike taking away the object the restore reads, it is the same inducement on every
#    platform: the scratch repository holds its objects loose on some runners and packed on
#    others, and this case once failed on windows-latest alone for that reason. It is put back
#    afterwards, because every case below needs a checkout that can restore itself.
restore_blocked=crates/onetaskgraph/Cargo.toml
index_lock="$repo/.git/index.lock"
: > "$index_lock" || fatal \
  "could not leave an index lock behind in the scratch repository" \
  "check the permissions of the scratch repository and rerun"
# The tree has to differ from HEAD, or git restores nothing and the lock is never reached:
# this is the shape a preparation that did not finish leaves, which is the tree the restore
# is for.
printf '\n# left behind by a preparation that did not finish\n' >> "$repo/$restore_blocked" || fatal \
  "could not leave $restore_blocked as an unfinished preparation leaves it" \
  "check the permissions of the scratch repository and rerun"
: > "$state/calls"
unrestorable_status=0
(cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1 || unrestorable_status=$?
if [ "$unrestorable_status" -eq 0 ]; then
  report "a checkout whose tracked files could not be restored was ACCEPTED, so a release would be prepared over whatever the last run left. It said:"
  quote_case_log
elif [ "$unrestorable_status" -ne 1 ]; then
  report "the unrestorable checkout was refused with status $unrestorable_status, but its own contract gives a failed phase 1, so a workflow reading the status cannot tell a wrong call from a failed phase. It said:"
  quote_case_log
else
  if ! grep -qF "could not restore this checkout's tracked files to HEAD" "$case_log"; then
    report "the preparation refused an unrestorable checkout without naming the phase that stopped. It said:"
    quote_case_log
  fi
  if ! grep -qF "index.lock" "$case_log"; then
    report "the refusal never quotes what git said it could not do, so the reader is told which phase failed and nothing about why. It said:"
    quote_case_log
  fi
  if [ -s "$state/calls" ]; then
    report "the preparation went on to release-plz over a checkout it could not restore. The calls were:"
    sed 's/^/    /' "$state/calls" >&2
  fi
fi
rm -f "$index_lock" || fatal \
  "could not take the index lock back out of the scratch repository after the unrestorable-checkout case" \
  "remove $index_lock by hand, then rerun"
restore_scratch
git -C "$repo" diff --quiet -- "$restore_blocked" || fatal \
  "$restore_blocked still differs from the baseline after the unrestorable-checkout case, so every case below would run against a half-bumped tree" \
  "restore the scratch repository and rerun"

# 8. The two ways the sync itself can fail. Replacing the script in the scratch tree rather
#    than the manifests: what is under test here is that the preparation stops, and stops
#    with the failing phase named, whatever made that phase fail.
#    The stand-in is committed rather than left in the working tree, because the preparation
#    restores its tracked files to HEAD before it prepares anything — a fixture it could
#    discard would put the real sync back and the case would prove nothing.
sync_stub() {
  cat > "$repo/scripts/set-version.sh" <<STUB
#!/usr/bin/env bash
set -euo pipefail
$1
STUB
  chmod +x "$repo/scripts/set-version.sh" || fatal \
    "could not make the set-version.sh stand-in executable in $repo" \
    "check the permissions of \$TMPDIR, then rerun"
  git -C "$repo" add scripts/set-version.sh >/dev/null 2>&1 && \
    git -C "$repo" -c user.email=check@example.invalid -c user.name=check \
      commit --quiet --no-verify -m "sync stand-in" >/dev/null 2>&1 || fatal \
    "could not commit the set-version.sh stand-in in $repo" \
    "check that git works ('git --version') and rerun"
}
sync_stub 'echo "set-version stand-in: could not write npm/cli/package.json" >&2; exit 1'
expect_refusal "the sync failing outright" \
  "could not bring every manifest to $new_version" 1 \
  scripts/prepare-release-pr.sh
sync_stub '[ "${1:-}" = --check ] || exit 0
echo "npm/cli/package.json has '"$old_version"'; expected '"$new_version"'" >&2
exit 1'
expect_refusal "drift the sync did not resolve" \
  "the manifests still disagree after the sync" 1 \
  scripts/prepare-release-pr.sh
git -C "$repo" reset --quiet --hard "$baseline_commit" || fatal \
  "could not put the scratch repository back on its baseline commit after the sync cases" \
  "check that git works ('git --version') and rerun"

# 9. The workflow has to open its pull request through the script above; the mechanism
#    cannot help a release pull request opened around it. The pin is a function so the cases
#    after it can watch it refuse — a pin nobody has watched fail is a pin nobody knows works.
workflow_opens_pr_through_the_script() {
  local workflow="$1" steps
  # The commands the workflow RUNS, with its prose dropped: the comment above the step names
  # the command it is not, and a pin reading that would refuse the arrangement it wants.
  steps="$(grep -v '^[[:space:]]*#' "$workflow" | grep '^[[:space:]]*run:')" || steps=""
  if ! grep -q 'run:[[:space:]]*scripts/prepare-release-pr\.sh' <<<"$steps"; then
    echo "no step of the release workflow runs scripts/prepare-release-pr.sh, so the version" >&2
    echo "manifests the pull request proposes would disagree and it would fail 'check'." >&2
    return 1
  fi
  if grep -q 'release-plz release-pr' <<<"$steps"; then
    echo "a step of the release workflow calls 'release-plz release-pr' directly, which bumps the" >&2
    echo "Cargo manifests alone; run scripts/prepare-release-pr.sh, which syncs the rest first." >&2
    return 1
  fi
  return 0
}
workflow="$ROOT/.github/workflows/release-plz.yml"
[ -f "$workflow" ] || fatal \
  "$workflow does not exist, so nothing prepares a release pull request at all" \
  "restore it with 'git checkout -- .github/workflows/release-plz.yml'"
if ! workflow_opens_pr_through_the_script "$workflow" > "$case_log" 2>&1; then
  report "the release workflow goes around scripts/prepare-release-pr.sh:"
  quote_case_log
fi

# python3 rather than sed: the replacements below carry the shell operators a workflow step
# is written with, and every sed delimiter is one of them.
mutate_workflow() {
  local destination="$1" before="$2" after="$3"
  python3 - "$workflow" "$destination" "$before" "$after" <<'MUTATE' || fatal \
    "could not write a mutated workflow copy at $destination (its diagnostic is above)" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
import pathlib
import sys

source, destination = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
before, after = sys.argv[3], sys.argv[4]
text = source.read_text(encoding="utf-8")
if before not in text:
    print(
        f"check-release-pr-sync: {source} no longer contains the text this case rewrites:\n"
        f"    {before}\n"
        "check-release-pr-sync: so the case would put nothing to the pin; update it to the text that replaced it.",
        file=sys.stderr,
    )
    raise SystemExit(3)
destination.write_text(text.replace(before, after, 1), encoding="utf-8")
MUTATE
}

# 10. The pin, watched refusing the workflow this repository had before this journey existed:
#    the step calls release-plz itself. This is the mutation a revert makes.
around="$scratch/around.yml"
mutate_workflow "$around" 'run: scripts/prepare-release-pr.sh' 'run: release-plz release-pr'
if workflow_opens_pr_through_the_script "$around" > "$case_log" 2>&1; then
  report "the pin ACCEPTS a workflow whose step calls 'release-plz release-pr' directly — the arrangement that blocked #26 — so it would not notice a revert"
fi

# 11. The sync stays, but a second call opens the pull request beside it — a retry, a
#    leftover step — and whichever one opens it, the drift is back in the release commit.
beside="$scratch/beside.yml"
mutate_workflow "$beside" \
  'run: scripts/prepare-release-pr.sh' \
  'run: scripts/prepare-release-pr.sh; release-plz release-pr'
if workflow_opens_pr_through_the_script "$beside" > "$case_log" 2>&1; then
  report "the pin ACCEPTS a workflow that opens the pull request with 'release-plz release-pr' beside the sync, so the manifests it carries are whatever that call wrote"
fi

# 12. The stand-in models one release-plz, so the workflow has to install that one. Without
#     this the gate would keep passing against behaviour the real tool no longer has.
installed_release_plz="$(sed -n 's/.*[ ,]release-plz@\([^ ,]*\).*/\1/p' "$workflow" | head -n1)"
if [ "$installed_release_plz" != "$RECORDED_RELEASE_PLZ" ]; then
  report "the workflow installs release-plz '${installed_release_plz:-<unpinned>}' but the stand-in above was recorded from $RECORDED_RELEASE_PLZ, so this journey proves nothing about the tool that will actually prepare the release. Re-observe the real tool — what its update writes, what its release-pr refuses, and what the release commit carries — then move RECORDED_RELEASE_PLZ with the pin"
fi

if [ "$failures" -ne 0 ]; then
  echo "check-release-pr-sync: $failures case(s) failed." >&2
  echo "check-release-pr-sync: a release pull request whose manifests disagree cannot pass its own" >&2
  echo "check-release-pr-sync: required checks, so it never merges and no release is cut. Repair" >&2
  echo "check-release-pr-sync: scripts/prepare-release-pr.sh or the workflow that calls it rather" >&2
  echo "check-release-pr-sync: than relaxing the cases above." >&2
  exit 1
fi
