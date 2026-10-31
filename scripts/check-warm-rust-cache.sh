#!/usr/bin/env bash
# Drive scripts/warm-rust-cache.sh over a scratch copy of itself, behind a stand-in `cargo` that
# answers `cargo metadata` with two crates and records every other invocation, and a stand-in
# distribution journey that records its arguments. Each leg must build every crate's test units
# with every feature and with its defaults, the Linux leg each crate's instrumented units and
# the crate publication's packaging verification too, and both the distribution journey's
# release build alone; any other invocation is refused.
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

fail() {
  echo "check-warm-rust-cache: $1" >&2
  echo "check-warm-rust-cache: next: $2" >&2
  exit 1
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d "${ONEPIPELINE_NODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/warm-rust-cache.XXXXXX")" \
  || fail "could not create a scratch directory" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

tree="$scratch/tree"
{ mkdir -p "$tree/scripts" "$scratch/bin" && cp "$root/scripts/warm-rust-cache.sh" "$tree/scripts/"; } \
  || fail "could not plant the warmer in $scratch" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
# Spelled as the warmer resolves its own root, `cd` and `pwd`: macOS's TMPDIR ends in a slash,
# and the doubled one it leaves in $scratch is not in the target directory the warmer names.
tree="$(cd "$tree" && pwd)"
cat >"$tree/scripts/test-distribution.sh" <<'STUB'
echo "distribution $*" >>"$WARM_RECORD"
STUB
cat >"$scratch/bin/cargo" <<'STUB'
#!/usr/bin/env bash
if [ "$1" = metadata ]; then
  echo '{"packages":[{"name":"alpha"},{"name":"beta"}]}'
else
  echo "cargo $*" >>"$WARM_RECORD"
  [ -z "${WARM_FAIL:-}" ] || [[ "$*" != *"$WARM_FAIL"* ]] || exit 101
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
  [ "$1" = plain ] || echo "cargo package -p alpha -p beta --locked --allow-dirty --target-dir $tree/target/crate-publication"
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
        elif word in ("-p", "--package", "--test", "-E", "--target-dir"):
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
  status=0
  # shellcheck disable=SC2086 # Each case is the words it spells.
  leg refused $arguments || status=$?
  [ "$status" -eq 2 ] || fail "the warmer exited $status with '$arguments', not 2, its usage error" "refuse every argument but --instrumented alone in scripts/warm-rust-cache.sh, exiting 2"
  [ ! -s "$scratch/refused" ] || fail "the warmer built before refusing '$arguments'" "refuse its arguments before building anything"
  grep -q "not '$arguments'" "$scratch/refused.log" || { cat "$scratch/refused.log" >&2; fail "the refusal of '$arguments' does not name it" "name the arguments in the refusal"; }
done

# A build that fails stops the warmer, exit 1, naming that build and a next action.
: >"$scratch/broken"
status=0
WARM_FAIL="-p beta" leg broken --instrumented || status=$?
[ "$status" -eq 1 ] && grep -q "building beta's test units with every feature failed" "$scratch/broken.log" \
  && grep -q "next:" "$scratch/broken.log" && [ "$(tail -1 "$scratch/broken")" = "cargo test --quiet --no-run --locked --all-features -p beta" ] \
  || { cat "$scratch/broken.log" >&2; fail "the warmer exited $status on a failed build of beta, without stopping there naming it and a next action" "end each build in scripts/warm-rust-cache.sh with || fatal naming it"; }

# The release build it hands to the distribution journey, driven for real behind a stand-in
# `uv`: --release-build-only has to make exactly the journey's maturin build and stop there,
# and any other argument has to be refused, exit 2, before anything is built. The journey's
# `uv` is spawned by python, and on Windows CreateProcess resolves only `uv.exe`, so the
# stand-in is a real executable compiled here by the pinned toolchain — the same harness on
# every platform. It records its arguments beside itself, a path no environment translation
# between bash and a native process can rewrite.
journey="$scratch/journey"
journey_bin="$scratch/journey-bin"
{ mkdir -p "$journey/scripts" "$journey/crates/onetaskgraph" "$journey_bin" \
  && cp "$root/scripts/test-distribution.sh" "$root/scripts/cli_wheel_lock.py" "$root/scripts/file-lock.sh" "$journey/scripts/"; } \
  || fail "could not plant the distribution journey in $scratch" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
printf '[package]\nname = "onetaskgraph"\nversion = "1.2.3"\n' >"$journey/crates/onetaskgraph/Cargo.toml"
cat >"$scratch/uv.rs" <<'RUST'
use std::io::Write;
fn main() {
    let record = std::env::current_exe().unwrap().with_file_name("uv.record");
    let words: Vec<String> = std::env::args().skip(1).collect();
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(record).unwrap();
    writeln!(file, "uv {}", words.join(" ")).unwrap();
}
RUST
exe=""
case "${OS:-}${OSTYPE:-}" in *Windows_NT* | *msys* | *cygwin* | *win32*) exe=.exe ;; esac
# From the root, so rustup reads rust-toolchain.toml rather than wanting a default toolchain.
(cd "$root" && rustc --edition 2021 -o "$journey_bin/uv$exe" "$scratch/uv.rs") >"$scratch/uv-build.log" 2>&1 \
  || { cat "$scratch/uv-build.log" >&2; fail "could not compile the stand-in uv" "fix what rustc reports above"; }
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' >"$journey_bin/npm"
chmod +x "$journey_bin/npm"
: >"$journey_bin/uv.record"
PATH="$journey_bin:$PATH" bash "$journey/scripts/test-distribution.sh" --release-build-only >"$scratch/journey.log" 2>&1 \
  || { cat "$scratch/journey.log" >&2; fail "test-distribution.sh --release-build-only failed" "fix what it reports above"; }
tr -d '\r' <"$journey_bin/uv.record" >"$scratch/journey.record"
grep -Eq "^uv tool run --from maturin>=1\.9,<2 maturin build --release --locked --target [^ ]+ --out [^ ]+[/\\]python-wheel\$" "$scratch/journey.record" \
  && [ "$(wc -l <"$scratch/journey.record" | tr -d ' ')" = 1 ] \
  || { cat "$scratch/journey.record" >&2; fail "--release-build-only made something other than the journey's one maturin build (above)" "keep it to build_release_wheel in scripts/test-distribution.sh and an exit"; }
: >"$journey_bin/uv.record"
status=0
PATH="$journey_bin:$PATH" bash "$journey/scripts/test-distribution.sh" --release-build-only extra >"$scratch/journey.log" 2>&1 || status=$?
[ "$status" -eq 2 ] && [ ! -s "$journey_bin/uv.record" ] \
  || { cat "$scratch/journey.log" >&2; fail "test-distribution.sh exited $status for '--release-build-only extra', not 2 before building" "refuse it as a usage error before any build"; }

# And scripts/pr-build-commands.sh refuses a file it cannot read naming that file and a next
# action, never with a traceback: a crate's project.json that does not parse, and a missing
# scripts/rust-coverage.sh.
reader="$scratch/reader"
{ mkdir -p "$reader/scripts" "$reader/crates/broken" && cp "$root/scripts/pr-build-commands.sh" "$reader/scripts/"; } \
  || fail "could not plant scripts/pr-build-commands.sh in $scratch" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
printf '{"targets": {"test": \n' >"$reader/crates/broken/project.json"
refused_by_reader() {
  local status=0
  bash "$reader/scripts/pr-build-commands.sh" >/dev/null 2>"$scratch/reader.log" || status=$?
  [ "$status" -eq 1 ] && grep -q "$1" "$scratch/reader.log" && grep -q "next:" "$scratch/reader.log" \
    && ! grep -q Traceback "$scratch/reader.log" \
    || { cat "$scratch/reader.log" >&2; fail "pr-build-commands.sh exited $status for $2, without naming $1 and a next action" "refuse it through sys.exit with the file and a next action"; }
}
refused_by_reader "crates/broken/project.json: could not be read as JSON" "a project.json that does not parse"
printf '{"targets": {"test": {"options": {"command": "cargo test -p broken"}}}}\n' >"$reader/crates/broken/project.json"
refused_by_reader "scripts/rust-coverage.sh could not be read" "a missing scripts/rust-coverage.sh"
exit 0
