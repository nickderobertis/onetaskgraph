#!/usr/bin/env bash
# Drive scripts/macos-lane-selection.sh over real git states and a stand-in GitHub, and
# hold .github/workflows/ci.yml to consulting it the way it says it does.
#
# A push to main used to schedule the macOS lanes again over a tree the pull request's own
# required check had just proven on the same scarce runners (#1991). The decision that now
# stops that is one script, and every answer it gives is a claim about which runs may be
# skipped — so each is watched here: `proven` for the squash of a merged pull request whose
# head has this tree and whose macOS checks passed on their most recent run, and `run` for
# every way that can fail to hold, plus every way the question cannot be asked at all. The
# GitHub API is the stand-in; the repository, the refs and the trees are real.
#
# Then the workflow. A hosted run cannot be exercised here, so what stands for it is the
# document: the decision job runs only on a push and hands the matrices its answer, both
# matrices fall back to the full platform list when it did not run — which is every pull
# request, so the required check names are unchanged — and neither is skipped when it
# fails. The step that turns the answer into a matrix is read out of the workflow and run
# over both answers, so what `fromJson` will be handed is proven to parse.
set -euo pipefail

fatal() {
  echo "check-macos-lane-selection: $1" >&2
  echo "check-macos-lane-selection: next: $2" >&2
  exit 2
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just check' does"
readonly ROOT
readonly DECISION="scripts/macos-lane-selection.sh"
readonly WORKFLOW=".github/workflows/ci.yml"

# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi
scratch_clone_strip_git_env

for tool in git python3; do
  command -v "$tool" >/dev/null 2>&1 || fatal "$tool is not on PATH" "install $tool, then rerun"
done

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

failures=0
report() {
  failures=$((failures + 1))
  echo "check-macos-lane-selection: $1" >&2
}
quote() { printf '%s\n' "$1" | sed 's/^/    /' >&2; }

# The repository: a base on main, a pull request head that changes one file, the squash
# of that head onto the base (the same tree, one new commit — what a squash merge of an
# up-to-date branch produces), and a squash whose tree differs (a branch merged behind the
# base). The origin is a bare repository holding refs/pull/7/head, as GitHub keeps it.
readonly ORIGIN="$scratch/origin.git"
readonly CLONE="$scratch/clone"
git init --quiet --bare "$ORIGIN" || fatal "could not create the bare origin" "check that git works and rerun"
git init --quiet "$CLONE" || fatal "could not create the clone" "check that git works and rerun"
g() { git -C "$CLONE" -c user.name=check -c user.email=check@example.invalid "$@"; }
g checkout --quiet -b main
# The decision sits where the workflow's step runs it from, so the step is driven as written.
mkdir -p "$CLONE/scripts" && cp "$ROOT/$DECISION" "$CLONE/$DECISION"
echo base > "$CLONE/file"
g add -A && g commit --quiet -m "base"
BASE="$(g rev-parse HEAD)"
g checkout --quiet -b feature
echo changed > "$CLONE/file"
g commit --quiet -am "feat: change the file"
HEAD_SHA="$(g rev-parse HEAD)"
g checkout --quiet main
SQUASH="$(g commit-tree "$HEAD_SHA^{tree}" -p "$BASE" -m "feat: change the file (#7)")"
echo elsewhere > "$CLONE/other"
g add other && g commit --quiet -m "another change on main"
DIVERGENT_TREE_COMMIT="$(g rev-parse HEAD)"
DIVERGENT="$(g commit-tree "$DIVERGENT_TREE_COMMIT^{tree}" -p "$BASE" -m "feat: change the file (#7)")"
g reset --quiet --hard "$SQUASH"
g remote add origin "$ORIGIN"
g push --quiet origin main "feature:refs/pull/7/head" || fatal "could not populate the bare origin" "check that git works and rerun"
g branch --quiet -D feature
readonly BASE HEAD_SHA SQUASH DIVERGENT
readonly SLUG="octo/repo"
readonly NAMES=("check (macos-latest)" "install path (macos-latest)")

# The stand-in `gh`: `gh api <path>` answers from a fixture file named after the path.
readonly GH_BIN="$scratch/gh-bin"
readonly FIXTURES="$scratch/fixtures"
mkdir -p "$GH_BIN" "$FIXTURES" || fatal "could not create the stand-in directories" "check the permissions of \$TMPDIR, then rerun"
cat > "$GH_BIN/gh" <<'GH'
#!/usr/bin/env bash
[ "${GH_STANDIN_FAIL:-}" != yes ] || { echo "gh stand-in: HTTP 403: API rate limit exceeded" >&2; exit 1; }
[ "${1:-}" = api ] || { echo "gh stand-in: only 'api' is served, not '${1:-}'" >&2; exit 2; }
fixture="$GH_STANDIN_FIXTURES/$(printf '%s' "$2" | tr '/?=' '___')"
[ -f "$fixture" ] || { echo "gh stand-in: HTTP 404: no fixture for $2" >&2; exit 1; }
cat "$fixture"
GH
chmod +x "$GH_BIN/gh"
export GH_STANDIN_FIXTURES="$FIXTURES"

# fixture <commit> <pulls json> [<head> <check-runs json>]
fixture() {
  rm -f "$FIXTURES"/*
  printf '%s' "$2" > "$FIXTURES/repos_${SLUG//\//_}_commits_$1_pulls"
  [ $# -lt 4 ] || printf '%s' "$4" > "$FIXTURES/repos_${SLUG//\//_}_commits_$3_check-runs_per_page_100"
}
pull() { printf '[{"number": 7, "merged_at": %s, "merge_commit_sha": "%s", "head": {"sha": "%s"}}]' "$1" "$2" "$3"; }
runs() { printf '{"check_runs": [%s]}' "$1"; }
run_json() { printf '{"id": %s, "name": "%s", "conclusion": %s}' "$1" "$2" "$3"; }
BOTH_PASSED="$(run_json 1 "check (macos-latest)" '"success"'),$(run_json 2 "install path (macos-latest)" '"success"'),$(run_json 3 "check (ubuntu-latest)" '"failure"')"

ANSWER=""
REASON=""
decide() {
  local commit="$1"
  shift
  ANSWER="$(cd "$CLONE" && PATH="$GH_BIN:$PATH" bash "$ROOT/$DECISION" "$SLUG" "$commit" "$@" 2>"$scratch/reason")" || ANSWER="<exit $?>"
  REASON="$(cat "$scratch/reason")"
}
expect() {
  local expected="$1" case_name="$2" reason_names="${3:-}"
  if [ "$ANSWER" != "$expected" ]; then
    report "$case_name answered '$ANSWER', expected '$expected'. It said:"
    quote "$REASON"
  elif [ -n "$reason_names" ] && ! grep -qF -- "$reason_names" <<<"$REASON"; then
    report "$case_name answered '$expected' without saying '$reason_names'. It said:"
    quote "$REASON"
  fi
}

# 1. Proven: merged as this commit, same tree, both macOS checks passed most recently.
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" "$(runs "$BOTH_PASSED")"
decide "$SQUASH" "${NAMES[@]}"
expect proven "the squash of a merged pull request whose macOS checks passed" "#7"

# 2. No pull request was merged as this commit.
fixture "$SQUASH" "[]"
decide "$SQUASH" "${NAMES[@]}"
expect run "a commit no pull request was merged as" "no pull request was merged as"

# 3. A pull request is listed, but was merged as some other commit.
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$DIVERGENT" "$HEAD_SHA")" "$HEAD_SHA" "$(runs "$BOTH_PASSED")"
decide "$SQUASH" "${NAMES[@]}"
expect run "a pull request merged as a different commit" "no pull request was merged as"

# 4. A pull request that names this commit but is not merged.
fixture "$SQUASH" "$(pull null "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" "$(runs "$BOTH_PASSED")"
decide "$SQUASH" "${NAMES[@]}"
expect run "an unmerged pull request" "no pull request was merged as"

# 4b. A merged pull request whose document names no usable head.
fixture "$SQUASH" "$(printf '[{"number": 7, "merged_at": "2026-09-19T00:00:00Z", "merge_commit_sha": "%s", "head": "not an object"}]' "$SQUASH")"
decide "$SQUASH" "${NAMES[@]}"
expect run "a pull request document with no usable head" "names no usable number and head"

# 5. The squash's tree is not the head's — the branch was merged behind the base.
fixture "$DIVERGENT" "$(pull '"2026-09-19T00:00:00Z"' "$DIVERGENT" "$HEAD_SHA")" "$HEAD_SHA" "$(runs "$BOTH_PASSED")"
decide "$DIVERGENT" "${NAMES[@]}"
expect run "a squash whose tree differs from the pull request head's" "is not the tree of pull request #7"

# 6. The pull request reports a head that refs/pull/7/head does not name.
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$BASE")" "$BASE" "$(runs "$BOTH_PASSED")"
decide "$SQUASH" "${NAMES[@]}"
expect run "a pull request head the origin's ref does not name" "not the head"

# 7. One macOS check failed; one is missing altogether.
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" \
  "$(runs "$(run_json 1 "check (macos-latest)" '"failure"'),$(run_json 2 "install path (macos-latest)" '"success"')")"
decide "$SQUASH" "${NAMES[@]}"
expect run "a failed macOS check" "concluded 'failure'"
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" \
  "$(runs "$(run_json 1 "check (macos-latest)" '"success"')")"
decide "$SQUASH" "${NAMES[@]}"
expect run "a macOS check that never ran" "no check run named 'install path (macos-latest)'"

# 8. The most recent run is the verdict: a re-run that failed after a pass is a failure,
#    and a re-run that passed after a failure is a pass.
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" \
  "$(runs "$(run_json 1 "check (macos-latest)" '"success"'),$(run_json 2 "install path (macos-latest)" '"success"'),$(run_json 9 "check (macos-latest)" '"failure"')")"
decide "$SQUASH" "${NAMES[@]}"
expect run "a re-run that failed after an earlier pass" "concluded 'failure'"
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" \
  "$(runs "$(run_json 1 "check (macos-latest)" '"failure"'),$(run_json 2 "install path (macos-latest)" '"success"'),$(run_json 9 "check (macos-latest)" '"success"')")"
decide "$SQUASH" "${NAMES[@]}"
expect proven "a re-run that passed after an earlier failure"

# 8b. Entries of the check-run array that are not check runs — a bare string, an object
#     with no numeric id — are passed over rather than ordered.
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" \
  "$(runs "\"not a run\",{\"name\": \"check (macos-latest)\", \"conclusion\": \"failure\"},$BOTH_PASSED")"
decide "$SQUASH" "${NAMES[@]}"
expect proven "a check-run array carrying entries that are not check runs"

# 9. A check still running has no conclusion yet.
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" \
  "$(runs "$(run_json 1 "check (macos-latest)" null),$(run_json 2 "install path (macos-latest)" '"success"')")"
decide "$SQUASH" "${NAMES[@]}"
expect run "a macOS check still in progress" "concluded None"

# 10. The question cannot be asked: GitHub refuses, answers something other than JSON, or
#     there is no gh at all. Each is `run`, with the cause.
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" "$(runs "$BOTH_PASSED")"
ANSWER="$(cd "$CLONE" && PATH="$GH_BIN:$PATH" GH_STANDIN_FAIL=yes bash "$ROOT/$DECISION" "$SLUG" "$SQUASH" "${NAMES[@]}" 2>"$scratch/reason")" || ANSWER="<exit $?>"
REASON="$(cat "$scratch/reason")"
expect run "GitHub refusing the request" "rate limit"
fixture "$SQUASH" "<html>not json</html>"
decide "$SQUASH" "${NAMES[@]}"
expect run "an answer that is not JSON" "not JSON"
fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" "$(runs "$BOTH_PASSED")"
readonly NO_GH="$scratch/no-gh"
mkdir -p "$NO_GH"
for tool in bash git python3 env sed grep cat tr; do
  resolved="$(command -v "$tool" 2>/dev/null)" || continue
  printf '#!%s\nexec "%s" "$@"\n' "$(command -v bash)" "$resolved" > "$NO_GH/$tool" && chmod +x "$NO_GH/$tool"
done
ANSWER="$(cd "$CLONE" && PATH="$NO_GH" bash "$ROOT/$DECISION" "$SLUG" "$SQUASH" "${NAMES[@]}" 2>"$scratch/reason")" || ANSWER="<exit $?>"
REASON="$(cat "$scratch/reason")"
expect run "no gh on PATH" "gh is not on PATH"

# 11. Usage: a call this cannot understand is refused rather than answered.
for arguments in "" "$SLUG" "$SLUG $SQUASH" "not-a-repo $SQUASH check" "$SLUG abc check"; do
  # shellcheck disable=SC2086 # the cases are whitespace-separated argument lists on purpose
  status=0; (cd "$CLONE" && PATH="$GH_BIN:$PATH" bash "$ROOT/$DECISION" $arguments) >/dev/null 2>&1 || status=$?
  [ "$status" -eq 64 ] || report "'$DECISION $arguments' exited $status rather than 64 (EX_USAGE)"
done

# 12. The workflow consults the decision the way it says it does. Read rather than trusted:
#     the shape is what makes every pull request's required names unchanged and every push
#     schedule its lanes when the decision cannot answer.
if ! (cd "$ROOT" && python3 - "$WORKFLOW" "$DECISION" "${NAMES[@]}" <<'PY'
import json
import re
import sys
from pathlib import Path

workflow_path, decision = sys.argv[1], sys.argv[2]
names = set(sys.argv[3:])
workflow = Path(workflow_path).read_text(encoding="utf-8")
problems = []


def job(name):
    match = re.search(rf"(?ms)^  {re.escape(name)}:\n(.*?)(?=^  [a-z-]+:\n|\Z)", workflow)
    if not match:
        problems.append(f"{workflow_path} has no job `{name}`")
        return ""
    return match.group(1)


def header(text):
    """A job's own keys, before its steps."""
    return text.split("\n    steps:", 1)[0]


FULL = '["ubuntu-latest","macos-latest","windows-latest"]'
selector = job("macos-lanes")
if selector:
    head = header(selector)
    if not re.search(r"(?m)^    if: github\.event_name == 'push'\s*$", head):
        problems.append("`macos-lanes` must run only on a push: on a pull request it is skipped, its output is empty, and the matrices fall back to every platform")
    if not re.search(r"(?m)^      os: \$\{\{ steps\.select\.outputs\.os \}\}\s*$", head):
        problems.append("`macos-lanes` must hand its `os` output up from the `select` step")
    if decision not in selector:
        problems.append(f"`macos-lanes` never runs {decision}, the one implementation of the decision")
    passed = set(re.findall(r'"((?:check|install path) \(macos-latest\))"', selector))
    if passed != names:
        problems.append(f"`macos-lanes` asks the decision about {sorted(passed)}, but the macOS lanes are {sorted(names)}")

matrix_jobs = {}
for name in ("install-path", "check"):
    text = job(name)
    if not text:
        continue
    head = header(text)
    matrix_jobs[name] = head
    if not re.search(r"(?m)^    needs: \[macos-lanes\]\s*$", head):
        problems.append(f"`{name}` must `needs: [macos-lanes]`, or its matrix cannot read the decision")
    if not re.search(r"(?m)^    if: \$\{\{ !cancelled\(\) \}\}\s*$", head):
        problems.append(f"`{name}` must carry `if: ${{{{ !cancelled() }}}}`: without it a skipped or failed `macos-lanes` skips this job — on every pull request, where the decision does not run")
    expected = f"        os: ${{{{ fromJson(needs.macos-lanes.outputs.os || '{FULL}') }}}}"
    if expected not in head:
        problems.append(f"`{name}` must take its matrix from the decision with the full list as the fallback:\n      {expected.strip()}")
    if "os: [" in head:
        problems.append(f"`{name}` still carries a literal os list beside the decision")

# The required names a pull request reports: unchanged, and derived from the same
# `name:` the lanes are asked about.
for name, expected in (("check", "check (${{ matrix.os }})"), ("install-path", "install path (${{ matrix.os }})")):
    head = matrix_jobs.get(name, "")
    if head and not re.search(rf"(?m)^    name: {re.escape(expected)}\s*$", head):
        problems.append(f"`{name}` must keep `name: {expected}`, which is what the required checks are called")
for name in names:
    lane = name.replace(" (macos-latest)", "")
    if not any(re.search(rf"(?m)^    name: {re.escape(lane)} \(\$\{{\{{ matrix\.os \}}\}}\)\s*$", head) for head in matrix_jobs.values()):
        problems.append(f"the decision is asked about {name!r}, but no matrix job is named `{lane} (${{{{ matrix.os }}}})`")

if problems:
    print(f"check-macos-lane-selection: {workflow_path} does not consult the decision the way it must:", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    raise SystemExit(1)
PY
); then
  failures=$((failures + 1))
fi

# 13. The step that turns the answer into a matrix, read out of the workflow and run over
#     both answers: what `fromJson` is handed has to parse, and has to be the full list
#     whenever the answer is anything but `proven`.
if ! (cd "$ROOT" && python3 - "$WORKFLOW" <<'PY'
import re
import sys
from pathlib import Path

workflow = Path(sys.argv[1]).read_text(encoding="utf-8")
steps = re.split(r"(?m)^      - (?=name:|uses:)", workflow)
declaring = [step for step in steps if re.search(r"(?m)^\s*id:\s*select\s*$", step)]
if len(declaring) != 1:
    print(f"check-macos-lane-selection: expected exactly one workflow step with `id: select`, found {len(declaring)}", file=sys.stderr)
    raise SystemExit(1)
body = re.search(r"(?m)^(\s*)run:\s*\|\s*$", declaring[0])
if not body:
    print("check-macos-lane-selection: the `select` step has no `run: |` body to drive", file=sys.stderr)
    raise SystemExit(1)
# The block ends where the indentation falls back to the `run:` key's or less: the step is
# the last of its job, so what follows it in this chunk is the next job.
key_indent = len(body.group(1))
lines = []
for line in declaring[0][body.end():].splitlines()[1:]:
    if line.strip() and len(line) - len(line.lstrip()) <= key_indent:
        break
    lines.append(line)
margin = min((len(line) - len(line.lstrip()) for line in lines if line.strip()), default=0)
sys.stdout.reconfigure(newline="\n")
print("\n".join(line[margin:] for line in lines))
PY
) > "$scratch/select-step.sh" 2>"$scratch/select-step-error"; then
  report "the workflow's select step could not be read: $(cat "$scratch/select-step-error")"
else
  drive_select() {
    local case_name="$1" commit="$2" expected="$3" derived
    : > "$scratch/github-output"
    if ! (cd "$CLONE" && PATH="$GH_BIN:$PATH" GITHUB_OUTPUT="$scratch/github-output" REPOSITORY="$SLUG" COMMIT="$commit" \
      bash "$scratch/select-step.sh") > "$scratch/select-output" 2>&1; then
      report "the workflow's select step failed for $case_name, which would skip every lane rather than schedule them all: $(cat "$scratch/select-output")"
      return
    fi
    derived="$(sed -n 's/^os=//p' "$scratch/github-output" | tr -d '\r')"
    if ! python3 -c 'import json, sys; value = json.loads(sys.argv[1]); assert isinstance(value, list) and value' "$derived" 2>/dev/null; then
      report "the workflow's select step wrote os='$derived' for $case_name, which is not the JSON list fromJson needs"
    elif [ "$derived" != "$expected" ]; then
      report "the workflow's select step wrote os=$derived for $case_name, expected $expected"
    fi
  }
  fixture "$SQUASH" "$(pull '"2026-09-19T00:00:00Z"' "$SQUASH" "$HEAD_SHA")" "$HEAD_SHA" "$(runs "$BOTH_PASSED")"
  drive_select "a proven commit" "$SQUASH" '["ubuntu-latest","windows-latest"]'
  fixture "$SQUASH" "[]"
  drive_select "a commit with no pull request" "$SQUASH" '["ubuntu-latest","macos-latest","windows-latest"]'
  fixture "$DIVERGENT" "$(pull '"2026-09-19T00:00:00Z"' "$DIVERGENT" "$HEAD_SHA")" "$HEAD_SHA" "$(runs "$BOTH_PASSED")"
  drive_select "a squash whose tree differs" "$DIVERGENT" '["ubuntu-latest","macos-latest","windows-latest"]'
  # The decision missing altogether — the step must still schedule everything.
  : > "$scratch/github-output"
  if (cd "$CLONE" && PATH="$GH_BIN:$PATH" GITHUB_OUTPUT="$scratch/github-output" REPOSITORY="$SLUG" COMMIT="not-a-commit" \
    bash "$scratch/select-step.sh") > "$scratch/select-output" 2>&1; then
    derived="$(sed -n 's/^os=//p' "$scratch/github-output" | tr -d '\r')"
    [ "$derived" = '["ubuntu-latest","macos-latest","windows-latest"]' ] || report "the select step wrote os=$derived when the decision refused the call, expected the full list"
  else
    report "the select step failed when the decision refused the call; it has to schedule every lane instead: $(cat "$scratch/select-output")"
  fi
fi

if [ "$failures" -ne 0 ]; then
  echo "check-macos-lane-selection: $failures case(s) failed." >&2
  echo "check-macos-lane-selection: repair $DECISION or $WORKFLOW rather than these cases; a wrong" >&2
  echo "check-macos-lane-selection: 'proven' skips a required lane, and a wrong shape skips a whole matrix." >&2
  exit 1
fi
