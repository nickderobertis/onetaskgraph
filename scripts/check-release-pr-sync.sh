#!/usr/bin/env bash
# Drive scripts/prepare-release-pr.sh over a real version bump and prove the release pull
# request it prepares carries version manifests that agree.
#
# The bump is the only moment they can part, and v0.1.0 needed none, so nothing before #26
# had ever run this path. release-plz itself is stood in for below rather than installed:
# putting it in the gate would put crates.io on the critical path of a required check. Every
# behaviour the stand-in reproduces was observed from the pinned release-plz, driven against
# this repository; the pin and this file are reconciled in case 10. What is under test is
# this repository's own scripts, which are real here.
#
# Exit status: 0, the preparation carries what it must; 1, a case below failed and the
# release pull request would not pass its own checks; 2, this check could not be run at all
# — a scratch tree it could not build, a toolchain it could not find — which is a different
# thing from a finding and reads differently in a gate log.
set -euo pipefail

# The release-plz the stand-in below was recorded from. The workflow installs exactly this
# version, and case 10 fails when the two part: a stand-in models one version's behaviour.
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
readonly old_version new_version

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

# The stand-in, and the three behaviours this repository depends on, all three observed from
# the pinned release-plz against this workspace:
#
#   `update` rewrites crates/*/Cargo.toml and the [workspace.dependencies] path pins, and
#   refreshes Cargo.lock. It leaves every manifest in `carriers` above untouched — the real
#   run left all ten at 0.1.0 while every crate went to 0.2.0.
#
#   `release-pr` refuses a dirty tree unless --allow-dirty.
#
#   `release-pr --allow-dirty` builds the release commit from what differs from HEAD: the
#   real run sent exactly those files, contents and all, in the createCommitOnBranch mutation
#   that makes the commit. So what it records here is what the pull request would carry.
cat > "$scratch/release-plz" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
version="$RELEASE_PLZ_STUB_VERSION"
state="$RELEASE_PLZ_STUB_STATE"
subcommand="${1:-}"
shift || true
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
    perl -pi -e "s/^version = \"[^\"]+\"/version = \"$version\"/" crates/*/Cargo.toml
    perl -pi -e "s|(path = \"crates/[^\" ]+\", version = \")[^\"]+|\${1}$version|g" Cargo.toml
    # One case asks for a bump that leaves no version to read: cargo accepts an inherited
    # one, so this is a refactor the workspace could really take, and the reader of the
    # release's version would silently get an empty string.
    if [ "${RELEASE_PLZ_STUB_INHERIT_VERSION:-}" = yes ]; then
      perl -pi -e 's/^version = "[^"]+"/version.workspace = true/' crates/onetaskgraph/Cargo.toml
    fi
    # release-plz runs `cargo update --workspace`; this is the same refresh, and the one
    # scripts/set-version.sh uses to keep Cargo.lock in step with a version change.
    cargo metadata --format-version 1 >/dev/null
    echo "update $*" >> "$state/calls"
    ;;
  release-pr)
    echo "release-pr $*" >> "$state/calls"
    allow_dirty=no
    for argument in "$@"; do
      [ "$argument" = --allow-dirty ] && allow_dirty=yes
    done
    if [ -n "$(git status --porcelain)" ] && [ "$allow_dirty" = no ]; then
      echo "the working directory of this project has uncommitted changes." >&2
      echo "Otherwise, please commit or stash these changes" >&2
      exit 1
    fi
    git status --porcelain | sed 's/^...//' > "$state/carried"
    while IFS= read -r carried; do
      [ -f "$carried" ] || continue
      cp "$carried" "$state/seen/${carried//\//__}"
    done < "$state/carried"
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
if [ -s "$case_log" ]; then
  report "the preparation succeeded but was not quiet, so a real failure would arrive inside routine output. It said:"
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

# 5. Every way the preparation can fail is a way a release stalls, so each one has to say
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

# 6. The two ways the sync itself can fail. Replacing the script in the scratch tree rather
#    than the manifests: what is under test here is that the preparation stops, and stops
#    with the failing phase named, whatever made that phase fail.
sync_stub() {
  cat > "$repo/scripts/set-version.sh" <<STUB
#!/usr/bin/env bash
set -euo pipefail
$1
STUB
  chmod +x "$repo/scripts/set-version.sh" || fatal \
    "could not make the set-version.sh stand-in executable in $repo" \
    "check the permissions of \$TMPDIR, then rerun"
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

# 7. The workflow has to open its pull request through the script above; the mechanism
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

# 8. The pin, watched refusing the workflow this repository had before this journey existed:
#    the step calls release-plz itself. This is the mutation a revert makes.
around="$scratch/around.yml"
mutate_workflow "$around" 'run: scripts/prepare-release-pr.sh' 'run: release-plz release-pr'
if workflow_opens_pr_through_the_script "$around" > "$case_log" 2>&1; then
  report "the pin ACCEPTS a workflow whose step calls 'release-plz release-pr' directly — the arrangement that blocked #26 — so it would not notice a revert"
fi

# 9. The sync stays, but a second call opens the pull request beside it — a retry, a
#    leftover step — and whichever one opens it, the drift is back in the release commit.
beside="$scratch/beside.yml"
mutate_workflow "$beside" \
  'run: scripts/prepare-release-pr.sh' \
  'run: scripts/prepare-release-pr.sh; release-plz release-pr'
if workflow_opens_pr_through_the_script "$beside" > "$case_log" 2>&1; then
  report "the pin ACCEPTS a workflow that opens the pull request with 'release-plz release-pr' beside the sync, so the manifests it carries are whatever that call wrote"
fi

# 10. The stand-in models one release-plz, so the workflow has to install that one. Without
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
