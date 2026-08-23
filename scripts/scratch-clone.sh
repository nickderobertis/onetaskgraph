#!/usr/bin/env bash
# Clone this repository into a scratch tree, safely enough to run inside a git hook.
#
# Sourced by the two guards that prove themselves against a throwaway copy of the tree —
# check-affected-selection.sh and check-isolation-enforced.sh. It exists because both of
# them were unsafe for one reason, and the reason is invisible at the call site.
#
# Git exports GIT_DIR to every hook it runs, and GIT_DIR takes precedence over `git -C`.
# So inside .githooks/pre-push, which runs the whole gate,
#
#     git -C "$scratch/repo" checkout --quiet "$sha"
#
# does not touch the scratch clone at all. It checks that commit out in the REAL
# repository, exits 0, and leaves the scratch tree empty. That is what rejected a
# publishing push of this branch: the clone came out empty, the first `>>` into it failed
# with "No such file or directory", and the Nx that ran next found no workspace root in
# the scratch tree, resolved itself through the outer worktree instead, and blocked there
# against the sweep that had invoked it.
#
# For check-isolation-enforced.sh the same variable is worse than confusing. Its
# `git -C "$scratch/repo" add -A` and `git -C "$scratch/repo" checkout -- .` would stage
# and then discard in the real working copy, so a guard that exists to protect the tree
# would have been rewriting it. It only escaped because the other guard failed first.
#
# Stripping the environment once, here, is what makes every `git -C` in a caller mean the
# directory it names. Do not reintroduce a bare `git clone`/`git -C` pair in a script the
# gate runs; call this instead.

# Every variable git sets for a hook that can redirect a later command at another
# repository. Unsetting one that was never set is not an error, including under `set -u`.
scratch_clone_strip_git_env() {
  unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY \
    GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_COMMON_DIR GIT_PREFIX GIT_NAMESPACE \
    GIT_INDEX_VERSION GIT_QUARANTINE_PATH
}

# scratch_clone <source-repo> <destination>
#
# Clones the COMMITTED tree — the selections and the isolation rule must be provable from
# what is in git, not from whatever happens to be uncommitted right now — and verifies the
# checkout actually materialised. That verification is the point: an empty scratch tree
# surfaced last time as a shell redirection error twenty lines away from its cause, and
# then as a fifteen-minute block, rather than as the one sentence below.
scratch_clone() {
  local source="$1" dest="$2"

  scratch_clone_strip_git_env

  git clone --quiet --shared --no-checkout "$source" "$dest"
  git -C "$dest" checkout --quiet "$(git -C "$source" rev-parse HEAD)"

  # nx.json is at the root of every commit this repository has, so its absence means the
  # checkout did not land here.
  if [ ! -f "$dest/nx.json" ]; then
    echo "scratch-clone: the clone at $dest came out empty — the checkout landed elsewhere." >&2
    echo "scratch-clone: a GIT_* variable in the environment is redirecting these commands" >&2
    echo "scratch-clone: at another repository. Run 'env | grep ^GIT_' — under a git hook" >&2
    echo "scratch-clone: GIT_DIR is set, and it overrides 'git -C'. Call scratch_clone" >&2
    echo "scratch-clone: from scripts/scratch-clone.sh rather than cloning by hand." >&2
    exit 1
  fi
}
