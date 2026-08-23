#!/usr/bin/env bash
# Run one Python project's credential-gated live tests, passing when it has none.
#
# `test-live` is a uniform target on every project, so it has to succeed on a project
# whose live lane is still empty. pytest reports "no tests collected" as exit code 5,
# which is the one non-zero code that means "nothing to do" rather than "something
# broke" — so it is translated here and every other code is passed through unchanged.
#
# Usage: scripts/pytest-live.sh <project-directory>
# llmlint: ignore-file[live_tier_compiles_and_requires_credential] empty live lane passes by design
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

readonly PROJECT="${1:?usage: scripts/pytest-live.sh <project-directory>}"
readonly NO_TESTS_COLLECTED=5

# The argument names a project of this workspace; it is not a path the caller chooses.
# Matched against the real set, so an absolute or traversing path cannot reach pytest.
if ! find sdks -mindepth 1 -maxdepth 1 -type d -exec test -f '{}/pyproject.toml' \; -print \
  | grep -qxF -- "$PROJECT"; then
  echo "pytest-live: $PROJECT is not a Python project of this workspace." >&2
  echo "pytest-live: pass one of: $(find sdks -mindepth 1 -maxdepth 1 -type d \
    -exec test -f '{}/pyproject.toml' \; -print | tr '\n' ' ')" >&2
  exit 1
fi

# --no-cov: the coverage floor in this project's addopts is for the `coverage` target,
# which measures the product. A live run exercises a third-party API and imports almost
# nothing, so measuring it would fail the floor for the one reason that is not a defect.
status=0
report="$(cd "$PROJECT" && uv run --frozen pytest -q --no-cov tests/live 2>&1)" || status=$?

if [ "$status" -eq 0 ] || [ "$status" -eq "$NO_TESTS_COLLECTED" ]; then
  exit 0
fi

printf '%s\n' "$report" >&2
echo "pytest-live: $PROJECT's live tests failed (exit $status) — see above." >&2
exit "$status"
