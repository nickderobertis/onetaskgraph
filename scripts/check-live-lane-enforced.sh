#!/usr/bin/env bash
# Prove that scripts/check-live-lane.sh REFUSES the arrangements it exists to refuse.
#
# That guard passes on this repository, and it would pass just as quietly if one of its
# scans had stopped matching what it describes. Two of its assertions were exactly that
# when this was written: it refused a live test marked `#[ignore]` but not one the build
# leaves out with `cfg`, and it accepted a credential-carrying step that merely MENTIONED
# `ONETASKGRAPH_LIVE_REQUIRED` — which `ONETASKGRAPH_LIVE_REQUIRED: 0` does, and that is
# the value which leaves the lane free to skip. Neither hole was visible from a passing
# run, because on this repository both arrangements are correct.
#
# So each one is introduced for real, in a scratch clone, and every case asserts on the
# DIAGNOSTIC as well as the exit status: a guard that refuses without naming the file and
# the value sends the next author hunting, which is most of what the guard is for. The
# last case is the one that keeps the rest honest — the fork exception, which the guard
# must ACCEPT, so that a guard refusing every tree cannot satisfy the cases above.
set -euo pipefail

fatal() {
  echo "check-live-lane-enforced: $1" >&2
  echo "check-live-lane-enforced: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

readonly JOURNEY="crates/onetaskgraph-github-projects/tests/live.rs"
readonly WORKFLOW=".github/workflows/ci.yml"
readonly LIVE_CRATE="crates/onetaskgraph-live/src/lib.rs"

# Tested before it is sourced, not merely guarded after: bash 3.2 ends the shell where
# `source` cannot find its file, so a handler after `||` never runs there — and
# macos-latest is a 3.2 runner.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal \
    "could not load $ROOT/scripts/scratch-clone.sh, which is how every case below gets a tree it may break" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh', then rerun"
fi

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree these cases mutate" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# A clone of the committed tree, so a fixture is torn down by restoring from git and cannot
# leave a broken workflow or an ignored journey behind in anybody's working copy. Through
# scripts/scratch-clone.sh, which strips the git environment first: under a git hook GIT_DIR
# overrides `git -C`, so the `git add -A` below and the `git checkout -- .` between cases
# would stage and then DISCARD in the real working copy.
scratch_clone "$ROOT" "$scratch/repo" || fatal \
  "could not clone this repository into $scratch/repo" \
  "read the scratch-clone diagnostic above; if it names GIT_*, run this through 'just script-check'"

# Then overlay the working tree's tracked files and stage them, so `git checkout -- .`
# restores to THEM between cases. What is under test has to be the guard as it is right now:
# against a clone of HEAD alone, an author repairing the guard would watch this check keep
# failing against the version they had just replaced.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$scratch/repo" || fatal \
  "could not overlay this working tree's files onto $scratch/repo" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
git -C "$scratch/repo" add -A || fatal \
  "could not stage the overlaid tree in $scratch/repo, so no case could be undone after it ran" \
  "check 'df -h' for free space, then rerun"

failures=0
GUARD_OUTPUT=""
GUARD_STATUS=0

# The real script, from inside the scratch tree, so what is under test is the guard rather
# than a copy of its logic.
run_guard() {
  GUARD_OUTPUT="$(cd "$scratch/repo" && bash scripts/check-live-lane.sh 2>&1)" \
    && GUARD_STATUS=0 || GUARD_STATUS=$?
}

reset_fixture() {
  git -C "$scratch/repo" checkout --quiet -- . || fatal \
    "could not restore $scratch/repo, so every case after this one would run against a tree still carrying the previous fixture" \
    "check 'df -h' for free space, then rerun"
}

report_guard_output() {
  printf '%s\n' "$GUARD_OUTPUT" | sed 's/^/    /' >&2
}

# Replace one literal substring of a file in the scratch tree. python3 rather than `sed -i`,
# whose in-place spelling differs between GNU and BSD and so would fail on the macOS runner.
substitute() {
  python3 - "$scratch/repo/$1" "$2" "$3" <<'PY' || fatal \
    "the helper that rewrites a file in the scratch tree did not finish, so that case was never put to the guard" \
    "read the diagnostic above; if it names text this repository no longer contains, update the case to the text that replaced it"
import pathlib
import sys

path, before, after = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
text = path.read_text(encoding="utf-8")
if before not in text:
    print(
        f"check-live-lane-enforced: {path} no longer contains the text this case rewrites,\n"
        f"check-live-lane-enforced: so the case would prove nothing:\n"
        f"    {before}",
        file=sys.stderr,
    )
    raise SystemExit(1)
path.write_text(text.replace(before, after, 1), encoding="utf-8")
PY
}

expect_refused() {
  local case_name="$1"
  shift
  if [ "$GUARD_STATUS" -eq 0 ]; then
    echo "check-live-lane-enforced: $case_name — the guard PASSED it. That arrangement is one" >&2
    echo "check-live-lane-enforced: where these tests can conclude without having run, and a" >&2
    echo "check-live-lane-enforced: conclusion branch protection accepts in place of success is" >&2
    echo "check-live-lane-enforced: how the credentialed lane stops being a check at all." >&2
    failures=$((failures + 1))
    return
  fi
  local term
  for term in "$@"; do
    # A here-string rather than a pipe into `grep -q`: under `pipefail` a quiet grep's early
    # exit SIGPIPEs its writer, which can invert the pipeline's status on a match.
    if ! grep -qF -- "$term" <<<"$GUARD_OUTPUT"; then
      echo "check-live-lane-enforced: $case_name — the guard refused, but its diagnostic never" >&2
      echo "check-live-lane-enforced: mentions '$term', so it does not say what to go and fix." >&2
      echo "check-live-lane-enforced: It said:" >&2
      report_guard_output
      failures=$((failures + 1))
      return
    fi
  done
}

expect_passed() {
  local case_name="$1"
  if [ "$GUARD_STATUS" -ne 0 ]; then
    echo "check-live-lane-enforced: $case_name — the guard REFUSED an arrangement this" >&2
    echo "check-live-lane-enforced: repository is entitled to. A guard that refuses everything" >&2
    echo "check-live-lane-enforced: satisfies every refusal case above and proves none of them." >&2
    echo "check-live-lane-enforced: It said:" >&2
    report_guard_output
    failures=$((failures + 1))
  fi
}

# 0. The control. Without it, a guard that refused every tree — including this one — would
#    satisfy every refusal case below and look like the strictest check in the repository.
run_guard
if [ "$GUARD_STATUS" -ne 0 ]; then
  echo "check-live-lane-enforced: the guard refuses this repository as it stands, so the cases" >&2
  echo "check-live-lane-enforced: below would prove nothing. Run 'bash scripts/check-live-lane.sh'" >&2
  echo "check-live-lane-enforced: and fix what it reports first. It said:" >&2
  report_guard_output
  exit 1
fi

# 1. The journey compiled out on the item. It runs exactly as often as an ignored one and
#    reports less about it: cargo names an ignored test and says nothing at all about one
#    that was never built.
substitute "$JOURNEY" '#[tokio::test]' '#[cfg(feature = "live")]
#[tokio::test]'
run_guard
expect_refused "the journey excluded by a cfg on the test itself" \
  "$JOURNEY" '#[cfg(feature = "live")]' "runs nowhere"
reset_fixture

# 2. The same thing one level up, which takes the whole file rather than one item and is
#    the shorter way to do it.
substitute "$JOURNEY" '//!' '#![cfg(never)]
//!'
run_guard
expect_refused "the whole journey file excluded by an inner cfg" \
  "$JOURNEY" '#![cfg(never)]'
reset_fixture

# 3. `#[ignore]` put back under a condition, which is neither of the two spellings a scan
#    for one or the other would catch.
substitute "$JOURNEY" '#[tokio::test]' '#[cfg_attr(windows, ignore)]
#[tokio::test]'
run_guard
expect_refused "the ignore attribute reintroduced through cfg_attr" \
  "$JOURNEY" '#[cfg_attr(windows, ignore)]'
reset_fixture

# 4. The plain `#[ignore]`, which is what kept this journey out of every target but the
#    separate lane back when there was one. Watched here rather than taken on trust.
substitute "$JOURNEY" '#[tokio::test]' '#[ignore]
#[tokio::test]'
run_guard
expect_refused "the journey marked ignored" "$JOURNEY" '`#[ignore]`'
reset_fixture

# 5. The demand set to the value that turns it off. This is the hole: the name is there,
#    spelled correctly, on the step that carries the credential — and the lane skips.
substitute "$WORKFLOW" 'ONETASKGRAPH_LIVE_REQUIRED: "1"' 'ONETASKGRAPH_LIVE_REQUIRED: "0"'
run_guard
expect_refused "the demand set to 0 on a credentialed step" \
  "$WORKFLOW" "GH_PROJECTS_TOKEN" "which is not the demand"
reset_fixture

# 6. The fork exception with its two branches swapped, so the run that is NOT a fork pull
#    request — every run the merge waits on — is the one that stops demanding a credential.
substitute "$WORKFLOW" \
  "ONETASKGRAPH_LIVE_REQUIRED: \${{ github.event.pull_request.head.repo.fork && '0' || '1' }}" \
  "ONETASKGRAPH_LIVE_REQUIRED: \${{ github.event.pull_request.head.repo.fork && '1' || '0' }}"
run_guard
expect_refused "the fork exception with its branches swapped" \
  "$WORKFLOW" "which is not the demand"
reset_fixture

# 7. A condition that is not the fork case at all but carries the word in it. A guard that
#    recognised the exception by that word would accept this, and it turns the demand off
#    for every run on the default branch.
substitute "$WORKFLOW" \
  "ONETASKGRAPH_LIVE_REQUIRED: \${{ github.event.pull_request.head.repo.fork && '0' || '1' }}" \
  "ONETASKGRAPH_LIVE_REQUIRED: \${{ github.ref != 'refs/heads/fork-lane' && '0' || '1' }}"
run_guard
expect_refused "a guard that merely contains the word fork" \
  "$WORKFLOW" "refs/heads/fork-lane" "which is not the demand"
reset_fixture

# 8. The fork exception with a value nothing reads. `onetaskgraph_live::required` refuses
#    anything but the two constants and unset, so a lane spelled this way fails on a fork
#    pull request for a reason that has nothing to do with what it was testing — and it is
#    the shape a guard checking only "not the demand" would wave through.
substitute "$WORKFLOW" \
  "ONETASKGRAPH_LIVE_REQUIRED: \${{ github.event.pull_request.head.repo.fork && '0' || '1' }}" \
  "ONETASKGRAPH_LIVE_REQUIRED: \${{ github.event.pull_request.head.repo.fork && '2' || '1' }}"
run_guard
expect_refused "the fork exception yielding a value the parser refuses" \
  "$WORKFLOW" "which is not the demand"
reset_fixture

# 9. The demand dropped from a credentialed step entirely, which is the older half of the
#    same assertion and has never been watched fail either.
substitute "$WORKFLOW" '          ONETASKGRAPH_LIVE_REQUIRED: "1"
' ''
run_guard
expect_refused "the demand dropped from a credentialed step" \
  "$WORKFLOW" "does not set ONETASKGRAPH_LIVE_REQUIRED"
reset_fixture

# 10. The value renamed out of the live crate. The guard reads what the demand has to BE
#    from `onetaskgraph_live::DEMANDED` rather than restating it, so it has to say so
#    rather than fall back on a value of its own.
substitute "$LIVE_CRATE" 'pub const DEMANDED: &str' 'pub const REQUIRED_VALUE: &str'
run_guard
expect_refused "the demanded value renamed out of the crate that decides it" \
  "$LIVE_CRATE" "no longer declares DEMANDED"
reset_fixture

# 11. Its pair, the off value, which the same read gets from the same crate. Two constants
#     through one helper, so this is the case that says the helper is reached for both
#     rather than for the one that happened to be written first.
substitute "$LIVE_CRATE" 'pub const NOT_DEMANDED: &str' 'pub const UNDEMANDED: &str'
run_guard
expect_refused "the off value renamed out of the crate that decides it" \
  "$LIVE_CRATE" "no longer declares NOT_DEMANDED"
reset_fixture

# 12. The other half of that: the two really are tied. A crate demanding a different value
#     makes the workflow's `1` wrong, which is what a single source with a drift gate means
#     and what a restated constant could never do.
substitute "$LIVE_CRATE" 'pub const DEMANDED: &str = "1";' 'pub const DEMANDED: &str = "yes";'
run_guard
expect_refused "the crate and the workflow disagreeing about the demand" \
  "$WORKFLOW" "which is not the demand" "It has to be yes"
reset_fixture

# 13. The fork exception itself, which the guard must ACCEPT. GitHub hands a fork pull
#     request no secrets at all, so demanding a credential there would fail every outside
#     contribution for something its author cannot supply — and without this case, a guard
#     that refused every arrangement would have satisfied all twelve cases above.
substitute "$WORKFLOW" 'ONETASKGRAPH_LIVE_REQUIRED: "1"' \
  "ONETASKGRAPH_LIVE_REQUIRED: \${{ github.event.pull_request.head.repo.fork && '0' || '1' }}"
run_guard
expect_passed "the fork exception, spelled whole"
reset_fixture

if [ "$failures" -ne 0 ]; then
  echo "check-live-lane-enforced: $failures case(s) above did not go the way they must." >&2
  echo "check-live-lane-enforced: next: fix scripts/check-live-lane.sh so it refuses each" >&2
  echo "check-live-lane-enforced: arrangement named above and names the file and the value in" >&2
  echo "check-live-lane-enforced: its diagnostic. AGENTS.md records why each of them matters." >&2
  exit 1
fi
