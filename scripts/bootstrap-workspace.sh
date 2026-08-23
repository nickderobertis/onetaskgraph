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
if ! setup_output="$(just setup-llmlint 2>&1)"; then
  printf '%s\n' "$setup_output" >&2
  echo "bootstrap-workspace: the judged-lint tier did not install — see above." >&2
  echo "bootstrap-workspace: what still works: 'just check' and 'just gate' are unaffected," >&2
  echo "bootstrap-workspace: because the judged tier is deliberately out of both." >&2
  echo "bootstrap-workspace: what does not: 'just lint-llm', 'just lint-llm-diff' and" >&2
  echo "bootstrap-workspace: 'just lint-llm-validate', which is the blocking 'llmlint' PR check." >&2
  echo "bootstrap-workspace: next action: fix the cause above, then re-run 'just setup-llmlint'." >&2
  echo "bootstrap-workspace: bootstrap itself succeeded; this one optional tier is missing." >&2
fi
