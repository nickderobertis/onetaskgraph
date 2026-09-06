#!/usr/bin/env bash
# Prove the live-lane selection decides the same way whatever code page the host's python
# decodes a subprocess with.
#
# The decision compares a file read through `git show` against the same file read off the
# disk, and text mode with no encoding decodes the first with the platform's own — UTF-8
# here, the ANSI code page on the Windows runner. AGENTS.md records what that cost.
#
# So this is the real decision and the real check, run again through a python whose code
# page is that runner's. Nothing stands in for either — only for the platform, which is the
# variable under test. Cases 1 and 2 refuse to let case 3 pass vacuously: a shim that did
# not take, and a tree with no byte the two decodings disagree about, would each look
# exactly like a pass.
set -euo pipefail

fatal() {
  echo "check-selection-code-page: $1" >&2
  echo "check-selection-code-page: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

# On Windows the host's own python already decodes this way, so the run `just script-check`
# does there IS the native case this simulates. Simulating it a second time would only put
# this shim's portability on the required check it exists to protect.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-selection-code-page: skipped on Windows (its own code page is the one this simulates; the real check runs there natively)" >&2
    exit 0
    ;;
esac

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check runs its simulation from" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# Every python3 the runs below reach imports this, through PYTHONPATH, and it does one
# thing: hand back the encoding a Windows python picks when nobody names one. That is what
# text mode consults, so it reaches every place a decoding is left to the platform without
# touching how any of them is invoked.
cat > "$scratch/sitecustomize.py" <<'PY' || fatal \
  "could not write the code-page shim in $scratch, so nothing would be simulated" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
"""The code page the Windows runner's python decodes with, and nothing else."""

import locale

# cp1252 is that runner's ANSI code page. It maps every byte to some character rather than
# refusing one, which is why the defect this simulates is silent: the decoding succeeds and
# hands back a different string.
locale.getpreferredencoding = lambda do_setlocale=True: "cp1252"
locale.getencoding = lambda: "cp1252"
PY

failures=0
fail() {
  echo "check-selection-code-page: $1" >&2
  failures=$((failures + 1))
}

# 1. The simulation takes. Everything below is evidence only while this holds: a shim that
# did nothing would leave the cases passing on the very platform they cannot fail on.

simulated="$(PYTHONPATH="$scratch" python3 - <<'PY'
import subprocess
import sys

finished = subprocess.run(
    [sys.executable, "-c", 'import sys; sys.stdout.buffer.write("\\u2014".encode("utf-8"))'],
    capture_output=True,
    text=True,
    check=False,
)
print("simulated" if finished.stdout != "—" else "not-simulated")
PY
)" || fatal \
  "could not run the code-page shim's own probe" \
  "run 'python3 -c \"import locale; print(locale.getencoding())\"' to confirm this python has an encoding to override, then rerun"

if [ "$simulated" != "simulated" ]; then
  fatal \
    "the code-page shim did not take: python3 still decoded a subprocess as UTF-8, so every case below would pass without simulating anything" \
    "confirm this python asks locale.getencoding() when text mode is given no encoding, then rerun"
fi

# 2. This tree still holds a byte the two decodings disagree about, in a file a version-only
# diff touches. Without one, case 3 would pass on any python at all and this would quietly
# stop being a guard.

carrier="$(cd "$ROOT" && python3 - <<'PY'
from pathlib import Path

candidates = [Path("Cargo.toml"), Path("Cargo.lock")]
candidates.extend(sorted(Path("crates").glob("*/Cargo.toml")))
for candidate in candidates:
    try:
        raw = candidate.read_bytes()
    except OSError:
        continue
    try:
        raw.decode("ascii")
    except UnicodeDecodeError:
        print(candidate.as_posix())
        break
PY
)" || fatal \
  "could not look for a non-ASCII byte among the paths a version-only diff touches" \
  "run the check from a checkout of this repository, as 'just script-check' does"

if [ -z "$carrier" ]; then
  fatal \
    "no manifest a version-only diff touches carries a byte the two decodings disagree about, so case 3 below would pass whatever python it ran on and this check has stopped proving anything" \
    "that is a weakening rather than a repair: either restore the case by driving the decision over a fixture that carries such a byte, or retire this check and record in AGENTS.md why the decoding no longer needs watching"
fi

echo "check-selection-code-page: $carrier is what makes the run below evidence" >&2

# 3. The real check, over the real decision, on the simulated platform. It is the whole of
# the proof: its version-only fixtures are exactly the diffs the defect answered `run` for.

if ! (cd "$ROOT" && PYTHONPATH="$scratch" bash scripts/check-live-lane-selection.sh) \
  >"$scratch/selection-output" 2>&1; then
  fail "scripts/check-live-lane-selection.sh fails when python decodes a subprocess with the Windows runner's code page, which is what the windows-latest lane runs. It said:"
  sed 's/^/    /' <"$scratch/selection-output" >&2
fi

if [ "$failures" -ne 0 ]; then
  echo "check-selection-code-page: $failures expectation(s) failed." >&2
  echo "check-selection-code-page: text a python reads back from a subprocess has to be" >&2
  echo "check-selection-code-page: decoded with the encoding it was written in. Name that" >&2
  echo "check-selection-code-page: encoding — 'encoding=\"utf-8\"', as" >&2
  echo "check-selection-code-page: scripts/live-lane-selection.sh does — rather than leaving" >&2
  echo "check-selection-code-page: text mode to pick the platform's. Then rerun" >&2
  echo "check-selection-code-page: 'just script-check'." >&2
  exit 1
fi
