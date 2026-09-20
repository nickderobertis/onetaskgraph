#!/usr/bin/env bash
# Decide whether a push to the default branch needs its macOS lanes, or whether the same
# tree already passed them on the pull request it was squash-merged from.
#
# The account has one small macOS runner pool, shared by every push to main and every
# release, and a push to main started two macOS jobs over a tree the pull request's own
# required check had just proven on the same runners (#1991). This is the ONE implementation
# of the question, consulted by .github/workflows/ci.yml before it schedules the matrix:
#
#   scripts/macos-lane-selection.sh <owner/repo> <commit> <check name>...
#
# answers `proven` when ALL of the following hold, and `run` otherwise:
#
#   1. GitHub lists a pull request that was merged AS this commit — its merge_commit_sha is
#      the commit, which a squash merge makes the one commit the merge produced;
#   2. `refs/pull/<number>/head` on the origin still names the head that pull request
#      reports, and that head's TREE is byte-for-byte this commit's tree — what the lanes
#      proved is a tree, and a squash of an up-to-date branch reproduces it exactly, while
#      a branch merged behind the base does not;
#   3. for every check name given, the most recent check run on that head concluded
#      `success` — the most recent, because a re-run that failed after a pass is the
#      verdict that stands.
#
# **Any question it cannot answer is `run`**: a `gh` it cannot find, an API call GitHub
# refuses, a ref it cannot fetch, a tree it cannot read. It fails toward spending a runner,
# never toward skipping a lane, because that is the direction a wrong answer is recoverable
# in. The reason for each answer goes to stderr; the answer alone goes to stdout, held to
# "\n", because the caller compares a word against it.
#
# Exit codes: 0 with an answer; 64 (EX_USAGE) for a call this does not understand.
set -uo pipefail

usage() {
  echo "macos-lane-selection: $1" >&2
  echo "macos-lane-selection: next: call it as 'scripts/macos-lane-selection.sh <owner/repo> <commit> <check name>...'" >&2
  exit 64
}

# Print the reason, answer `run`, and stop.
run_because() {
  echo "macos-lane-selection: $1" >&2
  printf 'run\n'
  exit 0
}

[ $# -ge 3 ] || usage "this script takes a repository, a commit and at least one check name and received $# argument(s)"
repository="$1"
commit="$2"
shift 2
[[ $repository =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] || usage "'$repository' is not an owner/repo"
[[ $commit =~ ^[0-9a-f]{40}$ ]] || usage "'$commit' is not a full commit id"

for tool in gh git python3; do
  command -v "$tool" >/dev/null 2>&1 || run_because "$tool is not on PATH, so whether the lanes already ran cannot be asked"
done

# 1. The pull request merged as this commit.
pulls=""
pulls="$(gh api "repos/$repository/commits/$commit/pulls" 2>&1)" \
  || run_because "GitHub did not list the pull requests of $commit: $pulls"
merged=""
merged="$(printf '%s' "$pulls" | COMMIT="$commit" python3 -c '
import json, os, sys
commit = os.environ["COMMIT"]
try:
    pulls = json.load(sys.stdin)
except ValueError as problem:
    print(f"the pull-request answer is not JSON: {problem}")
    raise SystemExit(1)
for pull in pulls if isinstance(pulls, list) else []:
    if pull.get("merged_at") and pull.get("merge_commit_sha") == commit:
        print(pull["number"], pull["head"]["sha"])
        raise SystemExit(0)
print(f"no pull request was merged as {commit}")
raise SystemExit(1)
' 2>&1)" || run_because "$merged"
number="${merged%% *}"
head="${merged##* }"
[[ $number =~ ^[0-9]+$ ]] && [[ $head =~ ^[0-9a-f]{40}$ ]] \
  || run_because "the pull-request answer named no usable number and head ('$merged')"

# 2. The head that was proven is the tree that was pushed.
fetch_output=""
fetch_output="$(git fetch --no-tags --quiet origin "refs/pull/$number/head" 2>&1)" \
  || run_because "refs/pull/$number/head could not be fetched from origin: $fetch_output"
fetched="$(git rev-parse --verify --quiet FETCH_HEAD)" \
  || run_because "refs/pull/$number/head was fetched but names no commit"
[ "$fetched" = "$head" ] \
  || run_because "refs/pull/$number/head is $fetched, not the head $head the pull request reports"
pushed_tree="$(git rev-parse --verify --quiet "$commit^{tree}")" \
  || run_because "$commit is not in this checkout, so its tree cannot be compared"
proven_tree="$(git rev-parse --verify --quiet "$head^{tree}")" \
  || run_because "$head has no tree this checkout can read"
[ "$pushed_tree" = "$proven_tree" ] \
  || run_because "the tree of $commit ($pushed_tree) is not the tree of pull request #$number's head ($proven_tree), so what the lanes proved is not what was pushed"

# 3. Every named check passed on that head, on its most recent run.
runs=""
runs="$(gh api "repos/$repository/commits/$head/check-runs?per_page=100" 2>&1)" \
  || run_because "GitHub did not list the check runs of $head: $runs"
verdict=""
verdict="$(printf '%s' "$runs" | python3 -c '
import json, sys
names = sys.argv[1:]
try:
    answer = json.load(sys.stdin)
except ValueError as problem:
    print(f"the check-run answer is not JSON: {problem}")
    raise SystemExit(1)
latest = {}
for run in answer.get("check_runs", []) if isinstance(answer, dict) else []:
    name = run.get("name")
    if name in names and (name not in latest or run.get("id", 0) > latest[name].get("id", 0)):
        latest[name] = run
for name in names:
    run = latest.get(name)
    if run is None:
        print(f"no check run named {name!r} exists on that head")
        raise SystemExit(1)
    if run.get("conclusion") != "success":
        print("the most recent run of %r on that head concluded %r, not success" % (name, run.get("conclusion")))
        raise SystemExit(1)
print("every named check passed on its most recent run")
' "$@" 2>&1)" || run_because "$verdict"

echo "macos-lane-selection: pull request #$number was merged as $commit with the same tree, and $verdict; the macOS lanes are already proven" >&2
printf 'proven\n'
