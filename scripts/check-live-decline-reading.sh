#!/usr/bin/env bash
# Prove that scripts/check-live-decline.sh reads which Exclusivity a lane opens with the same
# way on every runner.
#
# It once piped the comment-stripped journey into `grep -q` under `pipefail`. `grep -q` exits
# on its first match, so whenever `sed` still had text to write it took SIGPIPE and the
# pipeline read as no match: `check (ubuntu-latest)` reported the Linear journey, which opens
# `Exclusivity::OneAtATime`, as naming no Exclusivity at all, while the same script over the
# same file passed on a slower machine. How much text follows the match is what decides the
# race, so each journey here names its variant first and then carries far more than a pipe
# buffer after it — which makes the old form misread it on every run rather than on some.
#
# The real guard runs from a scratch tree of its own, with a stand-in `cargo` first on PATH
# answering the three outcomes the guard drives. The stand-in is the guard's collaborator, not
# the guard: what is under test is only how the guard reads the journey and what it concludes.
set -euo pipefail

fatal() {
  echo "check-live-decline-reading: $1" >&2
  echo "check-live-decline-reading: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT
readonly GUARD="scripts/check-live-decline.sh"
readonly CRATE="onetaskgraph-reading-fixture"

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree the fixture journeys are written into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

mkdir -p "$scratch/tree/scripts" "$scratch/tree/crates/$CRATE/tests" "$scratch/bin" || fatal \
  "could not lay out the scratch tree under $scratch" \
  "check 'df -h' for free space, then rerun"
cp "$ROOT/$GUARD" "$scratch/tree/$GUARD" || fatal \
  "could not copy $GUARD into the scratch tree" \
  "restore it with 'git checkout -- $GUARD', then rerun"
printf '{"targets": {"test": {"options": {"commands": []}}}}\n' > "$scratch/tree/crates/$CRATE/project.json"

# The three outcomes as the journey reports them, keyed on the environment the guard sets.
cat > "$scratch/bin/cargo" <<'CARGO'
#!/usr/bin/env bash
if [ -f "${ONETASKGRAPH_LIVE_SEAT_DIR:-}" ]; then
  echo "live session DID NOT RUN: not a test failure in the code under test; no seat could be taken under ONETASKGRAPH_LIVE_SEAT_DIR"
  exit 101
fi
if [ "${ONETASKGRAPH_LIVE_REQUIRED:-}" = 1 ]; then
  echo "no credential, and ONETASKGRAPH_LIVE_REQUIRED=1 demanded one"
  exit 101
fi
echo "live session skipped: no credential"
CARGO
chmod +x "$scratch/bin/cargo"

# A first line, then about 400 KB of ordinary code with no comment on it, so every byte of it
# survives the comment strip and follows the first line into whatever reads it.
write_journey() {
  {
    printf '%s\n' "$1"
    # awk rather than `yes | head`, which is this very defect: `yes` takes SIGPIPE.
    awk 'BEGIN { for (i = 0; i < 16000; i++) print "    let padding = Some(1);" }'
  } > "$scratch/tree/crates/$CRATE/tests/live.rs"
}

failures=0
fail() {
  echo "check-live-decline-reading: $1" >&2
  failures=$((failures + 1))
}

OUTPUT=""
STATUS=0
run_guard() {
  OUTPUT="$(cd "$scratch/tree" && PATH="$scratch/bin:$PATH" bash "$GUARD" "$CRATE" 2>&1)" \
    && STATUS=0 || STATUS=$?
}

write_journey '    let session = Session::open(SESSION_NAME, key, Exclusivity::OneAtATime);'
run_guard
case "$OUTPUT" in
  *"does not say which Exclusivity"*)
    fail "a journey opening Exclusivity::OneAtATime, followed by more text than a pipe holds, was read as naming no Exclusivity. Match the stripped journey without a pipeline whose writer can take SIGPIPE, then rerun. Output: $OUTPUT"
    ;;
esac
if [ "$STATUS" -ne 0 ]; then
  fail "$GUARD refused a journey opening Exclusivity::OneAtATime whose three outcomes the stand-in reported correctly (exit $STATUS). Read the guard's own diagnostic in the output and fix what it names in $GUARD, then rerun. Output: $OUTPUT"
fi

# The other half, so a guard that answers OneAtATime for everything cannot pass the case above.
write_journey '    // A note that mentions Exclusivity::OneAtATime without opening a session with it.'
run_guard
case "$OUTPUT" in
  *"does not say which Exclusivity"*) ;;
  *) fail "a journey that names Exclusivity::OneAtATime only in a comment was not refused as naming none (exit $STATUS). Strip line comments in $GUARD before matching, then rerun. Output: $OUTPUT" ;;
esac
if [ "$STATUS" -eq 0 ]; then
  fail "a journey that names no Exclusivity outside a comment passed $GUARD. Make the guard fail when no Exclusivity is named, then rerun. Output: $OUTPUT"
fi

write_journey '    let session = Session::open(SESSION_NAME, key, Exclusivity::OneAtATime); let other = Exclusivity::Shared;'
run_guard
case "$OUTPUT" in
  *"both Exclusivity::OneAtATime and Exclusivity::Shared"*) ;;
  *) fail "a journey naming both variants was not refused as ambiguous (exit $STATUS). Make $GUARD test every variant and refuse a second match, then rerun. Output: $OUTPUT" ;;
esac

if [ "$failures" -ne 0 ]; then
  echo "check-live-decline-reading: $failures expectation(s) failed; the reading is in $GUARD." >&2
  exit 1
fi
