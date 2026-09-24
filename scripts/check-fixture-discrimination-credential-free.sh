#!/usr/bin/env bash
# Prove the fixture-discrimination step cannot open a live session — on the process, not the
# text.
#
# scripts/check-live-lane.sh asserts that step carries the `unset` line. This drives the REAL
# step with every credential seeded PRESENT, behind a stand-in `cargo` that records the
# environment of each invocation it is handed. A sentinel seeded beside the credentials has
# to arrive in every recording: a recording that captured nothing would report each
# credential absent and read exactly like a pass. AGENTS.md records why that step is where a
# second session per lane would otherwise come from.
set -euo pipefail

fatal() {
  echo "check-fixture-discrimination-credential-free: $1" >&2
  echo "check-fixture-discrimination-credential-free: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT
cd "$ROOT" || fatal \
  "could not enter $ROOT" \
  "check that directory's permissions, then rerun"

readonly STEP="scripts/check-fixture-discrimination.sh"
readonly LANE="scripts/check-live-lane.sh"
for required in "$STEP" "$LANE"; do
  [ -r "$ROOT/$required" ] || fatal \
    "$required is missing, so this check has nothing to drive" \
    "restore it with 'git checkout -- $required', or point this check at where it moved to"
done

# Derived, never restated: $LANE's own SESSIONS map and DEMAND are where the credentials and
# the demand are declared, and scripts/check-credential-names.sh reconciles those names
# against every file that states them.
#
# tr: a value captured from python carries that python's line endings, and on the Windows
# runner each one arrives as CR LF.
CREDENTIALS_AND_DEMAND="$(python3 - "$ROOT/$LANE" <<'PY'
import pathlib
import re
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
credentials = re.findall(r'"credential": "(\w+)"', text)
demand = re.search(r'(?m)^DEMAND = "(\w+)"$', text)
if not credentials or not demand:
    print("no SESSIONS credentials, or no DEMAND, to read there", file=sys.stderr)
    raise SystemExit(1)
sys.stdout.write("".join(f"{name}\n" for name in sorted({*credentials, demand.group(1)})))
PY
)" || fatal \
  "could not read the credentials and the demand out of $LANE" \
  "restore that guard's SESSIONS map and its DEMAND, or point this check at where they moved"
CREDENTIALS_AND_DEMAND="$(printf '%s' "$CREDENTIALS_AND_DEMAND" | tr -d '\r')"
# Seeded beside them and never cleared: its arrival is what makes their absence evidence.
readonly PROBE=FIXTURE_DISCRIMINATION_ENVIRONMENT_PROBE

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check records into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

mkdir -p "$scratch/bin" "$scratch/invocations" || fatal \
  "could not lay out the stand-in toolchain under $scratch" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

# The stand-in answers the two outcomes the step demands — green, then a harness that
# compiled, ran and FAILED — so the step reaches its own conclusion. It finds where to record
# from its own path rather than from a variable, so a step that later cleared more of the
# environment could not silently stop it recording. Names only, never values: a name is the
# whole of what is asked, and a credential exported empty is still one the step failed to
# clear.
cat >"$scratch/bin/cargo" <<'STANDIN'
#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
record="$here/../invocations"
index=1
while [ -e "$record/$index.names" ]; do index=$((index + 1)); done
python3 -c 'import os, sys; sys.stdout.write("".join(name + "\n" for name in sorted(os.environ)))' \
  >"$record/$index.names"
if [ "$index" -eq 1 ]; then
  echo "test result: ok. 128 passed; 0 failed; 0 ignored"
  exit 0
fi
echo "test result: FAILED. 127 passed; 1 failed; 0 ignored"
exit 101
STANDIN
chmod +x "$scratch/bin/cargo" || fatal \
  "could not make the stand-in cargo at $scratch/bin/cargo executable" \
  "check the permissions of \$TMPDIR — a noexec mount is the usual cause — then rerun"

# One placeholder value for all of them: only the NAME is asked about here, and nothing
# credential-shaped is written anywhere.
seeded=("PATH=$scratch/bin:$PATH" "$PROBE=present")
for name in $CREDENTIALS_AND_DEMAND; do
  seeded+=("$name=seeded-by-this-check-and-never-a-real-credential")
done

STEP_OUTPUT=""
STEP_STATUS=0
# llmlint: ignore[work_goes_through_command_surface] The step is the SUBJECT here, and what
# is under test is the environment of the process the gate starts — so this check has to be
# what starts it, with the stand-in cargo on that process's own PATH. `just
# distribution-test` would run two unrelated commands beside it, take minutes of real cargo,
# and need an Nx these scratch trees never install. The same reason
# scripts/check-fixture-discrimination.sh invokes cargo directly.
STEP_OUTPUT="$(env "${seeded[@]}" bash "$STEP" 2>&1)" && STEP_STATUS=0 || STEP_STATUS=$?

if [ "$STEP_STATUS" -ne 0 ]; then
  echo "check-fixture-discrimination-credential-free: $STEP did not reach its own conclusion" >&2
  echo "check-fixture-discrimination-credential-free: behind the stand-in cargo, so the" >&2
  echo "check-fixture-discrimination-credential-free: recordings say nothing about what its" >&2
  echo "check-fixture-discrimination-credential-free: invocations received. It said:" >&2
  printf '%s\n' "$STEP_OUTPUT" | sed 's/^/    /' >&2
  echo "check-fixture-discrimination-credential-free: next: if the step now needs more of" >&2
  echo "check-fixture-discrimination-credential-free: cargo than a pass and then a" >&2
  echo "check-fixture-discrimination-credential-free: 'test result: FAILED', teach the" >&2
  echo "check-fixture-discrimination-credential-free: stand-in in this check to answer it." >&2
  exit 1
fi

recordings="$(find "$scratch/invocations" -name '*.names' | sort)" || fatal \
  "could not list what the stand-in cargo recorded under $scratch/invocations" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

count=0
if [ -n "$recordings" ]; then
  count="$(printf '%s\n' "$recordings" | wc -l | tr -d ' ')"
fi
# Two, because the step runs the suite unmutated and then mutated. Fewer means this check
# read fewer environments than the step handed out, and would find no credential in them.
if [ "$count" -lt 2 ]; then
  echo "check-fixture-discrimination-credential-free: the stand-in cargo recorded $count" >&2
  echo "check-fixture-discrimination-credential-free: invocation(s), and $STEP runs the suite" >&2
  echo "check-fixture-discrimination-credential-free: twice — so this check would pass on a" >&2
  echo "check-fixture-discrimination-credential-free: step that cleared nothing." >&2
  echo "check-fixture-discrimination-credential-free: next: confirm the step still resolves" >&2
  echo "check-fixture-discrimination-credential-free: cargo from PATH rather than by absolute" >&2
  echo "check-fixture-discrimination-credential-free: path, so a stand-in can be put in front" >&2
  echo "check-fixture-discrimination-credential-free: of it." >&2
  exit 1
fi

failures=0
while IFS= read -r recording; do
  [ -n "$recording" ] || continue
  invocation="$(basename "$recording" .names)"
  if ! grep -qx -- "$PROBE" "$recording"; then
    echo "check-fixture-discrimination-credential-free: invocation $invocation never received" >&2
    echo "check-fixture-discrimination-credential-free: $PROBE, which this check seeds beside" >&2
    echo "check-fixture-discrimination-credential-free: the credentials and nothing clears, so" >&2
    echo "check-fixture-discrimination-credential-free: a credential missing from that" >&2
    echo "check-fixture-discrimination-credential-free: recording is evidence of nothing." >&2
    echo "check-fixture-discrimination-credential-free: next: repair the recording rather than" >&2
    echo "check-fixture-discrimination-credential-free: the step — check that the stand-in's" >&2
    echo "check-fixture-discrimination-credential-free: python3 still writes every name of its" >&2
    echo "check-fixture-discrimination-credential-free: own environment, and that $STEP does" >&2
    echo "check-fixture-discrimination-credential-free: not start its cargo with a cleared" >&2
    echo "check-fixture-discrimination-credential-free: environment ('env -i' or similar)," >&2
    echo "check-fixture-discrimination-credential-free: which would need a probe it preserves." >&2
    failures=$((failures + 1))
    continue
  fi
  for name in $CREDENTIALS_AND_DEMAND; do
    if grep -qx -- "$name" "$recording"; then
      echo "check-fixture-discrimination-credential-free: invocation $invocation of cargo" >&2
      echo "check-fixture-discrimination-credential-free: received $name, so $STEP can open a" >&2
      echo "check-fixture-discrimination-credential-free: live session — on every gate of every" >&2
      echo "check-fixture-discrimination-credential-free: branch, since it runs a whole hosted" >&2
      echo "check-fixture-discrimination-credential-free: plugin's package from outside" >&2
      echo "check-fixture-discrimination-credential-free: affected selection." >&2
      echo "check-fixture-discrimination-credential-free: next: clear $name near the top of" >&2
      echo "check-fixture-discrimination-credential-free: $STEP, as scripts/rust-coverage.sh" >&2
      echo "check-fixture-discrimination-credential-free: clears it and for the reason" >&2
      echo "check-fixture-discrimination-credential-free: AGENTS.md records." >&2
      failures=$((failures + 1))
    fi
  done
done <<EOF
$recordings
EOF

if [ "$failures" -ne 0 ]; then
  echo "check-fixture-discrimination-credential-free: $failures problem(s) above, each with" >&2
  echo "check-fixture-discrimination-credential-free: what to do about it." >&2
  exit 1
fi
