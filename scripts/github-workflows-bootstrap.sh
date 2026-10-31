#!/usr/bin/env bash
# Install .github's workflow-test dependencies, one installer at a time per checkout.
#
#   scripts/github-workflows-bootstrap.sh
#
# github-workflows:bootstrap is a dependency of every workflow-policy project, and
# scripts/ci-job.sh runs a job's phases side by side, so two phases can each reach it at once:
# two `bun install`s linking into one node_modules failed test-macos with `Failed to link
# @onebudgetspec/cli: EEXIST`. Holding scripts/file-lock.sh's lock serialises them, and the
# second install then finds nothing to change, so it costs a job no more than that one no-op.
# The lock sits under .nx/, outside the node_modules an install rewrites.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
if [ ! -r "$ROOT/scripts/file-lock.sh" ]; then
  echo "github-workflows-bootstrap: $ROOT/scripts/file-lock.sh is missing" >&2
  echo "github-workflows-bootstrap: next: restore it with 'git checkout -- scripts/file-lock.sh', then rerun" >&2
  exit 1
fi
# shellcheck source=scripts/file-lock.sh
source "$ROOT/scripts/file-lock.sh"
with_file_lock "$ROOT/.nx/locks/github-workflows-bootstrap.lock" \
  bun install --cwd "$ROOT/.github" --frozen-lockfile
