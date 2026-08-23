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

# Nx's default output narrates every task that succeeded. `dynamic-legacy` keeps only
# minimal logs and always shows errors, which is what a reader wants — but it needs a
# terminal, and in CI the full log IS the artefact. So it is chosen when something is
# watching and left alone otherwise. An explicit --outputStyle from the caller wins.
if [ -t 1 ] && [ -z "${CI:-}" ] && [[ " $* " != *" --outputStyle"* ]]; then
  exec node_modules/.bin/nx "$@" --outputStyle=dynamic-legacy
fi

# Quiet on success. Nx's default narrates every task that succeeded, which is the bulk of
# what a green `just check` prints and none of what a reader needs; `dynamic-legacy` keeps
# minimal logs and always shows errors, so a failing task's own output still comes through
# in full. A caller that passes its own --outputStyle wins.
case " $* " in
  *" --outputStyle"*) exec node_modules/.bin/nx "$@" ;;
  *) exec node_modules/.bin/nx "$@" --outputStyle=dynamic-legacy ;;
esac
