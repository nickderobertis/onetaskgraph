#!/usr/bin/env bash
# Workspace-level setup a clean clone needs before any target can run.
#
# Activates the repository's own git hooks (so `git push` runs the complete gate rather
# than discovering it in CI) and installs the judged-lint toolchain. Everything here is
# idempotent, so `just bootstrap` is safe to re-run.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Point git at the tracked hooks. Without this the pre-push gate is a file nobody runs.
git config core.hooksPath .githooks

# The judged tier is out of `just check`, but a contributor should not have to discover
# how to install it. Through the documented recipe, so there is one way to do it.
just setup-llmlint >/dev/null 2>&1 \
  || echo "bootstrap-workspace: llmlint did not install; run 'just setup-llmlint' to see why (the deterministic gate does not need it)" >&2
