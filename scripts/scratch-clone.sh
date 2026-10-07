#!/usr/bin/env bash
# Clone this repository into a scratch tree, safely from inside a git hook.
#
# Git exports GIT_DIR to every hook and GIT_DIR overrides `git -C <dir>`. The gate runs
# from .githooks/pre-push, so a guard that clones by hand never touches its scratch tree:
# the checkout lands in the real repository, exits 0, and leaves the clone empty. The
# guards that clone also stage and restore between cases, which under a hook rewrites the
# real working copy.
#
# Stripping the environment once, here, is what makes every `git -C` in a caller mean the
# directory it names. Do not reintroduce a bare clone plus `git -C` in a gate script.

# Every variable git sets for a hook that can redirect a later command at another
# repository. Unsetting an unset variable is not an error, including under `set -u`.
scratch_clone_strip_git_env() {
  unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY \
    GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_COMMON_DIR GIT_PREFIX GIT_NAMESPACE \
    GIT_INDEX_VERSION GIT_QUARANTINE_PATH
}

# scratch_clone <source-repo> <destination>
#
# Clones the COMMITTED tree, because what these guards prove has to be provable from git
# rather than from whatever is uncommitted right now.
scratch_clone() {
  local source="$1" dest="$2"

  scratch_clone_strip_git_env

  local head
  if ! head="$(git -C "$source" rev-parse HEAD 2>&1)"; then
    echo "scratch-clone: could not read HEAD of $source:" >&2
    printf '%s\n' "$head" >&2
    echo "scratch-clone: it must be a git repository with at least one commit." >&2
    return 1
  fi

  local output
  if ! output="$(git clone --quiet --shared --no-checkout "$source" "$dest" 2>&1)"; then
    echo "scratch-clone: could not clone $source into $dest:" >&2
    printf '%s\n' "$output" >&2
    echo "scratch-clone: check that the parent of $dest is writable, and 'df -h' for space." >&2
    return 1
  fi

  if ! output="$(git -C "$dest" checkout --quiet "$head" 2>&1)"; then
    echo "scratch-clone: cloned $source but could not check $head out in $dest:" >&2
    printf '%s\n' "$output" >&2
    echo "scratch-clone: re-run; if it persists, check 'df -h' for a full disk." >&2
    return 1
  fi

  # nx.json is at the root of every commit here, so its absence means the checkout landed
  # somewhere else — which is exactly what a stray GIT_* variable does.
  if [ ! -f "$dest/nx.json" ]; then
    echo "scratch-clone: the clone at $dest came out empty — the checkout landed elsewhere." >&2
    echo "scratch-clone: run 'env | grep ^GIT_'. Under a git hook GIT_DIR is set, and it" >&2
    echo "scratch-clone: overrides 'git -C'; call scratch_clone rather than cloning by hand." >&2
    return 1
  fi
}

# extract_tar_stream <destination>
#
# Unpacks the tar stream on standard input into <destination>, then reads what is left of
# the stream to its end, which macOS's tar does not; scripts/check-tar-drained.sh says why
# that matters and refuses an extraction from a pipe anywhere else.
extract_tar_stream() {
  tar -xf - -C "$1" && cat >/dev/null && return 0
  echo "scratch-clone: could not unpack a tar stream into $1; tar said why above." >&2
  echo "scratch-clone: check that $1 exists and is writable, and 'df -h' for free space." >&2
  return 1
}

# copy_tracked_files <source-repo> <destination> [<pathspec>...]
#
# Copies the files <source-repo> tracks, as they stand in its WORKING tree, into
# <destination> — which is how a guard puts what is under test right now over a clone of
# what was last committed. With pathspecs, only the tracked files they match.
copy_tracked_files() {
  local source="$1" dest="$2"
  shift 2
  if ! (cd "$source" && git ls-files -z -- "$@" | tar --null -T - -cf -) | extract_tar_stream "$dest"; then
    echo "scratch-clone: could not copy the files $source tracks into $dest." >&2
    echo "scratch-clone: run 'git -C $source ls-files' to see that it lists them, then rerun." >&2
    return 1
  fi
}
