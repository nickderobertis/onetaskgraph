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

# Validate the crate name against the real workspace before it is used as a Cargo
# selector and as a path segment. Unchecked, a typo becomes a silent "measured nothing"
# and a caller-supplied string reaches the filesystem.
if ! cargo metadata --format-version 1 --no-deps 2>/dev/null \
  | python3 -c 'import json,sys; print("\n".join(p["name"] for p in json.load(sys.stdin)["packages"]))' \
  | grep -qx "$CRATE"; then
  echo "rust-coverage: $CRATE is not a member of this Cargo workspace." >&2
  echo "rust-coverage: run 'cargo metadata --no-deps' to see the members, then pass one." >&2
  exit 1
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "rust-coverage: cargo-llvm-cov is not installed." >&2
  echo "rust-coverage: install it with 'cargo binstall cargo-llvm-cov' and re-run." >&2
  exit 1
fi

# Nx runs these targets in parallel, and cargo-llvm-cov clears the raw profile data in
# its target directory before each run — so two crates sharing one directory delete each
# other's coverage and both report a number that is not theirs. A directory per crate is
# what makes per-crate measurement safe to parallelise.
export CARGO_LLVM_COV_TARGET_DIR="${CARGO_LLVM_COV_TARGET_DIR:-target/llvm-cov/$CRATE}"

# The e2e journeys spawn the built binary; cargo-llvm-cov exports the profile path into
# that subprocess, so its coverage is attributed to this crate rather than lost.
#
# The per-file table and the uncovered line numbers are exactly what you need when the
# crate is under the bar, and noise when it is over — so they are held and replayed only
# on failure.
if ! report="$(cargo llvm-cov \
  --package "$CRATE" \
  --all-features \
  --locked \
  --summary-only \
  --show-missing-lines \
  --fail-under-lines "$MIN_LINES" 2>&1)"; then
  printf '%s\n' "$report" >&2
  echo "rust-coverage: $CRATE is below ${MIN_LINES}% line coverage." >&2
  echo "rust-coverage: the uncovered lines are listed above — cover them with a test that" >&2
  echo "rust-coverage: drives the real behaviour, not one written to move the number." >&2
  exit 1
fi
