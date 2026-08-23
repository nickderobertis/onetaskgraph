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

# One line to say what is running, then nothing unless it fails — at which point the whole
# of Nx's output is replayed, because that is when every line of it is worth reading.
echo "nx: $*" >&2
if ! output="$(node_modules/.bin/nx "$@" --outputStyle=static 2>&1)"; then
  printf '%s\n' "$output" >&2
  exit 1
fi
