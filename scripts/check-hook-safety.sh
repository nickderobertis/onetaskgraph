#!/usr/bin/env bash
# Prove that the scratch-clone helper survives the environment a git hook runs it in.
#
# Git exports GIT_DIR to hooks and it overrides `git -C`, so a guard that clones can
# operate on the real repository instead of its scratch tree. Run by hand there is no
# GIT_DIR, which is why that shipped, so the hostile environment is set up here on purpose.
#
# Every case runs against a throwaway fixture repository. A check that reproduced the
# hazard against this one would be the hazard.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly HELPER="$ROOT/scripts/scratch-clone.sh"

# Strip the ambient git environment before building anything of our own; the hostile
# GIT_DIR is set per-command below. This runs inside the gate, which runs from the hook, so
# GIT_DIR is set at the first line — without this, the `git init` that builds the fixture
# reinitialises the real repository and the fixture commits land on the real branch.
# shellcheck source=scripts/scratch-clone.sh
source "$HELPER"
scratch_clone_strip_git_env

if [ -n "${GIT_DIR:-}" ] || [ -n "${GIT_WORK_TREE:-}" ] || [ -n "${GIT_INDEX_FILE:-}" ]; then
  echo "check-hook-safety: the git environment is still set after stripping it, so every" >&2
  echo "check-hook-safety: git command below would run against another repository. Fix" >&2
  echo "check-hook-safety: scratch_clone_strip_git_env in scripts/scratch-clone.sh." >&2
  exit 1
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

failures=0

fail() {
  echo "check-hook-safety: $1" >&2
  failures=$((failures + 1))
}

# A fixture repository shaped like this one only where it matters: nx.json at the root is
# what the helper verifies materialised, and the marker is a stand-in for the file whose
# missing parent directory produced the original "No such file or directory".
fixture="$scratch/fixture"
mkdir -p "$fixture/crates/pkg/src"
git init --quiet -b main "$fixture"
git -C "$fixture" config user.email "check-hook-safety@invalid"
git -C "$fixture" config user.name "check-hook-safety"
echo '{}' > "$fixture/nx.json"
echo 'marker' > "$fixture/crates/pkg/src/lib.rs"
git -C "$fixture" add -A
git -C "$fixture" commit --quiet -m "fixture"

# The environment git gives a hook, pointed at the fixture.
hostile_git_dir="$(git -C "$fixture" rev-parse --absolute-git-dir)"

fixture_head_before="$(git -C "$fixture" rev-parse HEAD)"

# 1. The hazard is real, and it is silent. The pattern both guards used before this fix —
#    a bare clone plus `git -C <dest> checkout` — must be shown to leave the destination
#    empty while reporting success, or the rest of this check is asserting about nothing.
if ! GIT_DIR="$hostile_git_dir" bash -c '
    set -eu
    git clone --quiet --shared --no-checkout "$1" "$2"
    git -C "$2" checkout --quiet "$(git -C "$1" rev-parse HEAD)"
  ' _ "$fixture" "$scratch/bare-pattern" >/dev/null 2>&1; then
  fail "the unguarded clone pattern failed outright under GIT_DIR; this check expected it
check-hook-safety: to succeed silently. Re-read it: the hazard it reproduces may have
check-hook-safety: changed shape, and the assertions below may no longer mean what they say."
elif [ -f "$scratch/bare-pattern/nx.json" ]; then
  fail "the unguarded clone pattern checked out correctly under GIT_DIR, so the hazard
check-hook-safety: this check exists to pin is no longer reproducible here. Confirm that
check-hook-safety: before relaxing anything: git's precedence rules are what changed."
fi

# 2. The helper resists it. Same hostile GIT_DIR, and the tree has to be really there.
if GIT_DIR="$hostile_git_dir" bash -c '
    set -eu
    # shellcheck source=scripts/scratch-clone.sh
    source "$1"
    scratch_clone "$2" "$3"
  ' _ "$HELPER" "$fixture" "$scratch/guarded" >/dev/null 2>&1; then
  [ -f "$scratch/guarded/nx.json" ] \
    || fail "scratch_clone reported success under GIT_DIR but left no nx.json behind."
  [ -f "$scratch/guarded/crates/pkg/src/lib.rs" ] \
    || fail "scratch_clone reported success under GIT_DIR but the tree is not checked out:
check-hook-safety: crates/pkg/src/lib.rs is missing, which is the shape of the failure that
check-hook-safety: rejected a publishing push."
else
  fail "scratch_clone failed under GIT_DIR, the one environment it exists to survive."
fi

# 3. It reached nowhere near the repository GIT_DIR named. This is the property that makes
#    the other guard's `add -A` and `checkout -- .` safe to run at all.
if [ "$(git -C "$fixture" rev-parse HEAD)" != "$fixture_head_before" ]; then
  fail "scratch_clone moved HEAD in the repository GIT_DIR pointed at."
fi
if [ -n "$(git -C "$fixture" status --porcelain)" ]; then
  fail "scratch_clone dirtied the working tree of the repository GIT_DIR pointed at."
fi

# 4. When a clone really does come out empty, it says so instead of failing twenty lines
#    later as a redirection error. A repository whose HEAD carries no nx.json stands in for
#    any future cause of an empty scratch tree.
empty="$scratch/no-nx-json"
mkdir -p "$empty"
git init --quiet -b main "$empty"
git -C "$empty" config user.email "check-hook-safety@invalid"
git -C "$empty" config user.name "check-hook-safety"
echo 'nothing' > "$empty/README.md"
git -C "$empty" add -A
git -C "$empty" commit --quiet -m "no nx.json"

diagnostic="$(bash -c '
    source "$1"
    scratch_clone "$2" "$3"
  ' _ "$HELPER" "$empty" "$scratch/empty-dest" 2>&1)" && status=0 || status=$?

if [ "${status:-0}" -eq 0 ]; then
  fail "scratch_clone accepted a clone with no nx.json in it, so an empty scratch tree
check-hook-safety: would again surface as an error somewhere else entirely."
else
  case "$diagnostic" in
    *"came out empty"*) ;;
    *) fail "scratch_clone refused an empty clone without saying it came out empty. Its
check-hook-safety: diagnostic was: $diagnostic" ;;
  esac
  case "$diagnostic" in
    *GIT_*) ;;
    *) fail "scratch_clone refused an empty clone without naming GIT_* as the cause, which
check-hook-safety: is the one thing the next reader needs. Its diagnostic was: $diagnostic" ;;
  esac
fi

if [ "$failures" -ne 0 ]; then
  echo "check-hook-safety: $failures expectation(s) failed." >&2
  echo "check-hook-safety: the gate runs from a git hook, where GIT_DIR is set and silently" >&2
  echo "check-hook-safety: overrides 'git -C'. Until this passes, a guard that clones may be" >&2
  echo "check-hook-safety: operating on the real repository. See scripts/scratch-clone.sh." >&2
  exit 1
fi
