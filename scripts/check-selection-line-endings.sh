#!/usr/bin/env bash
# Prove the live-lane selection decides the same way whatever line ending the host's python
# writes.
#
# Both answer through python, which opens stdout in text mode — so on the Windows runner
# every "\n" they print leaves as "\r\n" while on Linux and macOS it does not, and a
# command substitution keeps the carriage return. AGENTS.md records what that cost. Either
# side of this seam is a real defect: a caller reading `not-selected\r` runs the very lane
# the decision refused, and this check's own captured version rides into a fixture as
# `version = "0.2.24<CR>"`, which python reads back as two lines.
#
# So this is the real decision and the real check, run again through a python that ends its
# lines the way the Windows runner's does. Nothing stands in for either — only for the
# platform, which is the variable under test. Case 1 refuses to let the rest pass
# vacuously, because a simulation that did not take would look exactly like a pass.
set -euo pipefail

fatal() {
  echo "check-selection-line-endings: $1" >&2
  echo "check-selection-line-endings: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

# On Windows the host's own python already ends its lines this way, so the run `just
# script-check` does there IS the native case this simulates. Simulating it a second time
# would only put this shim's portability on the required check it exists to protect.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-selection-line-endings: skipped on Windows (its own line ending is the one this simulates; the real check runs there natively)" >&2
    exit 0
    ;;
esac

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check runs its simulation from" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# Every python3 the runs below reach imports this, through PYTHONPATH, and it does one
# thing: hand back the newline translation Windows gives them by default. Reaching them
# through the interpreter's own startup rather than through a wrapper on PATH is what makes
# it cover all three spellings the scripts use — `python3 -c`, a heredoc on stdin, and a
# file — without touching how any of them is invoked. It shadows whatever sitecustomize the
# platform installs, which is the point: what a simulated platform runs is this one.
cat > "$scratch/sitecustomize.py" <<'PY' || fatal \
  "could not write the line-ending shim in $scratch, so nothing would be simulated" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
"""Line endings the way the Windows runner's python writes them, and nothing else."""

import sys

for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(newline="\r\n")
    except (AttributeError, ValueError):  # pragma: no cover - a stream that cannot be one
        pass
PY

failures=0
fail() {
  echo "check-selection-line-endings: $1" >&2
  failures=$((failures + 1))
}

# The `\r` this check is about, held in a variable: written as a literal it would be a
# carriage return in this file, which git's own `* text=auto eol=lf` would take back out.
CR="$(printf '\r')"
readonly CR

# 1. The simulation takes. Everything below is evidence only while this holds: a shim that
# did nothing would leave the cases passing on the very platform they cannot fail on.

if [ "$(PYTHONPATH="$scratch" python3 -c 'print("simulated")' | tr -d '\n')" != "simulated$CR" ]; then
  fatal \
    "the line-ending shim did not take: python3 still ended its line the way this host does, so every case below would pass without simulating anything" \
    "run 'python3 -c \"import sys; sys.stdout.reconfigure\"' to confirm this python supports reconfigure, then rerun"
fi

# 2. The decision's answer. A caller reads the exact word on stdout, so a carriage return
# anywhere in it is the defect whatever the word is.
#
# Against HEAD there is no diff, so the answer here is `run` and `--nx-exclusions` has
# nothing to exclude. That makes this the cheap half: it reaches the one `print` both modes
# answer through, and it is exactly the assertion a caller makes. The half it cannot reach
# is a state the decision REFUSES, where the same carriage return rides into
# `--exclude=<crate>` — case 3 drives that one, through the real recipes over a real
# version-only commit.

answer="$(cd "$ROOT" && PYTHONPATH="$scratch" bash scripts/live-lane-selection.sh onetaskgraph-github-projects HEAD 2>/dev/null)" \
  || answer="<the decision could not be run>"
if [ "$answer" != "run" ]; then
  fail "the decision did not answer the bare word 'run' for a tree identical to its base; a caller compares against a word, so anything else here is what a caller would read: $(printf '%s' "$answer" | cat -v)"
fi

exclusions="$(cd "$ROOT" && PYTHONPATH="$scratch" bash scripts/live-lane-selection.sh --nx-exclusions HEAD 2>/dev/null)" \
  || exclusions="<the decision could not be run>"
if [ -n "$exclusions" ]; then
  fail "the decision excluded a lane for a tree identical to its base, and whatever it printed the recipe splices straight into Nx's arguments: $(printf '%s' "$exclusions" | cat -v)"
fi

# 3. The real check, over the real decision, on the simulated platform. This is the case
# that reaches the check's own captures — the bumped version its fixtures are built from
# and the names of the targets held outside affected selection — which no assertion about
# the decision alone can reach.

if ! (cd "$ROOT" && PYTHONPATH="$scratch" bash scripts/check-live-lane-selection.sh) \
  >"$scratch/selection-output" 2>&1; then
  fail "scripts/check-live-lane-selection.sh fails when python ends its lines the Windows way, which is what the windows-latest lane runs. It said:"
  sed 's/^/    /' <"$scratch/selection-output" >&2
fi

if [ "$failures" -ne 0 ]; then
  echo "check-selection-line-endings: $failures expectation(s) failed." >&2
  echo "check-selection-line-endings: every value a script here captures from python has to" >&2
  echo "check-selection-line-endings: survive that python ending its lines with a carriage" >&2
  echo "check-selection-line-endings: return. Strip it where it is captured, with 'tr -d' as" >&2
  echo "check-selection-line-endings: scripts/check-plugin-isolation.sh does, or hold the" >&2
  echo "check-selection-line-endings: stream to \"\\n\" where it is written, as" >&2
  echo "check-selection-line-endings: scripts/live-lane-selection.sh does for the one answer" >&2
  echo "check-selection-line-endings: its callers read. Then rerun 'just script-check'." >&2
  exit 1
fi
