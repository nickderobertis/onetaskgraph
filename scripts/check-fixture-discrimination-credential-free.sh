#!/usr/bin/env bash
# Watch the fixture-discrimination step run with no way to reach a real API.
#
# That step runs the WHOLE onetaskgraph-github-projects package's tests, twice, in a scratch
# copy, on every gate — it sits outside the affected-selection fan-out on purpose — and that
# package's `tests/live.rs` is an ordinary test of it. So while the ambient credentials
# reached the `cargo` it invokes, the step opened a real GitHub Projects session on every
# gate of every branch, including branches whose diff reaches no plugin behaviour at all: the
# account's shared GraphQL allowance spent on a claim about in-process fixtures, and a lane
# elsewhere declined for want of the budget this one drew down.
#
# `scripts/check-live-lane.sh` asserts the `unset` line is there. That is a claim about the
# text of a script, and the property is about what a process receives — so this drives the
# REAL step, with all three variables seeded PRESENT, behind a stand-in `cargo` that records
# the environment of every invocation it is given, and reads those recordings back. A
# sentinel seeded beside the credentials is required to arrive in each one: without it, a
# recording that captured nothing at all would report every credential as absent and read
# exactly like a pass.
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
[ -r "$ROOT/$STEP" ] || fatal \
  "$STEP is missing, so the step this check drives does not exist" \
  "restore it with 'git checkout -- $STEP', or point this check at where it moved to"

# The three the live crate and the workflow name, which together are everything a session
# needs to be opened and everything that turns a skip into a failure.
readonly CREDENTIALS="GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED"
# Seeded beside them and never cleared: its arrival is what makes their absence evidence.
readonly PROBE=FIXTURE_DISCRIMINATION_ENVIRONMENT_PROBE

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check records into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

mkdir -p "$scratch/bin" "$scratch/invocations" || fatal \
  "could not lay out the stand-in toolchain under $scratch" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

# The stand-in. It answers the two outcomes the step demands of a real `cargo test` — green
# on the unmutated tree, then a harness that compiled, ran and FAILED on the mutated one —
# so the step runs to its own conclusion rather than stopping early on a tool it cannot use.
# It finds where to record from its own path rather than from a variable, so that a step
# which later cleared more of the environment could not silently stop it recording.
#
# The names alone, not the values: a value is never written to disk here, and a name is the
# whole of what is being asked — a credential exported as the empty string is still a name
# the step failed to clear.
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

# The real entry point the gate invokes, not a helper lifted out of it: `scripts:test` and
# `scripts:distribution-test` both run this file by this name.
STEP_OUTPUT=""
STEP_STATUS=0
# `env` rather than an assignment prefix, because the probe's name is held in a variable
# and bash reads `"$PROBE"=present` as an argument rather than as an assignment.
STEP_OUTPUT="$(
  env "PATH=$scratch/bin:$PATH" \
    GH_PROJECTS_TOKEN=not-a-real-token-seeded-by-this-check \
    LINEAR_API_KEY=not-a-real-key-seeded-by-this-check \
    ONETASKGRAPH_LIVE_REQUIRED=1 \
    "$PROBE=present" \
    bash "$STEP" 2>&1
)" && STEP_STATUS=0 || STEP_STATUS=$?

report_step_output() {
  printf '%s\n' "$STEP_OUTPUT" | sed 's/^/    /' >&2
}

if [ "$STEP_STATUS" -ne 0 ]; then
  echo "check-fixture-discrimination-credential-free: $STEP did not run to its own" >&2
  echo "check-fixture-discrimination-credential-free: conclusion behind the stand-in cargo, so" >&2
  echo "check-fixture-discrimination-credential-free: the recordings below say nothing about" >&2
  echo "check-fixture-discrimination-credential-free: what its invocations received. It said:" >&2
  report_step_output
  echo "check-fixture-discrimination-credential-free: next: if the step now needs something" >&2
  echo "check-fixture-discrimination-credential-free: more of cargo than a pass and then a" >&2
  echo "check-fixture-discrimination-credential-free: 'test result: FAILED', teach the stand-in" >&2
  echo "check-fixture-discrimination-credential-free: in this check to answer it." >&2
  exit 1
fi

recordings="$(find "$scratch/invocations" -name '*.names' | sort)" || fatal \
  "could not list what the stand-in cargo recorded under $scratch/invocations" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

# Two, because the step runs the suite unmutated and then mutated. Fewer means the stand-in
# was not what the step invoked, and a check reading no recording would find no credential
# in it and pass.
count=0
if [ -n "$recordings" ]; then
  count="$(printf '%s\n' "$recordings" | wc -l | tr -d ' ')"
fi
if [ "$count" -lt 2 ]; then
  echo "check-fixture-discrimination-credential-free: the stand-in cargo recorded $count" >&2
  echo "check-fixture-discrimination-credential-free: invocation(s), and $STEP runs the suite" >&2
  echo "check-fixture-discrimination-credential-free: twice — so this check read fewer" >&2
  echo "check-fixture-discrimination-credential-free: environments than the step handed out and" >&2
  echo "check-fixture-discrimination-credential-free: would pass on a step that cleared nothing." >&2
  echo "check-fixture-discrimination-credential-free: next: confirm the step still resolves" >&2
  echo "check-fixture-discrimination-credential-free: cargo from PATH rather than an absolute" >&2
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
    echo "check-fixture-discrimination-credential-free: the credentials and nothing clears — so" >&2
    echo "check-fixture-discrimination-credential-free: the recording is not the environment" >&2
    echo "check-fixture-discrimination-credential-free: that invocation was given, and a" >&2
    echo "check-fixture-discrimination-credential-free: credential missing from it is evidence" >&2
    echo "check-fixture-discrimination-credential-free: of nothing." >&2
    failures=$((failures + 1))
    continue
  fi
  for name in $CREDENTIALS; do
    if grep -qx -- "$name" "$recording"; then
      echo "check-fixture-discrimination-credential-free: invocation $invocation of cargo" >&2
      echo "check-fixture-discrimination-credential-free: received $name. $STEP runs the whole" >&2
      echo "check-fixture-discrimination-credential-free: onetaskgraph-github-projects package," >&2
      echo "check-fixture-discrimination-credential-free: whose tests/live.rs opens a real" >&2
      echo "check-fixture-discrimination-credential-free: session — on every gate of every" >&2
      echo "check-fixture-discrimination-credential-free: branch, since this step is outside" >&2
      echo "check-fixture-discrimination-credential-free: affected selection." >&2
      failures=$((failures + 1))
    fi
  done
done <<EOF
$recordings
EOF

if [ "$failures" -ne 0 ]; then
  echo "check-fixture-discrimination-credential-free: $failures problem(s) above." >&2
  echo "check-fixture-discrimination-credential-free: next: restore 'unset $CREDENTIALS'" >&2
  echo "check-fixture-discrimination-credential-free: near the top of $STEP. AGENTS.md records" >&2
  echo "check-fixture-discrimination-credential-free: why one session per lane per run is what" >&2
  echo "check-fixture-discrimination-credential-free: this protects." >&2
  exit 1
fi
