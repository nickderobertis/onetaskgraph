#!/usr/bin/env bash
# Drive scripts/warm-rust-cache.sh over a scratch copy of itself, behind a stand-in `cargo` that
# answers `cargo metadata` with two crates and records every other invocation, and a stand-in
# distribution journey that records its arguments. Each leg must build every crate's test units
# with every feature and with its defaults, the Linux leg each crate's instrumented units too,
# and both the distribution journey's release build alone; any other invocation is refused.
# Each build scripts/pr-build-commands.sh reads test-linux making has to be one of those
# shapes. A build the warmer stops making leaves every later pull request cold, and nothing
# else says so.
#
#   scripts/check-warm-rust-cache.sh
#
# Quiet on success.
set -euo pipefail

# Nothing here may open a live session; scripts/check-live-lane.sh holds it to that.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d "${ONEPIPELINE_NODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/warm-rust-cache.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
fail() {
  echo "check-warm-rust-cache: $1" >&2
  echo "check-warm-rust-cache: next: $2" >&2
  exit 1
}

tree="$scratch/tree"
mkdir -p "$tree/scripts" "$scratch/bin"
cp "$root/scripts/warm-rust-cache.sh" "$tree/scripts/"
cat >"$tree/scripts/test-distribution.sh" <<'STUB'
echo "distribution $*" >>"$WARM_RECORD"
STUB
cat >"$scratch/bin/cargo" <<'STUB'
#!/usr/bin/env bash
if [ "$1" = metadata ]; then
  echo '{"packages":[{"name":"alpha"},{"name":"beta"}]}'
else
  echo "cargo $*" >>"$WARM_RECORD"
fi
STUB
chmod +x "$scratch/bin/cargo"

# leg <name> <arguments...>: run the warmer, leaving what it ran in $scratch/<name>.
leg() {
  local name="$1"
  shift
  WARM_RECORD="$scratch/$name" PATH="$scratch/bin:$PATH" bash "$tree/scripts/warm-rust-cache.sh" "$@" \
    >"$scratch/$name.log" 2>&1
}
expected() {
  for crate in alpha beta; do
    echo "cargo test --quiet --no-run --locked --all-features -p $crate"
    echo "cargo test --quiet --no-run --locked -p $crate"
    [ "$1" = plain ] || echo "cargo llvm-cov --no-report nextest --package $crate --all-features --locked -E none() --no-tests=pass"
  done
  echo "distribution --release-build-only"
}

for name in plain instrumented; do
  arguments=()
  [ "$name" = plain ] || arguments=(--instrumented)
  : >"$scratch/$name"
  leg "$name" ${arguments[@]+"${arguments[@]}"} || { cat "$scratch/$name.log" >&2; fail "the $name leg failed" "fix what it reports above"; }
  if ! diff <(expected "$name") "$scratch/$name" >"$scratch/$name.diff"; then
    cat "$scratch/$name.diff" >&2
    fail "the $name leg built something other than what the pull-request jobs restore (expected '<', ran '>')" "keep scripts/warm-rust-cache.sh building every crate's test units, instrumented on Linux, and the release build alone"
  fi
done

# And every build test-linux makes, read from where each is declared, has to be one the
# instrumented leg makes for every crate, up to the crate, the test target and the filter.
bash "$root/scripts/pr-build-commands.sh" >"$scratch/pr-builds" 2>"$scratch/pr-builds.log" \
  || { cat "$scratch/pr-builds.log" >&2; fail "scripts/pr-build-commands.sh failed" "fix what it reports above"; }
python3 - "$scratch/instrumented" "$scratch/pr-builds" >"$scratch/unwarmed" <<'PY' || fail "could not compare the warmer's builds with test-linux's" "run scripts/pr-build-commands.sh and fix what it reports"
import sys

def shape(line):
    words = line.split()
    if words[:1] != ["cargo"]:
        return None
    kept, skip = [], False
    for word in words[1:]:
        if skip:
            skip = False
        elif word in ("-p", "--package", "--test", "-E"):
            skip = True
        elif word not in ("--quiet", "--no-run", "--no-tests=pass"):
            kept.append(word)
    return (kept[0], tuple(sorted(kept[1:])))

warmed = {shape(line) for line in open(sys.argv[1], encoding="utf-8").read().splitlines()}
for line in open(sys.argv[2], encoding="utf-8").read().splitlines():
    built = shape(line)
    if built is not None and built not in warmed:
        print(line)
PY
if [ -s "$scratch/unwarmed" ]; then
  tr -d '\r' <"$scratch/unwarmed" >&2
  fail "test-linux makes the builds above, which scripts/warm-rust-cache.sh makes in no shape" "build each of them in scripts/warm-rust-cache.sh with the same flags"
fi

for arguments in "--bogus" "--instrumented extra"; do
  : >"$scratch/refused"
  # shellcheck disable=SC2086 # Each case is the words it spells.
  if leg refused $arguments; then
    fail "the warmer ran with '$arguments'" "refuse every argument but --instrumented alone in scripts/warm-rust-cache.sh"
  fi
  [ ! -s "$scratch/refused" ] || fail "the warmer built before refusing '$arguments'" "refuse its arguments before building anything"
  grep -q "not '$arguments'" "$scratch/refused.log" || { cat "$scratch/refused.log" >&2; fail "the refusal of '$arguments' does not name it" "name the arguments in the refusal"; }
done
exit 0
