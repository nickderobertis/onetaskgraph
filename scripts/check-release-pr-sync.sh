#!/usr/bin/env bash
# Drive scripts/prepare-release-pr.sh end to end over a real version bump, and prove the
# release pull request it prepares carries version manifests that agree.
#
# The failure this exists to catch is the one that blocked v0.2.0's release pull request
# (#26): release-plz bumps the Cargo manifests and nothing else, so the tree it proposes has
# npm/cli/package.json at the previous version and `just distribution-check` — part of the
# required `check` on all three platforms — refuses it. The bump is the only moment that
# gap can appear, and v0.1.0 needed no bump, so nothing before this had ever run the path.
#
# release-plz is not on PATH in the gate, and putting it there would put crates.io on the
# critical path of a required check. So the tool is stood in for, and the stand-in is not
# invented: every behaviour it reproduces below was observed from release-plz 0.3.160 —
# the version .github/workflows/release-plz.yml installs — driven against this repository
# on 2026-08-26. What is under test is this repository's own scripts, which are real here.
set -euo pipefail

fatal() {
  echo "check-release-pr-sync: $1" >&2
  echo "check-release-pr-sync: next: $2" >&2
  exit 1
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
# from what differs from HEAD, and the stand-in below answers the same question the same
# way. Identity is passed per-command so no global git configuration is required.
git -C "$repo" init --quiet || fatal \
  "could not initialise the scratch repository at $repo" \
  "check that git works ('git --version') and 'df -h' for free space, then rerun"
git -C "$repo" add -A >/dev/null 2>&1 && \
  git -C "$repo" -c user.email=check@example.invalid -c user.name=check \
    commit --quiet --no-verify -m "baseline" >/dev/null 2>&1 || fatal \
  "could not commit the baseline in $repo, so nothing below could tell a bumped file from an unchanged one" \
  "check that git works ('git --version') and 'df -h' for free space, then rerun"

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
# A minor bump, which is what this repository's own policy makes of a `feat` before 1.0 —
# the same bump release-plz computed for v0.2.0.
new_version="${BASH_REMATCH[1]}.$(( BASH_REMATCH[2] + 1 )).0"
readonly old_version new_version

# Every version manifest release-plz does NOT write. Each one is a way the release pull
# request fails its own gate, and the root Cargo.toml is in the list because the version
# under [workspace.package] is one release-plz leaves behind too.
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

# The stand-in. Its three behaviours are the three this repository depends on, and all
# three were observed from release-plz 0.3.160 against this workspace on 2026-08-26:
#
#   `update` rewrites crates/*/Cargo.toml and the [workspace.dependencies] path pins in the
#   root Cargo.toml, refreshes Cargo.lock, and touches no package.json, no pyproject.toml
#   and not the version under [workspace.package] — the real run left all ten of the
#   manifests listed above at 0.1.0 while every crate went to 0.2.0.
#
#   `release-pr` refuses a dirty tree unless --allow-dirty: "the working directory of this
#   project has uncommitted changes ... please commit or stash these changes".
#
#   `release-pr --allow-dirty` builds the release commit from what differs from HEAD. The
#   real run sent exactly those files, contents and all, in the createCommitOnBranch
#   mutation it uses to make that commit — so what the stand-in records here is what the
#   pull request would carry.
cat > "$scratch/release-plz" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
version="$RELEASE_PLZ_STUB_VERSION"
state="$RELEASE_PLZ_STUB_STATE"
subcommand="${1:-}"
shift || true
case "$subcommand" in
  update)
    perl -pi -e "s/^version = \"[^\"]+\"/version = \"$version\"/" crates/*/Cargo.toml
    perl -pi -e "s|(path = \"crates/[^\" ]+\", version = \")[^\"]+|\${1}$version|g" Cargo.toml
    # release-plz runs `cargo update --workspace`; this is the same refresh, and the one
    # scripts/set-version.sh itself uses to keep Cargo.lock in step with a version change.
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

export PATH="$scratch:$PATH"
export RELEASE_PLZ_STUB_VERSION="$new_version"
export RELEASE_PLZ_STUB_STATE="$state"

failures=0
report() {
  failures=$((failures + 1))
  echo "check-release-pr-sync: $1" >&2
}

# 1. The regression itself, watched failing. A release pull request carrying only what
#    release-plz writes is the tree that failed #26, and this repository's own version check
#    has to be the thing that says so — if it ever stops, every case below proves nothing.
update_log="$scratch/update.log"
if ! (cd "$repo" && release-plz update) > "$update_log" 2>&1; then
  sed 's/^/    /' "$update_log" >&2
  fatal \
    "the release-plz stand-in could not bump the scratch tree, so this journey never reached the sync" \
    "read the output above; a cargo that cannot resolve the workspace offline lands here"
fi
drift_log="$scratch/drift.log"
if (cd "$repo" && scripts/set-version.sh --check) > "$drift_log" 2>&1; then
  fatal \
    "scripts/set-version.sh --check ACCEPTED a tree bumped the way release-plz bumps it, so it would not have caught #26 either" \
    "repair that check first — until it refuses this tree, nothing below can prove the sync fixes anything"
fi
if ! grep -qF "npm/cli/package.json has $old_version; expected $new_version" "$drift_log"; then
  report "the version check refused the release-plz-only tree, but never named npm/cli/package.json and the two versions, so it does not say what to go and fix. It said:"
  sed 's/^/    /' "$drift_log" >&2
fi
git -C "$repo" checkout --quiet -- . || fatal \
  "could not restore the scratch tree after the regression case, so the journey below would run against a half-bumped tree" \
  "rerun; if it persists, check 'df -h' for a full disk"
: > "$state/calls"

# 2. The journey. The same bump, prepared the way the workflow prepares it.
prepare_log="$scratch/prepare.log"
if ! (cd "$repo" && scripts/prepare-release-pr.sh --repo-url https://example.invalid/owner/repo) \
  > "$prepare_log" 2>&1; then
  sed 's/^/    /' "$prepare_log" >&2
  fatal \
    "scripts/prepare-release-pr.sh failed on a release-plz-shaped bump, so no release pull request can be prepared at all" \
    "read its diagnostic above and fix what it names"
fi

# 3. What the required check would say about the tree it prepared.
agreement_log="$scratch/agreement.log"
if ! (cd "$repo" && scripts/set-version.sh --check) > "$agreement_log" 2>&1; then
  report "the prepared tree still fails scripts/set-version.sh --check, which is what 'just distribution-check' runs — so the release pull request would still fail its own required check. It said:"
  sed 's/^/    /' "$agreement_log" >&2
fi

# 4. The pull request is opened over the synced tree, not beside it. Both halves matter:
#    without --allow-dirty release-plz refuses the tree outright, and a release-pr that ran
#    before the sync would carry the drift into the commit however green the tree ended up.
if ! grep -q -- '^release-pr .*--allow-dirty' "$state/calls"; then
  report "release-plz release-pr was not invoked with --allow-dirty, so the synced manifests never reach the release commit. The calls were:"
  sed 's/^/    /' "$state/calls" >&2
fi
if ! grep -q -- '^release-pr .*--repo-url https://example.invalid/owner/repo' "$state/calls"; then
  report "the arguments passed to scripts/prepare-release-pr.sh did not reach release-pr, so the workflow could not hand it a git token. The calls were:"
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

# 5. The workflow has to go through the script above; the mechanism cannot help a release
#    pull request opened around it. The pin is a function so the case after it can watch it
#    refuse — a pin nobody has watched fail is a pin nobody knows works.
workflow_opens_pr_through_the_script() {
  local workflow="$1" commands
  # What the workflow RUNS, with its prose dropped: the comment above the step names the
  # command it is not, and a pin that read that would refuse the very arrangement it wants.
  commands="$(grep -v '^[[:space:]]*#' "$workflow")"
  if ! grep -q 'scripts/prepare-release-pr.sh' <<<"$commands"; then
    echo "the release workflow does not open its pull request through scripts/prepare-release-pr.sh," >&2
    echo "so the version manifests it proposes would disagree and the pull request would fail 'check'." >&2
    return 1
  fi
  if grep -q 'release-plz release-pr' <<<"$commands"; then
    echo "the release workflow calls 'release-plz release-pr' directly, which bumps the Cargo manifests" >&2
    echo "alone; run scripts/prepare-release-pr.sh instead, which syncs the rest before opening it." >&2
    return 1
  fi
  return 0
}
workflow="$ROOT/.github/workflows/release-plz.yml"
[ -f "$workflow" ] || fatal \
  "$workflow does not exist, so nothing prepares a release pull request at all" \
  "restore it with 'git checkout -- .github/workflows/release-plz.yml'"
pin_log="$scratch/pin.log"
if ! workflow_opens_pr_through_the_script "$workflow" > "$pin_log" 2>&1; then
  report "the release workflow goes around scripts/prepare-release-pr.sh:"
  sed 's/^/    /' "$pin_log" >&2
fi

# 6. The pin, watched refusing the workflow this repository had before this journey existed:
#    the step calls release-plz itself, and the release pull request carries the Cargo side
#    alone. This is the mutation a revert makes.
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
around="$scratch/around.yml"
mutate_workflow "$around" 'run: scripts/prepare-release-pr.sh' 'run: release-plz release-pr'
if workflow_opens_pr_through_the_script "$around" > "$pin_log" 2>&1; then
  report "the pin ACCEPTS a workflow whose step calls 'release-plz release-pr' directly — the exact arrangement that blocked #26 — so it would not notice a revert"
fi

# 7. The sync stays, but a second call opens the pull request beside it — a retry, a
#    leftover step — and whichever one opens it, the drift is back in the release commit.
beside="$scratch/beside.yml"
mutate_workflow "$beside" \
  'run: scripts/prepare-release-pr.sh' \
  'run: scripts/prepare-release-pr.sh || release-plz release-pr'
if workflow_opens_pr_through_the_script "$beside" > "$pin_log" 2>&1; then
  report "the pin ACCEPTS a workflow that opens the pull request with 'release-plz release-pr' beside the sync, so the manifests it carries are whatever that call wrote"
fi

if [ "$failures" -ne 0 ]; then
  echo "check-release-pr-sync: $failures case(s) failed." >&2
  echo "check-release-pr-sync: a release pull request whose manifests disagree cannot pass its own" >&2
  echo "check-release-pr-sync: required checks, so it never merges and no release is cut. Repair" >&2
  echo "check-release-pr-sync: scripts/prepare-release-pr.sh or the workflow that calls it rather" >&2
  echo "check-release-pr-sync: than relaxing the cases above." >&2
  exit 1
fi
