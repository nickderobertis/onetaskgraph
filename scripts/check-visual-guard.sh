#!/usr/bin/env bash
# Drive the REAL pre-push screenshot guard through every outcome it owes.
#
# The guard is the local half of the strict visual gate, and every one of its decisions is
# one nothing else can catch: a guard that stopped blocking on drift would let a capture
# reach the workflow already wrong, and one that captured on a push that cannot change a
# shot would put a release build in front of every push. So this drives
# scripts/screenshots-guard.sh itself, in a scratch clone, with the two third-party
# subprocesses stubbed at the seam — the way scripts/check-pre-push-provisioning.sh stubs
# `just` — and asserts what the guard did with each answer.
#
# What is stubbed is screencomp (whose classification is screencomp's own to test) and the
# capture (whose bytes the committed baseline gates and which costs a release build). What
# is real is the guard, the clone, the git history, the ref records and the diff.
#
# Cases:
#   1. no ref records at all            — nothing is being pushed, so nothing is captured
#   2. records, nothing relevant        — the capture is not run and the push goes through
#   3. records, relevant, no drift      — the capture runs and the push goes through
#   4. records, relevant, drift         — the baseline and the gallery are regenerated and
#                                         the push is BLOCKED
#   5. screencomp absent                — a loud skip, no capture, and NOT the
#                                         host-prerequisite marker, which is one tool's
#   6. screencomp absent and required   — the same skip turned into a refusal
#   7. CI                               — the workflow owns it there, so the guard is inert
#   8. the real hook, with real records — the gate runs and the guard receives the very
#                                         records the base was derived from
set -euo pipefail

fatal() {
  echo "check-visual-guard: $1" >&2
  echo "check-visual-guard: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just check' does"
readonly ROOT

# The path is built from $ROOT at run time, so ShellCheck cannot follow it; the directive
# names the file it resolves to. Tested before it is sourced rather than guarded after:
# bash 3.2 ends the shell where `source` cannot find its file, so the handler after `||`
# never runs there and the reader is told nothing about what to restore.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check clones into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

readonly CLONE="$scratch/repo"
scratch_clone "$ROOT" "$CLONE" || fatal \
  "could not clone this repository into $CLONE" \
  "check 'git status' here and the free space on \$TMPDIR, then rerun"
# The clone carries HEAD, and what is under test is the guard as it is right now — so the
# WORKING tree's tracked files go over the top of it, exactly as
# scripts/check-pre-push-provisioning.sh does with the hook. The clone still supplies the
# `.git` directory the ranges below are computed in.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$CLONE" || fatal \
  "could not copy $ROOT's tracked files over the clone at $CLONE" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

git -C "$CLONE" config user.email "check-visual-guard@invalid" >/dev/null
git -C "$CLONE" config user.name "check-visual-guard" >/dev/null

readonly MARKERS="$scratch/markers"

# The stub capture. It records that it ran and writes the index the guard's classify step
# would read, so a case can assert on whether a capture happened at all.
cat > "$CLONE/scripts/screenshots.sh" <<STUB
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\${SHOTS_OUT:-unset}" >> "$MARKERS/captured"
mkdir -p "\${SHOTS_OUT:?the guard has to name where the capture goes}"
printf '{"schema":1,"shots":[]}\n' > "\$SHOTS_OUT/captures.json"
STUB

# The stub screencomp. Each subcommand records its arguments and its stdin, and answers
# with the exit code the case chose — which is the whole of what the guard branches on.
readonly STUB_BIN="$scratch/bin"
mkdir -p "$STUB_BIN" || fatal "could not create $STUB_BIN" \
  "check the permissions of \$TMPDIR, then rerun"
cat > "$STUB_BIN/screencomp" <<STUB
#!/usr/bin/env bash
set -uo pipefail
subcommand="\${1:-none}"
printf '%s\n' "\$*" >> "$MARKERS/\$subcommand.args"
case "\$subcommand" in
  scope)
    cat >> "$MARKERS/scope.stdin"
    exit "\${STUB_SCOPE_EXIT:-0}"
    ;;
  classify) exit "\${STUB_CLASSIFY_EXIT:-0}" ;;
  manifest)
    while [ \$# -gt 0 ]; do
      [ "\$1" = "--output" ] && { printf '{"schema":1,"shots":[]}\n' > "\$2"; break; }
      shift
    done
    exit 0
    ;;
  gallery)
    while [ \$# -gt 0 ]; do
      [ "\$1" = "--output" ] && { mkdir -p "\$2" && : > "\$2/index.html"; break; }
      shift
    done
    exit 0
    ;;
  *) exit 64 ;;
esac
STUB
chmod +x "$CLONE/scripts/screenshots.sh" "$STUB_BIN/screencomp" || fatal \
  "could not make the stubs executable" "check the permissions of \$TMPDIR, then rerun"

# A commit whose diff is what each case's ref records point at. Its content does not decide
# relevance — the stub screencomp answers that — but the guard has to compute a real diff
# from real records to have anything to hand it.
REMOTE_SHA="$(git -C "$CLONE" rev-parse HEAD)"
readonly REMOTE_SHA
printf '\n# touched by scripts/check-visual-guard.sh\n' >> "$CLONE/screencomp.toml"
git -C "$CLONE" add screencomp.toml >/dev/null
git -C "$CLONE" commit --quiet --no-verify -m "test: touch screencomp.toml" >/dev/null
LOCAL_SHA="$(git -C "$CLONE" rev-parse HEAD)"
readonly LOCAL_SHA
readonly RECORDS="refs/heads/main $LOCAL_SHA refs/heads/main $REMOTE_SHA"
readonly HOST_PREREQUISITE="onevcs: host-prerequisite: "

failures=0
GUARD_OUTPUT=""
GUARD_STATUS=0

# Run the real guard out of the scratch clone, under the PATH and the stub answers this
# case chose. `CI` is cleared for every case but the one about it, because this check runs
# inside the gate, which CI itself runs.
run_guard() {
  local records="$1" path="$2"
  # A fresh marker directory per case, at the path the stubs above were written with.
  # `:?` because the next line removes it: an empty expansion would name the root.
  rm -rf "${MARKERS:?the marker directory is unset}"
  mkdir -p "$MARKERS"
  GUARD_OUTPUT="$(printf '%s\n' "$records" | env -u CI -u SCREENCOMP_GUARD_RANGE \
    PATH="$path" "STUB_SCOPE_EXIT=${3:-0}" "STUB_CLASSIFY_EXIT=${4:-0}" \
    "SCREENCOMP_GUARD_REQUIRE=${5:-}" "CI=${6:-}" \
    bash "$CLONE/scripts/screenshots-guard.sh" 2>&1)" && GUARD_STATUS=0 || GUARD_STATUS=$?
}

report() {
  printf '%s\n' "$GUARD_OUTPUT" | sed 's/^/    /' >&2
}

fail() {
  echo "check-visual-guard: $1" >&2
  report
  failures=$((failures + 1))
}

captured() { [ -f "$MARKERS/captured" ]; }
called() { [ -f "$MARKERS/$1.args" ]; }

# 1. No ref records: nothing is being pushed. This is also what keeps
#    scripts/check-pre-push-provisioning.sh, which drives the real hook with an empty
#    stdin, from paying for a capture it is not about.
run_guard "" "$STUB_BIN:$PATH" 3 3
[ "$GUARD_STATUS" -eq 0 ] || fail "with no ref records the guard refused the push:"
captured && fail "with no ref records the guard captured anyway:"
called scope && fail "with no ref records the guard asked screencomp about the push:"

# 2. Records, and nothing in the push can change a shot (scope exits 0).
run_guard "$RECORDS" "$STUB_BIN:$PATH" 0 3
[ "$GUARD_STATUS" -eq 0 ] || fail "a push with nothing screenshot-relevant was refused:"
captured && fail "a push with nothing screenshot-relevant was captured anyway:"
called scope || fail "the guard never asked screencomp whether the push was relevant:"
if ! grep -qF "screencomp.toml" "$MARKERS/scope.stdin" 2>/dev/null; then
  fail "the guard handed screencomp a changed-path list that does not carry the file this push changed:"
fi

# 3. Relevant, and the capture matches the committed baseline.
run_guard "$RECORDS" "$STUB_BIN:$PATH" 3 0
[ "$GUARD_STATUS" -eq 0 ] || fail "an unchanged capture blocked the push:"
captured || fail "a screenshot-relevant push was not captured:"
called classify || fail "the guard captured and never classified the result:"
called manifest && fail "an unchanged capture still rewrote the committed baseline:"

# 4. Relevant, and the capture drifts: the push is blocked, the baseline and the gallery
#    are regenerated so the new bytes can be committed deliberately, and the message says
#    where to look and how to bypass it on purpose.
run_guard "$RECORDS" "$STUB_BIN:$PATH" 3 3
[ "$GUARD_STATUS" -eq 0 ] && fail "a drifted capture did NOT block the push:"
captured || fail "the guard reported drift without capturing:"
called manifest || fail "the guard blocked on drift without refreshing the baseline:"
called gallery || fail "the guard blocked on drift without building the review gallery:"
for term in "shots/review/index.html" "--no-verify" "SCREENSHOTS CHANGED"; do
  grep -qF -- "$term" <<<"$GUARD_OUTPUT" \
    || fail "the drift refusal never mentions '$term', so the reader is not told what to do:"
done

# 5. screencomp is not installed. The guard cannot evaluate the push, so it does NOT skip
#    silently — and it does not print the host-prerequisite marker, which is the pinned
#    release-plz's alone: that marker tells the engine on this host to stop retrying, and a
#    missing screenshot renderer is not that.
readonly NO_SCREENCOMP="$scratch/no-screencomp"
mkdir -p "$NO_SCREENCOMP" || fatal "could not create $NO_SCREENCOMP" \
  "check the permissions of \$TMPDIR, then rerun"
for tool in env bash sed grep git sort mkdir dirname cat cut tr basename; do
  resolved="$(command -v "$tool" 2>/dev/null)" || continue
  printf '#!%s\nexec "%s" "$@"\n' "$(command -v bash)" "$resolved" > "$NO_SCREENCOMP/$tool"
  chmod +x "$NO_SCREENCOMP/$tool"
done
PATH="$NO_SCREENCOMP" command -v screencomp >/dev/null 2>&1 && fatal \
  "screencomp is still reachable from $NO_SCREENCOMP, so this case cannot pose its question" \
  "report this — the whitelist directory is built here and should hold no screencomp"

run_guard "$RECORDS" "$NO_SCREENCOMP" 3 3
[ "$GUARD_STATUS" -eq 0 ] || fail "with screencomp absent the guard refused the push, where the workflow is the backstop:"
captured && fail "with screencomp absent the guard captured without being able to classify:"
grep -qF "screencomp" <<<"$GUARD_OUTPUT" \
  || fail "with screencomp absent the guard skipped without naming what is missing:"
if grep -qF -- "$HOST_PREREQUISITE" <<<"$GUARD_OUTPUT"; then
  fail "the guard printed the host-prerequisite marker, which is the pinned release-plz's alone:"
fi

# 6. The same, on a machine that says a skip is not acceptable.
run_guard "$RECORDS" "$NO_SCREENCOMP" 3 3 1
[ "$GUARD_STATUS" -eq 0 ] && fail "SCREENCOMP_GUARD_REQUIRE=1 did not turn the skip into a refusal:"

# 7. Under CI the visual-docs workflow owns this, and the gate job has no renderer.
run_guard "$RECORDS" "$STUB_BIN:$PATH" 3 3 "" 1
[ "$GUARD_STATUS" -eq 0 ] || fail "the guard refused a push under CI, where the workflow owns the comparison:"
captured && fail "the guard captured under CI, where the visual-docs workflow does that:"

# 8. The hook itself. It reads git's ref records ONCE into a variable and replays them to
#    two consumers — the base the gate selects against, and the guard's range — so a change
#    that dropped either half would leave the gate sweeping every project or the guard
#    blind, and neither shows up in the guard's own cases above. This drives the REAL
#    .githooks/pre-push in a clone of its own, with `just`, the provisioner and the guard
#    stubbed: provisioning is scripts/check-pre-push-provisioning.sh's subject, and a real
#    `just gate` here would be this repository's whole gate run from inside itself.
readonly HOOK_CLONE="$scratch/hook-repo"
scratch_clone "$ROOT" "$HOOK_CLONE" || fatal \
  "could not clone this repository into $HOOK_CLONE" \
  "check 'git status' here and the free space on \$TMPDIR, then rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$HOOK_CLONE" || fatal \
  "could not copy $ROOT's tracked files over the clone at $HOOK_CLONE" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

readonly HOOK_MARKERS="$scratch/hook-markers"
mkdir -p "$HOOK_MARKERS" || fatal "could not create $HOOK_MARKERS" \
  "check the permissions of \$TMPDIR, then rerun"
cat > "$HOOK_CLONE/scripts/provision-gate.sh" <<STUB
#!/usr/bin/env bash
exit 0
STUB
cat > "$HOOK_CLONE/scripts/screenshots-guard.sh" <<STUB
#!/usr/bin/env bash
set -euo pipefail
cat > "$HOOK_MARKERS/guard.stdin"
printf '%s\n' "\${NX_BASE:-unset}" > "$HOOK_MARKERS/guard.nx-base"
STUB
readonly HOOK_STUB_BIN="$scratch/hook-bin"
mkdir -p "$HOOK_STUB_BIN" || fatal "could not create $HOOK_STUB_BIN" \
  "check the permissions of \$TMPDIR, then rerun"
cat > "$HOOK_STUB_BIN/just" <<STUB
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$HOOK_MARKERS/gate"
STUB
chmod +x "$HOOK_CLONE/scripts/provision-gate.sh" \
  "$HOOK_CLONE/scripts/screenshots-guard.sh" "$HOOK_STUB_BIN/just" || fatal \
  "could not make the hook stubs executable" "check the permissions of \$TMPDIR, then rerun"

# From INSIDE the clone, because the hook finds its root with `git rev-parse
# --show-toplevel`: run from anywhere else and it would cd to whichever repository the
# working directory belongs to and drive that one's gate and guard instead.
hook_records="refs/heads/main $(git -C "$HOOK_CLONE" rev-parse HEAD) refs/heads/main $REMOTE_SHA"
hook_output="$(cd "$HOOK_CLONE" && printf '%s\n' "$hook_records" \
  | env -u CI PATH="$HOOK_STUB_BIN:$PATH" bash .githooks/pre-push 2>&1)" \
  && hook_status=0 || hook_status=$?

if [ "$hook_status" -ne 0 ]; then
  GUARD_OUTPUT="$hook_output"
  fail "the real hook refused a push whose gate and guard both succeed:"
fi
if ! grep -qF "gate" "$HOOK_MARKERS/gate" 2>/dev/null; then
  GUARD_OUTPUT="$hook_output"
  fail "the hook never ran 'just gate', so the bar it already had has stopped running:"
fi
if [ "$(cat "$HOOK_MARKERS/guard.stdin" 2>/dev/null)" != "$hook_records" ]; then
  GUARD_OUTPUT="$hook_output"
  fail "the hook did not replay git's ref records to the screenshot guard, which then has no range and captures nothing whatever the push carries:"
fi
if [ "$(cat "$HOOK_MARKERS/guard.nx-base" 2>/dev/null)" != "$REMOTE_SHA" ]; then
  GUARD_OUTPUT="$hook_output"
  fail "the hook did not derive the gate's base from those same records (the guard saw NX_BASE=$(cat "$HOOK_MARKERS/guard.nx-base" 2>/dev/null), expected $REMOTE_SHA):"
fi

if [ "$failures" -ne 0 ]; then
  echo "check-visual-guard: $failures expectation(s) failed." >&2
  echo "check-visual-guard: repair scripts/screenshots-guard.sh so the local half of the" >&2
  echo "check-visual-guard: strict gate blocks on drift and stays out of every other push." >&2
  exit 1
fi
