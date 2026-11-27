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
#      the stand-in copies and nothing else;
#   2. a timed pass that removes nothing is an error, not a figure;
#   3. a timed pass that removes an ordinary build is an error, not a figure;
#   4. a backlog that never settles is an error, not a figure;
#   5. the helper's stand-in copies are byte for byte their builds, symlinks kept as links,
#      and a stand-in already present is left as it is while every other entry is copied.
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
# A configuration above every fixture that moves cargo's build directory out of it: each case
# then proves the fixture's own build-dir keeps its units where the pruner walks, rather than
# relying on a host that sets none.
mkdir -p "$scratch/.cargo"
printf '[build]\nbuild-dir = "%s/outside-build-dir"\n' "$scratch" >"$scratch/.cargo/config.toml"

# Every build the fixture makes goes into its own target directory, whatever the caller set —
# its intermediates (.fingerprint, deps, build, incremental) included, which cargo writes to
# its build directory: the pruner and measure.sh find units only under the target directory.
# And it builds with this repository's pinned toolchain, as measure.sh's real target is built,
# never a host's default: a cargo whose build directory has a newer layout (1.100's keeps a
# unit under build/<package>/<hash>/) leaves the pinned pruner no profile to find, which failed
# this check on a CI runner whose default was 1.100.0 while this repository pinned 1.97.1.
unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR CARGO_BUILD_BUILD_DIR RUSTUP_TOOLCHAIN ONEBUDGETSPEC_RESULT

# Older than the pruner's 2-day window, and newer than the date measure.sh backdates the
# stand-in copies to, as a cache restored from an earlier run is.
readonly RESTORED_AT="2020-01-01T00:00:00"

# A warm workspace at $1. `c` under two feature sets gives the pruner rival builds of one
# unit name, and dating everything but the workspace member `b`'s source $RESTORED_AT is
# what a cache restore leaves: together they are the state one backlog pass cannot settle.
make_fixture() {
  local fx="$1"
  mkdir -p "$fx/.cargo" "$fx/pool-maintenance" "$fx/crates/a/src" "$fx/crates/b/src" \
    "$fx/vendor/c/src"
  cp "$ROOT/justfile" "$ROOT/rust-toolchain.toml" "$fx/"
  cp "$ROOT/pool-maintenance/measure.sh" "$ROOT/pool-maintenance/ordinary_builds.py" \
    "$ROOT/pool-maintenance/budgets.yaml" \
    "$fx/pool-maintenance/"
  # build-dir beside target-dir, so no configuration above the fixture (a CARGO_HOME or a
  # directory over the scratch tree) can move its intermediates out of the target the pruner
  # walks.
  printf '[build]\ntarget-dir = "target"\nbuild-dir = "target"\n\n[profile.dev]\ndebug = 0\n' \
    >"$fx/.cargo/config.toml"
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
#   timed-removes-nothing   the pass that faces the stand-in copies only reports.
#   timed-removes-ordinary  that pass also removes the ordinary build of `b`.
#   backlog-never-settles   every other pass also removes the ordinary build of `b`.
# The pass that faces the stand-in copies is the one whose target holds a build dated
# before 2002, where measure.sh's header puts the stand-in copies.
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
# What the fixture's cargo was and where it built, printed rather than only kept, because a
# CI runner's target/ is not uploaded: a toolchain or configuration that moves the fixture's
# builds out of where the pruner looks otherwise fails here naming only the symptom.
warm_environment() {
  local fx="$scratch/warm" dir
  echo "check-measure: the fixture's environment, at $fx:"
  (cd "$fx" && cargo --version && rustc --version) 2>&1 | sed 's/^/  /'
  env | grep -E '^(CARGO|RUSTUP|RUSTC|RUSTFLAGS)' | LC_ALL=C sort | sed 's/^/  env /'
  (cd "$fx" && RUSTC_BOOTSTRAP=1 cargo -Zunstable-options config get build 2>&1) | sed 's/^/  config /'
  dir="$fx"
  while :; do
    for config in "$dir/.cargo/config.toml" "$dir/.cargo/config"; do
      [ -f "$config" ] && echo "  config file $config: $(tr '\n' ' ' <"$config")"
    done
    [ "$dir" = / ] && break
    dir="$(dirname "$dir")"
  done
  for config in "${CARGO_HOME:-$HOME/.cargo}/config.toml" "${CARGO_HOME:-$HOME/.cargo}/config"; do
    [ -f "$config" ] && echo "  config file $config: $(tr '\n' ' ' <"$config")"
  done
  echo "  target, two levels:"
  (cd "$fx" && find target -mindepth 1 -maxdepth 2 2>&1 | LC_ALL=C sort | sed 's/^/    /')
}
warm_fatal() {
  rm -rf "$KEPT"
  mkdir -p "$(dirname "$KEPT")"
  cp -R "$scratch/warm/target/pool-maintenance-evidence" "$KEPT" 2>/dev/null || true
  warm_environment >&2 || true
  fatal "$1" "read the fixture's environment above and the evidence lists measure.sh wrote, kept in $KEPT, then $2"
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
# The fixture faced what it exists for: rival restored builds of crate c, both of which the
# ordinary build uses, so a pass that did not know they were in use would remove one and the
# next build would make it anew. measure.sh marks them used, so no backlog pass removes a
# build the ordinary build reported, and the backlog settles by its second pass (a stale
# incremental build, which CI never makes, is all a first pass may find).
rivals="$(grep -cE '^target/debug/\.fingerprint/c-' "$evidence/backlog-before-1.txt" || true)"
if [ "${rivals:-0}" -lt 2 ]; then
  warm_fatal "the fixture held $rivals build(s) of crate c, so it no longer reproduces a warm, cache-restored directory" \
    "restore the fixture's rival builds of one unit (crate c under two feature sets) and its restored dates"
fi
removed_in_use="$(cat "$evidence"/backlog-removed-*.txt \
  | python3 "$ROOT/pool-maintenance/ordinary_builds.py" entries "$evidence/ordinary-hashes.txt")"
if [ -n "$removed_in_use" ]; then
  warm_fatal "an untimed pass removed builds the ordinary build uses: $removed_in_use" \
    "restore measure.sh's marking of the ordinary build's entries as used before each backlog pass"
fi
if [ -f "$evidence/backlog-removed-3.txt" ]; then
  warm_fatal "the backlog did not settle after its first pass" \
    "restore measure.sh's marking of the ordinary build's entries as used before each backlog pass"
fi
if ! { [ -s "$evidence/removed.txt" ] && [ ! -s "$evidence/removed-not-alternative.txt" ] \
  && [ ! -s "$evidence/ordinary-missing.txt" ]; }; then
  warm_fatal "the timed pass did not remove exactly the stand-in copies" \
    "fix measure.sh's setup so the timed pass faces one run's superseded builds and nothing else"
fi

# 2-4. Each departure is an error naming its cause, never a figure.
for case in \
  "timed-removes-nothing|the timed pass removed nothing" \
  "timed-removes-ordinary|the timed pass removed entries the stand-in copies did not create" \
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

# 5. The copies measure.sh times the removal of are the builds themselves: same bytes, links
#    as links, an existing stand-in untouched, entries of other builds not copied.
tree="$scratch/copies"
mkdir -p "$tree/target/debug/deps" "$tree/target/debug/.fingerprint/a-1111111111111111"
printf 'library bytes\n' > "$tree/target/debug/deps/liba-1111111111111111.rlib"
printf 'fingerprint\n' > "$tree/target/debug/.fingerprint/a-1111111111111111/lib-a"
ln -s lib-a "$tree/target/debug/.fingerprint/a-1111111111111111/link"
printf 'other build\n' > "$tree/target/debug/deps/libb-2222222222222222.rlib"
printf '1111111111111111\n' > "$tree/hashes.txt"
stand_in="$(printf '1111111111111111' | sha256sum | cut -c1-16)"
mkdir "$tree/target/debug/.fingerprint/a-$stand_in"
printf 'already here\n' > "$tree/target/debug/.fingerprint/a-$stand_in/kept"
(cd "$tree" && find target/debug/deps target/debug/.fingerprint -mindepth 1 -maxdepth 1 \
  | LC_ALL=C sort | python3 "$ROOT/pool-maintenance/ordinary_builds.py" copy hashes.txt) \
  || fatal "the helper's copy failed over a small tree" "run pool-maintenance/ordinary_builds.py copy by hand over $tree"
cmp -s "$tree/target/debug/deps/liba-1111111111111111.rlib" "$tree/target/debug/deps/liba-$stand_in.rlib" \
  || fatal "a stand-in copy is not byte for byte its build" "restore the byte copy in ordinary_builds.py"
[ "$(readlink "$tree/target/debug/.fingerprint/a-$stand_in/link" 2>/dev/null)" = "" ] \
  || fatal "the existing stand-in was overwritten by a copy" "restore ordinary_builds.py's skip of a stand-in already present"
[ "$(cat "$tree/target/debug/.fingerprint/a-$stand_in/kept")" = "already here" ] \
  || fatal "the existing stand-in did not keep its own contents" "restore ordinary_builds.py's skip of a stand-in already present"
rm -rf "$tree/target/debug/.fingerprint/a-$stand_in"
(cd "$tree" && find target/debug/.fingerprint -mindepth 1 -maxdepth 1 \
  | python3 "$ROOT/pool-maintenance/ordinary_builds.py" copy hashes.txt) \
  || fatal "the helper's copy of a directory failed" "run pool-maintenance/ordinary_builds.py copy by hand over $tree"
[ "$(readlink "$tree/target/debug/.fingerprint/a-$stand_in/link")" = "lib-a" ] \
  || fatal "a symlink inside a build was not copied as a link" "restore copytree(symlinks=True) in ordinary_builds.py"
cmp -s "$tree/target/debug/.fingerprint/a-1111111111111111/lib-a" "$tree/target/debug/.fingerprint/a-$stand_in/lib-a" \
  || fatal "a file inside a copied build is not byte for byte its original" "restore the byte copy in ordinary_builds.py"
if ls "$tree/target/debug/deps" | grep -qv -e '^liba-' -e '^libb-2222222222222222.rlib$'; then
  fatal "the helper copied an entry of a build the ordinary build did not report: $(ls "$tree/target/debug/deps")" \
    "restore ordinary_builds.py's selection by the reported hashes"
fi
