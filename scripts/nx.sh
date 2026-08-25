#!/usr/bin/env bash
# Invoke Nx, healing a fresh worktree first.
#
# Every `just` recipe delegates through here rather than calling `nx` directly, so a
# clone that has never run `bun install` — a new worktree, a CI checkout, a contributor's
# first five minutes — gets the locked install rather than "nx: command not found". The
# install is skipped once node_modules/nx exists, so the common path costs one stat.
#
# Usage: scripts/nx.sh <nx arguments...>
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ ! -x "node_modules/.bin/nx" ]; then
  if ! command -v bun >/dev/null 2>&1; then
    echo "nx.sh: bun is not installed, and Nx orchestrates every target in this" >&2
    echo "nx.sh: workspace. Install it from https://bun.sh, then re-run." >&2
    exit 1
  fi
  echo "nx.sh: installing the locked Node toolchain (first run in this worktree)" >&2
  if ! install_log="$(bun install --frozen-lockfile 2>&1)"; then
    printf '%s\n' "$install_log" >&2
    echo "nx.sh: the locked install failed — see above. Try 'bun install' by hand." >&2
    exit 1
  fi
fi

# A caller that picked its own output style means it.
case " $* " in
  *" --outputStyle"*) exec node_modules/.bin/nx "$@" ;;
esac

# Buffering is for commands whose output is progress. A query's output IS the answer, so
# it goes straight through — swallowing it would break every caller that reads it.
case "${1:-}" in
  run | run-many | affected) ;;
  *) exec node_modules/.bin/nx "$@" ;;
esac

# TIMING PROBE BRANCH ONLY — do not merge.
#
# The committed script buffers Nx's output and replays it only on failure, so a green CI
# run carries no per-task timing at all and the Windows lane's 75 minutes cannot be
# attributed. Here it streams, one task at a time, so the GitHub log's per-line timestamps
# localise every second to a project, a target, and a compiled crate.
echo "nx: $* (streaming, parallel=1)" >&2
exec node_modules/.bin/nx "$@" --outputStyle=stream --parallel=1
