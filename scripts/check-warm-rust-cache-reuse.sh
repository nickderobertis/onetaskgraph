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
# With --warmed it builds nothing first and checks the target directory cargo already uses,
# which the warmer has just filled: .github/workflows/rust-cache-warm.yml runs it so as the
# Linux leg's last step, where the job's builds find everything built and cost little, so a
# push to main fails the moment the cache it saves stops being one a pull request reuses.
#
# The job's builds, read from where they are declared: each crate's `test` target's cargo
# commands with --no-run, its instrumented build as scripts/rust-coverage.sh runs it with
# --no-run, and the distribution journey's release build, by the same function the journey
# itself calls. A first, cheap pass of one of them against an empty directory has to report a
# compiled dependency, or a scan that never matches would read as a pass.
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
  *) fail "takes no argument or --warmed alone, not '$*'" "run it with none, or with --warmed after scripts/warm-rust-cache.sh --instrumented" ;;
esac

if [ "$(uname -s)" != Linux ]; then
  echo "check-warm-rust-cache-reuse: skipped on $(uname -s); it proves the Linux leg, so run it on Linux" >&2
  exit 0
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
scratch="$(mktemp -d "${ONEPIPELINE_NODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/warm-rust-cache-reuse.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

python_bin="$(command -v python3)" || fail "python3 is not on PATH" "install python3 and re-run"

members="$(cargo metadata --no-deps --format-version 1 --locked \
  | "$python_bin" -c 'import json, sys; print("\n".join(p["name"] for p in json.load(sys.stdin)["packages"]))' \
  | tr -d '\r')" || fail "could not list the workspace's crates" "run 'cargo metadata --no-deps' and fix what it reports"

# consumer_builds: one command per line, every build the Linux pull-request job makes.
consumer_builds() {
  local project
  for project in crates/*/project.json; do
    "$python_bin" - "$project" <<'PY'
import json, sys
target = json.load(open(sys.argv[1], encoding="utf-8")).get("targets", {}).get("test", {})
options = target.get("options", {})
commands = options.get("commands") or [options.get("command", "")]
for command in commands:
    command = command if isinstance(command, str) else command.get("command", "")
    if command.startswith("cargo test "):
        print(command.replace(" --quiet", "") + " --no-run")
PY
  done | tr -d '\r'
  local crate
  for crate in $members; do
    echo "cargo llvm-cov --no-report nextest --package $crate --all-features --locked --no-run"
  done
  echo "bash scripts/test-distribution.sh --release-build-only"
}

# rebuilt <log>: the non-workspace crates a build log says cargo compiled, one per line.
rebuilt() {
  sed -n 's/^ *Compiling \([^ ]*\) v.*/\1/p' "$1" | tr -d '\r' | sort -u \
    | { grep -vxF -f <(printf '%s\n' $members) || true; }
}

# The control: the same scan over a build into an empty directory must see dependencies built.
CARGO_TARGET_DIR="$scratch/empty" cargo test --no-run --locked --all-features -p onetaskgraph-plugin-api \
  >"$scratch/control.log" 2>&1 || { cat "$scratch/control.log" >&2; fail "the control build failed" "fix what cargo reports above"; }
[ -n "$(rebuilt "$scratch/control.log")" ] || {
  cat "$scratch/control.log" >&2
  fail "a build into an empty directory reported no dependency compiled, so this scan cannot see one" "make rebuilt() match cargo's 'Compiling' lines again"
}
rm -rf "$scratch/empty"

if ! "$warmed"; then
  export CARGO_TARGET_DIR="$scratch/target"
  bash scripts/warm-rust-cache.sh --instrumented >"$scratch/warm.log" 2>&1 || {
    cat "$scratch/warm.log" >&2
    fail "scripts/warm-rust-cache.sh --instrumented failed" "fix what it reports above"
  }
fi

consumer_builds >"$scratch/builds"
[ -s "$scratch/builds" ] || fail "found no pull-request build to make" "keep the crates' test targets' cargo commands in crates/*/project.json"
: >"$scratch/rebuilt"
while IFS= read -r build; do
  log="$scratch/build.log"
  # shellcheck disable=SC2086 # Each line is the words of one command.
  $build </dev/null >"$log" 2>&1 || { cat "$log" >&2; fail "'$build' failed" "fix what it reports above"; }
  rebuilt "$log" | sed "s|\$| (by '$build')|" >>"$scratch/rebuilt"
done <"$scratch/builds"

if [ -s "$scratch/rebuilt" ]; then
  cat "$scratch/rebuilt" >&2
  fail "the pull-request builds compiled dependencies the warmer had built (listed above)" "make scripts/warm-rust-cache.sh build each of those commands' units exactly as the job does"
fi
exit 0
