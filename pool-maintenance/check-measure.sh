#!/usr/bin/env bash
# Drive pool-maintenance/measure.sh under this project's budgets.yaml, through onebudgetspec
# and the pinned cargo-prune-superseded, over a scratch Cargo workspace small enough to
# build in seconds — never over this repository's own target directory.
#
# The workspace is warm the way the release pull request's Ubuntu leg is: its builds are
# restored with old modification times, as a cache restore leaves them, and one crate is
# built under two feature sets (once for a build script, once for a library), so the pruner
# sees rival builds of one unit name. One pass over that state removes builds the gate still
# needs, which is how the measurement once refused its own release pull request. Four cases:
#   1. that warm directory measures a figure, inside the budget, with the timed pass removing
#      the alternative build and nothing else;
#   2. a timed pass that removes nothing is an error, not a figure;
#   3. a timed pass that removes an ordinary build is an error, not a figure;
#   4. a backlog that never settles is an error, not a figure.
# Cases 2 to 4 put a stand-in pruner first on PATH that runs the real one and then departs
# from it, so each refusal is reached by the state it exists for.
set -euo pipefail

fatal() {
  echo "check-measure: $1" >&2
  echo "check-measure: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as the pool-maintenance project's test target does"
readonly ROOT

# The measurement runs only on the release pull request's Ubuntu leg and on Linux pool
# hosts, and touch, find and the pruner's modification times are what it is about.
if [ "$(uname -s)" != Linux ]; then
  echo "check-measure: skipped on $(uname -s): pool-maintenance/measure.sh runs only on Linux (the release pull request's Ubuntu leg and the pool hosts)."
  exit 0
fi

real_pruner="$(command -v cargo-prune-superseded)" || fatal \
  "cargo-prune-superseded is not on PATH" \
  "run 'just pool-maintenance-tool', then rerun"
readonly real_pruner

scratch="$(mktemp -d)" || fatal \
  "could not create a scratch directory" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# Every build the fixture makes goes into its own target directory, whatever the caller set.
unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR ONEBUDGETSPEC_RESULT

# Older than the pruner's 2-day window, and newer than the date measure.sh backdates the
# alternative build to, as a cache restored from an earlier run is.
readonly RESTORED_AT="2020-01-01T00:00:00"

# A warm workspace at $1. `c` under two feature sets gives the pruner rival builds of one
# unit name, and dating everything but the workspace member `b`'s source $RESTORED_AT is
# what a cache restore leaves: together they are the state one backlog pass cannot settle.
make_fixture() {
  local fx="$1"
  mkdir -p "$fx/.cargo" "$fx/pool-maintenance" "$fx/crates/a/src" "$fx/crates/b/src" \
    "$fx/vendor/c/src"
  cp "$ROOT/justfile" "$fx/justfile"
  cp "$ROOT/pool-maintenance/measure.sh" "$ROOT/pool-maintenance/budgets.yaml" \
    "$fx/pool-maintenance/"
  printf '[build]\ntarget-dir = "target"\n\n[profile.dev]\ndebug = 0\n' >"$fx/.cargo/config.toml"
  printf '[workspace]\nresolver = "2"\nmembers = ["crates/a", "crates/b"]\n' >"$fx/Cargo.toml"
  cat >"$fx/vendor/c/Cargo.toml" <<'EOF'
[package]
name = "c"
version = "0.1.0"
edition = "2021"

[features]
extra = []
EOF
  echo 'pub fn c() -> u32 { if cfg!(feature = "extra") { 2 } else { 1 } }' >"$fx/vendor/c/src/lib.rs"
  cat >"$fx/crates/a/Cargo.toml" <<'EOF'
[package]
name = "a"
version = "0.1.0"
edition = "2021"

[dependencies]
c = { path = "../../vendor/c", features = ["extra"] }

[build-dependencies]
c = { path = "../../vendor/c" }
EOF
  echo 'fn main() { println!("cargo:rustc-env=A_BUILT={}", c::c()); }' >"$fx/crates/a/build.rs"
  cat >"$fx/crates/a/src/lib.rs" <<'EOF'
pub fn a() -> u32 {
    env!("A_BUILT").parse::<u32>().unwrap() + c::c()
}

#[test]
fn built() {
    assert_eq!(a(), 3);
}
EOF
  cat >"$fx/crates/b/Cargo.toml" <<'EOF'
[package]
name = "b"
version = "0.1.0"
edition = "2021"

[dependencies]
a = { path = "../a" }
EOF
  cat >"$fx/crates/b/src/lib.rs" <<'EOF'
pub fn b() -> u32 {
    a::a()
}

#[test]
fn built() {
    assert_eq!(b(), 3);
}
EOF
  (cd "$fx" && cargo generate-lockfile --offline --quiet \
    && cargo test --quiet --workspace --all-features --locked --no-run) >"$scratch/build.log" 2>&1 \
    || fatal "the fixture workspace at $fx did not build: $(cat "$scratch/build.log")" \
      "build it by hand with 'cargo test --workspace --no-run' there and fix what it names"
  find "$fx" -path "$fx/pool-maintenance" -prune -o -exec touch -h -d "$RESTORED_AT" {} +
  touch "$fx/crates/b/src/lib.rs"
}

# A stand-in pruner first on PATH: it runs the real one, then departs from it as $1 says.
#   timed-removes-nothing   the pass that faces the alternative build only reports.
#   timed-removes-ordinary  that pass also removes the ordinary build of `b`.
#   backlog-never-settles   every other pass also removes the ordinary build of `b`.
# The pass that faces the alternative build is the one whose target holds a build dated
# before 2002, where measure.sh's header puts the alternative build.
install_stand_in() {
  mkdir -p "$scratch/bin"
  cat >"$scratch/bin/cargo-prune-superseded" <<EOF
#!/usr/bin/env bash
set -euo pipefail
mode="$1"
real="$real_pruner"
EOF
  cat >>"$scratch/bin/cargo-prune-superseded" <<'EOF'
marker="$(mktemp)"
touch -d "2002-01-01T00:00:00" "$marker"
alternative_present=""
[ -n "$(find target -path '*/.fingerprint/*' ! -newer "$marker" -print -quit)" ] \
  && alternative_present=1
rm -f "$marker"
remove_ordinary_b() {
  find target/debug/deps -maxdepth 1 -name 'libb-*.rlib' -exec rm -f {} +
}
case "$mode" in
  timed-removes-nothing)
    if [ -n "$alternative_present" ]; then exec "$real" --dry-run "$@"; fi
    exec "$real" "$@" ;;
  timed-removes-ordinary)
    "$real" "$@"
    if [ -n "$alternative_present" ]; then remove_ordinary_b; fi ;;
  backlog-never-settles)
    "$real" "$@"
    if [ -z "$alternative_present" ]; then remove_ordinary_b; fi ;;
esac
EOF
  chmod +x "$scratch/bin/cargo-prune-superseded"
}

# As `just pool-maintenance-budget` runs it; any further arguments go to onebudgetspec.
measure() {
  local fixture="$1" status=0
  shift
  (cd "$ROOT" && bun run onebudgetspec check "$fixture/pool-maintenance/budgets.yaml" --label host "$@") \
    >"$scratch/out" 2>"$scratch/err" || status=$?
  cat "$scratch/err" >>"$scratch/out"
  echo "$status"
}

# 1. The warm directory measures a figure. Its evidence lists are copied under this
# repository's own target/ when it does not, because the scratch tree goes with the trap.
readonly KEPT="$ROOT/target/pool-maintenance-check-evidence"
warm_fatal() {
  rm -rf "$KEPT"
  mkdir -p "$(dirname "$KEPT")"
  cp -R "$scratch/warm/target/pool-maintenance-evidence" "$KEPT" 2>/dev/null || true
  fatal "$1" "read the evidence lists measure.sh wrote, kept in $KEPT, then $2"
}
make_fixture "$scratch/warm"
status="$(measure "$scratch/warm" --json)"
[ "$status" -eq 0 ] || warm_fatal \
  "the measurement over a warm, cache-restored workspace exited $status, not 0: $(cat "$scratch/out")" \
  "fix measure.sh's setup so the timed pass faces one run's superseded builds and nothing else"
evidence="$scratch/warm/target/pool-maintenance-evidence"
# The figure is the timed pass's own report, within its budget, and its detail carries one
# untimed figure per backlog pass, so the settling is visible beside the figure it excludes.
backlog_passes="$(find "$evidence" -name 'backlog-removed-*.txt' | wc -l | tr -d ' ')"
report_problem="$(python3 - "$scratch/out" "$backlog_passes" <<'PY'
import json
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
result = json.loads(text[: text.rindex("}") + 1])["results"][0]
detail = result["detail"] or ""
untimed = re.search(r"untimed backlog passes before it, not counted: (.*) s$", detail)
removed = re.search(r"builds removed: (\d+);", detail)
if result["verdict"] != "within" or not isinstance(result["actual"], float):
    print(f"verdict {result['verdict']!r}, actual {result['actual']!r}")
elif not removed or int(removed.group(1)) == 0:
    print(f"the timed pass's own report is not the figure: {detail!r}")
elif not untimed or len(untimed.group(1).split(", ")) != int(sys.argv[2]):
    print(f"the detail does not carry {sys.argv[2]} untimed figures: {detail!r}")
PY
)" || report_problem="the report could not be read: $(cat "$scratch/out")"
[ -z "$report_problem" ] || warm_fatal "onebudgetspec's result is not the figure measure.sh owes: $report_problem" \
  "check that measure.sh still leaves the pruner's own report as the figure and appends each untimed pass's seconds"
# The fixture faced what it exists for: the first untimed pass removed builds, and so did
# one after it, which a single pass would have left for the timed pass to remove.
if ! { [ -s "$evidence/backlog-removed-1.txt" ] && [ -s "$evidence/backlog-removed-2.txt" ]; }; then
  warm_fatal "the fixture's backlog settled in one pass, so it no longer reproduces a warm, cache-restored directory" \
    "restore the fixture's rival builds of one unit (crate c under two feature sets) and its restored dates"
fi
if ! { [ -s "$evidence/removed.txt" ] && [ ! -s "$evidence/removed-not-alternative.txt" ] \
  && [ ! -s "$evidence/ordinary-missing.txt" ]; }; then
  warm_fatal "the timed pass did not remove exactly the alternative build" \
    "fix measure.sh's setup so the timed pass faces one run's superseded builds and nothing else"
fi

# 2-4. Each departure is an error naming its cause, never a figure.
for case in \
  "timed-removes-nothing|the timed pass removed nothing" \
  "timed-removes-ordinary|the timed pass removed entries the alternative build did not create" \
  "backlog-never-settles|never reached a steady state"; do
  mode="${case%%|*}"
  expected="${case#*|}"
  install_stand_in "$mode"
  make_fixture "$scratch/$mode"
  status="$(PATH="$scratch/bin:$PATH" measure "$scratch/$mode")"
  [ "$status" -ne 0 ] || fatal \
    "the measurement exited 0 when the pruner's pass was '$mode': $(cat "$scratch/out")" \
    "restore the refusal in measure.sh that reports '$expected'"
  if ! { grep -q "budget onetaskgraph-pool-maintenance-seconds: error" "$scratch/out" \
    && grep -q "$expected" "$scratch/out"; }; then
    fatal "the measurement did not refuse '$mode' as an error naming '$expected': $(cat "$scratch/out")" \
      "restore the refusal in measure.sh that reports '$expected'"
  fi
done
