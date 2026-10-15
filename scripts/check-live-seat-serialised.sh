#!/usr/bin/env bash
# Prove that no two Nx targets holding the same one-at-a-time live seat can run at once.
#
# A lane opening `Exclusivity::OneAtATime` takes a seat file, and a second session asking
# for it is DECLINED, which fails the required check. Two crates' tests take the Linear seat
# — the plugin's own live journey and the e2e suite's asset journey — and with both selected
# in one `just check`, Nx ran their `test` targets side by side and the second to start was
# declined: a red check from no defect in the code. What keeps them apart is `parallelism:
# false` on all but one of those targets, which the pinned Nx honours by running such a
# target alone.
#
# Reasoning about that flag proves nothing, so this drives the real Nx. The seat-holding
# crates are found by scanning their tests rather than listed, so a third that starts taking
# a seat is held to the rule unwatched by nobody. Each becomes a project of a scratch
# workspace whose `test` target carries the real target's `parallelism` and takes a stand-in
# seat — failing, as the real one declines, if it is already held. The control comes first:
# two such targets without the flag MUST collide, because a probe that could not see an
# overlap would report every arrangement serialised and look exactly like a pass.
set -euo pipefail

fatal() {
  echo "check-live-seat-serialised: $1" >&2
  echo "check-live-seat-serialised: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT
cd "$ROOT"

readonly NX="$ROOT/node_modules/.bin/nx"
[ -x "$NX" ] || [ -f "$NX" ] || fatal \
  "the pinned Nx is not installed at $NX" \
  "run 'just bootstrap', then rerun"

# Under target/, inside this checkout, so the scratch workspace resolves `nx` from this
# repository's own node_modules by ordinary module resolution — no link, which the Windows
# runner's bash would turn into a copy of the whole tree.
mkdir -p "$ROOT/target" || fatal \
  "could not create $ROOT/target for the scratch workspace" \
  "check the permissions of this checkout and 'df -h' for free space, then rerun"
scratch="$(mktemp -d "$ROOT/target/live-seat-serialised.XXXXXX")" || fatal \
  "could not create the scratch workspace under $ROOT/target" \
  "check the permissions of this checkout and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

export NX_DAEMON=false NX_NO_CLOUD=true NX_TUI=false NX_SKIP_NX_CACHE=true
export NX_CACHE_DIRECTORY="$scratch/nx-cache"
export NX_WORKSPACE_DATA_DIRECTORY="$scratch/nx-workspace-data"

# The seat-holding crates and each one's `test` target `parallelism`, one per line as
# `<project>\t<true|false>`. Python rather than a grep, because the answer is read out of
# JSON and a path is spelled with forward slashes whatever platform renders it.
holders="$(python3 - <<'PY' | tr -d '\r'
import json
import sys
from pathlib import Path

found = {}
for source in sorted(Path("crates").glob("*/tests/**/*.rs")):
    text = source.read_text(encoding="utf-8")
    if "Session::open" in text and "Exclusivity::OneAtATime" in text:
        found.setdefault(source.parts[1], source.as_posix())
for crate, source in sorted(found.items()):
    project_file = Path("crates") / crate / "project.json"
    try:
        project = json.loads(project_file.read_text(encoding="utf-8"))
        test = project["targets"]["test"]
    except (OSError, ValueError, KeyError) as problem:
        print(
            f"check-live-seat-serialised: {source} takes a one-at-a-time seat, but "
            f"{project_file.as_posix()} has no readable `test` target ({problem}).",
            file=sys.stderr,
        )
        sys.exit(1)
    flag = "false" if test.get("parallelism", True) is False else "true"
    print(f"{project['name']}\t{flag}")
PY
)" || fatal \
  "could not read which crates take a one-at-a-time live seat" \
  "fix the file the diagnostic above names, then rerun"

# Write one project of the scratch workspace whose `test` target takes the stand-in seat.
# `node` rather than `mkdir && sleep`: Nx runs a command through cmd on the Windows runner,
# which has no `sleep`.
write_project() {
  mkdir -p "$scratch/w/$1" && cat >"$scratch/w/$1/project.json" <<EOF
{"name": "$1", "targets": {"test": {"executor": "nx:run-commands", "cache": false, "parallelism": $2, "options": {"command": "node ../hold-seat.js $1", "cwd": "$1"}}}}
EOF
}

reset_workspace() {
  rm -rf "$scratch/w" && mkdir -p "$scratch/w" && printf '{}\n' >"$scratch/w/nx.json" \
    && printf '{"name": "live-seat-serialised", "private": true}\n' >"$scratch/w/package.json" \
    && cat >"$scratch/w/hold-seat.js" <<'JS'
// Take the seat as Seat::take does — refuse when it is held — hold it, release it.
const fs = require("fs");
const path = require("path");
const seat = path.join(__dirname, "seat");
const project = process.argv[2];
try {
  fs.mkdirSync(seat);
} catch (problem) {
  console.error(`SEAT HELD: ${project} started while another target held the seat`);
  process.exit(1);
}
fs.writeFileSync(path.join(__dirname, `ran-${project}`), "");
setTimeout(() => fs.rmdirSync(seat), 3000);
JS
}

RUN_OUTPUT=""
RUN_STATUS=0
run_nx() {
  RUN_OUTPUT="$(cd "$scratch/w" && "$NX" run-many --target test --parallel "$1" --skip-nx-cache 2>&1)" \
    && RUN_STATUS=0 || RUN_STATUS=$?
}

# 1. The control: two seat-holders without the flag, which Nx must start together.
reset_workspace && write_project control-a true && write_project control-b true || fatal \
  "could not write the control workspace in $scratch" \
  "check 'df -h' for free space, then rerun"
run_nx 3
if [ "$RUN_STATUS" -eq 0 ] || ! grep -qF "SEAT HELD" <<<"$RUN_OUTPUT"; then
  echo "check-live-seat-serialised: two targets without 'parallelism: false' did not collide" >&2
  echo "check-live-seat-serialised: on the stand-in seat, so this probe cannot see an overlap and" >&2
  echo "check-live-seat-serialised: the run below would prove nothing. Nx said:" >&2
  printf '%s\n' "$RUN_OUTPUT" | sed 's/^/    /' >&2
  fatal "the simulation did not take" \
    "check that '$NX run-many' runs tasks in parallel in this environment, then rerun"
fi

# 2. The repository's own seat-holders, each with its real target's `parallelism`.
reset_workspace || fatal "could not reset the scratch workspace in $scratch" \
  "check 'df -h' for free space, then rerun"
count=0
while IFS="$(printf '\t')" read -r project flag; do
  [ -n "$project" ] || continue
  write_project "$project" "$flag" || fatal \
    "could not write $project into the scratch workspace" \
    "check 'df -h' for free space, then rerun"
  count=$((count + 1))
done <<<"$holders"
[ "$count" -gt 0 ] || fatal \
  "no crate's tests open a one-at-a-time session, so this check is watching nothing" \
  "if the seat moved, point the scan at where Session::open with Exclusivity::OneAtATime now lives"
run_nx $((count + 1))
if [ "$RUN_STATUS" -ne 0 ]; then
  if grep -qF "SEAT HELD" <<<"$RUN_OUTPUT"; then
    echo "check-live-seat-serialised: two targets that take the same one-at-a-time live seat ran" >&2
    echo "check-live-seat-serialised: at once, so on the credentialed lane the second is declined" >&2
    echo "check-live-seat-serialised: and the required check fails. The seat-holders and their" >&2
    echo "check-live-seat-serialised: 'parallelism' are:" >&2
    printf '%s\n' "$holders" | sed 's/^/    /' >&2
    fatal "the seat-holding targets are not serialised" \
      "set \"parallelism\": false on the 'test' target of every crate above but one, in its project.json"
  fi
  printf '%s\n' "$RUN_OUTPUT" | sed 's/^/    /' >&2
  fatal "Nx failed over the scratch workspace for a reason other than the seat" \
    "read Nx's output above, then rerun"
fi
while IFS="$(printf '\t')" read -r project _; do
  [ -n "$project" ] || continue
  [ -f "$scratch/w/ran-$project" ] || fatal \
    "Nx reported success but $project never took the seat, so nothing was proven about it" \
    "read '$NX run-many' output for skipped tasks, then rerun"
done <<<"$holders"
