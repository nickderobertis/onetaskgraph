#!/usr/bin/env bash
# The three steps of the Rust line-coverage floor, over one shared profile directory.
#
#   rust-coverage.sh --clear     remove the raw profiles an earlier run left behind
#   rust-coverage.sh <crate>     run one crate's tests instrumented, keeping its profiles
#   rust-coverage.sh --report    the one report over every crate's run, held to the floor
#
# Every crate's `coverage` target runs the middle step and every one of them writes into
# cargo-llvm-cov's own `target/llvm-cov-target`, under the shared target directory
# `.cargo/config.toml` declares — no directory a script chose. `--no-report` is what makes
# that safe to run in parallel under Nx: a plain `cargo llvm-cov` clears every raw profile
# in the directory before it starts, so two crates sharing it deleted each other's
# coverage, while a `--no-report` run clears nothing and leaves its profiles for the
# report. The profiles are cleared once instead, before the fan-out, by the target every
# crate's run depends on; and the floor is enforced once, over the union, by the
# `workspace` project's target that depends on every crate's run — never per crate and
# never lowered. AGENTS.md records what the union costs.
#
# On Windows every step is skipped with a printed notice: LLVM instrumentation there does
# not attribute coverage from the binary the e2e journeys spawn, so the number would
# understate the binary crate and mean nothing for the rest. The functional lanes (lint,
# typecheck, test) still run on all three platforms, so Windows is still gated.
set -euo pipefail

readonly STEP="${1:?usage: scripts/rust-coverage.sh --clear | <crate-name> | --report}"
readonly MIN_LINES=95

# ONE live session per run of the gate, and this is where the second one would come from.
# `just check` runs `test` AND `coverage`, and `cargo llvm-cov --no-report --package <crate>`
# below re-runs those same integration tests — so credentials left set here would open a
# second session against the shared external fixture the first may still be writing to.
# Neither session would delete the other's work — every artifact carries its writing run's
# own process id and the sweep that recovers an interrupted run's is decided by that stamp,
# in `onetaskgraph_live::artifact` — but a second session is a second session's worth of a
# third party's budget, spent to measure tests llvm-cov never measured anyway.
#
# The demand is cleared with them, or the skip that clearing produces would fail this phase
# for a session it is deliberately not running. Nothing is lost: llvm-cov never measured
# these tests. The other half of "one session per run" is the platform matrix, which hands
# the credentials to one of three `check` lanes. Changing the matrix or this target means
# keeping that pair true.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "rust-coverage: $STEP skipped on Windows (instrumentation there does not attribute subprocess coverage); the functional lanes still gate this platform" >&2
    exit 0
    ;;
esac

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "rust-coverage: cargo-llvm-cov is not installed." >&2
  echo "rust-coverage: install it with 'cargo binstall cargo-llvm-cov' and re-run." >&2
  exit 1
fi

# Raw profiles only: the instrumented build stays, so the next run is incremental. Nothing
# reads them across runs — a stale profile counts lines a since-changed binary no longer
# runs — which is why every crate's run depends on this step rather than trusting it.
if [ "$STEP" = "--clear" ]; then
  if ! output="$(cargo llvm-cov clean --workspace --profraw-only 2>&1)"; then
    printf '%s\n' "$output" >&2
    echo "rust-coverage: could not clear the raw profiles under target/llvm-cov-target; fix what cargo-llvm-cov reports above, then re-run." >&2
    exit 1
  fi
  exit 0
fi

# The per-file table and the uncovered line numbers are exactly what you need when the
# union is under the bar, and noise when it is over — so they are held and replayed only
# on failure.
if [ "$STEP" = "--report" ]; then
  if ! report="$(cargo llvm-cov report \
    --summary-only \
    --show-missing-lines \
    --fail-under-lines "$MIN_LINES" 2>&1)"; then
    printf '%s\n' "$report" >&2
    echo "rust-coverage: the workspace is below ${MIN_LINES}% line coverage over every crate's run." >&2
    echo "rust-coverage: the uncovered lines are listed above — cover them with a test that" >&2
    echo "rust-coverage: drives the real behaviour, not one written to move the number." >&2
    exit 1
  fi
  exit 0
fi

readonly CRATE="$STEP"

# Validate the crate name against the real workspace before it is used as a Cargo
# selector. Unchecked, a typo becomes a silent "measured nothing".
if ! metadata="$(cargo metadata --format-version 1 --no-deps 2>&1)"; then
  echo "rust-coverage: could not read the workspace metadata:" >&2
  printf '%s\n' "$metadata" >&2
  echo "rust-coverage: fix the manifest so 'cargo metadata' resolves, then re-run." >&2
  exit 1
fi

# tr: $CRATE is matched against this list with `grep -qxF`, which a trailing CR defeats.
if ! members="$(printf '%s' "$metadata" \
  | python3 -c '
import json, sys
document = json.load(sys.stdin)
packages = document.get("packages") if isinstance(document, dict) else None
if not isinstance(packages, list) or not all(
    isinstance(package, dict) and isinstance(package.get("name"), str) for package in packages
):
    raise SystemExit("no packages array of named packages")
print("\n".join(package["name"] for package in packages))
' \
  | tr -d '\r')"; then
  echo "rust-coverage: 'cargo metadata' did not answer with a packages array of named packages." >&2
  echo "rust-coverage: fix the manifest so 'cargo metadata' resolves, then re-run." >&2
  exit 1
fi

if ! printf '%s\n' "$members" | grep -qxF -- "$CRATE"; then
  echo "rust-coverage: $CRATE is not a member of this Cargo workspace. Members:" >&2
  printf '  %s\n' $members >&2
  echo "rust-coverage: re-run with one of those names, or with --clear or --report." >&2
  exit 1
fi

# The e2e journeys spawn the built binary; cargo-llvm-cov exports the profile path into
# that subprocess, so its coverage is attributed rather than lost. A failing test is the
# only failure here — the floor is the report's to enforce.
if ! run="$(cargo llvm-cov \
  --no-report \
  --package "$CRATE" \
  --all-features \
  --locked 2>&1)"; then
  printf '%s\n' "$run" >&2
  echo "rust-coverage: $CRATE's tests failed under instrumentation — fix the failures named above, then re-run." >&2
  exit 1
fi
