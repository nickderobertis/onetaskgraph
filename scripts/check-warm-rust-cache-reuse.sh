#!/usr/bin/env bash
# Prove the Rust cache warmer leaves what a pull request's Linux build reuses: run the real
# scripts/warm-rust-cache.sh in the Linux leg's mode (--instrumented) into a scratch target
# directory, then make that job's builds against the same directory, and fail naming every
# dependency crate cargo compiled again. rust-cache saves the target directory's dependency
# units, so a dependency the second pass rebuilt is one the warmer's save would not have
# spared a pull request.
#
#   scripts/check-warm-rust-cache-reuse.sh [--warmed]
#
# Exits 0 when nothing was rebuilt (or off Linux), 1 when a build was or a step failed, 2 on
# any other argument.
#
# With --warmed it builds nothing first and checks the target directory cargo already uses,
# which the warmer has just filled: .github/workflows/rust-cache-warm.yml runs it so as the
# Linux leg's last step, where the job's builds find everything built and cost little, so a
# push to main fails the moment the cache it saves stops being one a pull request reuses.
#
# The job's builds are what scripts/pr-build-commands.sh reads from where each is declared.
# A first, cheap control warms one crate incompletely and has to be refused, or a scan that
# never matches would read as a pass.
#
# Without it, a cold build of every dependency three ways, so it is no phase of `check`: run it
# by name (`just warm-cache-reuse-check`) when the warmer, the build commands or the toolchain
# change.
# Linux only, as the leg it proves is; elsewhere it says so and exits 0. Quiet on success.
set -euo pipefail

# Nothing here may open a live session: it builds the live crates' tests and runs none.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

fail() {
  echo "check-warm-rust-cache-reuse: $1" >&2
  echo "check-warm-rust-cache-reuse: next: $2" >&2
  exit 1
}

warmed=false
case "$#:${1:-}" in
  0:) ;;
  1:--warmed) warmed=true ;;
  *)
    echo "check-warm-rust-cache-reuse: takes no argument or --warmed alone, not '$*'" >&2
    echo "check-warm-rust-cache-reuse: next: run it with none, or with --warmed after scripts/warm-rust-cache.sh --instrumented" >&2
    exit 2
    ;;
esac

if [ "$(uname -s)" != Linux ]; then
  echo "check-warm-rust-cache-reuse: skipped on $(uname -s); it proves the Linux leg, so run it on Linux" >&2
  exit 0
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
scratch="$(mktemp -d "${ONEPIPELINE_NODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/warm-rust-cache-reuse.XXXXXX")" \
  || fail "could not create a scratch directory" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

python_bin="$(command -v python3)" || fail "python3 is not on PATH" "install python3 and re-run"

members="$(cargo metadata --no-deps --format-version 1 --locked \
  | "$python_bin" -c 'import json, sys
document = json.load(sys.stdin)
packages = document.get("packages") if isinstance(document, dict) else None
if not isinstance(packages, list) or not packages or not all(
    isinstance(package, dict) and isinstance(package.get("name"), str) and package["name"].strip()
    for package in packages
):
    raise SystemExit("cargo metadata did not answer with a packages array of named packages")
print("\n".join(package["name"] for package in packages))
' \
  | tr -d '\r')" || fail "could not list the workspace's crates" "run 'cargo metadata --no-deps' and fix what it reports"

# plain <log>: the log as text, without the ANSI escapes a coloured run puts before each status
# word — the hosted runner sets CARGO_TERM_COLOR=always, which reaches cargo under cargo-llvm-cov
# and nextest alike, and a scan anchored on '^ *Compiling' would otherwise match no line at all.
plain() {
  sed "s/$(printf '\033')\[[0-9;]*[A-Za-z]//g" "$1" | tr -d '\r'
}

# rebuilt <log>: the non-workspace crates a build log says cargo compiled, one per line.
rebuilt() {
  local status=0
  plain "$1" | sed -n 's/^ *Compiling \([^ ]*\) v.*/\1/p' | sort -u \
    | grep -vxF -f <(printf '%s\n' $members) || status=$?
  # 1 is grep's "nothing left", which is the answer; anything above it is grep failing.
  [ "$status" -le 1 ] || fail "grep failed (exit $status) filtering $1" "re-run; if it repeats, check the scratch directory is readable"
}

# replay <builds> <rebuilt>: make each build, one command per line, over $CARGO_TARGET_DIR,
# listing in <rebuilt> every dependency one of them compiled and which.
replay() {
  local build log="$scratch/build.log"
  : >"$2"
  while IFS= read -r build; do
    # shellcheck disable=SC2086 # Each line is the words of one command.
    $build </dev/null >"$log" 2>&1 || { cat "$log" >&2; fail "'$build' failed" "fix what it reports above"; }
    # Cargo's own closing line, so this build's output reached the scan: a command whose cargo
    # output went elsewhere would otherwise report nothing compiled and pass. No `grep -q`: its
    # early exit would fail plain's sed on a closed pipe, which pipefail reports.
    plain "$log" | grep -E '^ *Finished ' >/dev/null || { cat "$log" >&2; fail "'$build' printed no cargo 'Finished' line, so this scan cannot see what it compiled" "let that command's cargo output reach its standard error"; }
    rebuilt "$log" | sed "s|\$| (by '$build')|" >>"$2"
  done <"$1"
}

bash scripts/pr-build-commands.sh >"$scratch/builds"
[ -s "$scratch/builds" ] || fail "found no pull-request build to make" "keep the crates' test targets' cargo commands in crates/*/project.json"

# The control: an incomplete warm — one crate's plain test units, none instrumented — then
# that crate's instrumented build from the job's own list, which has to be refused naming the
# dependencies it compiled, or a refusal this check owes would read as a pass.
grep -E -- '^cargo llvm-cov .*--package onetaskgraph-plugin-api ' "$scratch/builds" >"$scratch/control.builds" \
  || fail "the job's builds hold no instrumented build of onetaskgraph-plugin-api for the control" "point the control at a crate scripts/pr-build-commands.sh lists"
CARGO_TARGET_DIR="$scratch/empty" cargo test --no-run --locked --all-features -p onetaskgraph-plugin-api \
  >"$scratch/control.log" 2>&1 || { cat "$scratch/control.log" >&2; fail "the control's partial warm failed" "fix what cargo reports above"; }
CARGO_TARGET_DIR="$scratch/empty" replay "$scratch/control.builds" "$scratch/control.rebuilt"
[ -s "$scratch/control.rebuilt" ] || fail "an incomplete warm passed: the instrumented build after an uninstrumented one reported no dependency compiled" "make rebuilt() match cargo's 'Compiling' lines again"
rm -rf "$scratch/empty"

if ! "$warmed"; then
  export CARGO_TARGET_DIR="$scratch/target"
  bash scripts/warm-rust-cache.sh --instrumented >"$scratch/warm.log" 2>&1 || {
    cat "$scratch/warm.log" >&2
    fail "scripts/warm-rust-cache.sh --instrumented failed" "fix what it reports above"
  }
fi

replay "$scratch/builds" "$scratch/rebuilt"
if [ -s "$scratch/rebuilt" ]; then
  cat "$scratch/rebuilt" >&2
  fail "the pull-request builds compiled dependencies the warmer had built (listed above)" "make scripts/warm-rust-cache.sh build each of those commands' units exactly as the job does"
fi
exit 0
