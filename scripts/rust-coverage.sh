#!/usr/bin/env bash
# Measure one crate's own line coverage and fail below the bar.
#
# Per-crate rather than workspace-wide, for two reasons. A workspace average lets a weak
# crate hide behind a strong one. And — decisively — a workspace-wide pass runs every
# crate's tests on every change, which is exactly what affected selection exists to avoid.
#
# On Windows the *measurement* is skipped with a printed notice: LLVM instrumentation
# there does not attribute coverage from the binary the e2e journeys spawn, so the number
# would understate the binary crate and mean nothing for the rest. The functional lanes
# (lint, typecheck, test) still run on all three platforms, so Windows is still gated.
#
# Usage: scripts/rust-coverage.sh <crate-name>
set -euo pipefail

readonly CRATE="${1:?usage: scripts/rust-coverage.sh <crate-name>}"
readonly MIN_LINES=95

case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "rust-coverage: skipping the coverage measurement for $CRATE on Windows —" >&2
    echo "rust-coverage: instrumentation there does not attribute subprocess coverage." >&2
    echo "rust-coverage: the functional lanes still gate this platform." >&2
    exit 0
    ;;
esac

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "rust-coverage: cargo-llvm-cov is not installed." >&2
  echo "rust-coverage: install it with 'cargo binstall cargo-llvm-cov' and re-run." >&2
  exit 1
fi

# The e2e journeys spawn the built binary; cargo-llvm-cov exports the profile path into
# that subprocess, so its coverage is attributed to this crate rather than lost.
exec cargo llvm-cov \
  --package "$CRATE" \
  --all-features \
  --locked \
  --summary-only \
  --show-missing-lines \
  --fail-under-lines "$MIN_LINES"
