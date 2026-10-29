#!/usr/bin/env bash
# Watch scripts/check-warm-rust-cache-reuse.sh refuse. It runs, unchanged, in a scratch tree
# beside the real scripts/pr-build-commands.sh and scripts/rust-coverage.sh, one crate's
# project.json, and a stand-in `cargo` that prints what a real one would: `Compiling` lines for
# the builds a case names, and `Finished` unless a case silences it. An incomplete warm — a build of the job's
# compiling a dependency again — has to fail naming that dependency and the build; a complete
# one has to pass, warming first unless given --warmed, which warms nothing; a scan that cannot
# see a rebuild, because the control's incomplete warm reports none, and a build whose cargo
# output never reached the scan, have to fail.
#
#   scripts/check-warm-rust-cache-reuse-enforced.sh
#
# The check proves the Linux leg and skips elsewhere, so this does too. Quiet on success.
set -euo pipefail

# Nothing here may open a live session; scripts/check-live-lane.sh holds it to that.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

fail() {
  echo "check-warm-rust-cache-reuse-enforced: $1" >&2
  echo "check-warm-rust-cache-reuse-enforced: next: $2" >&2
  exit 1
}

if [ "$(uname -s)" != Linux ]; then
  echo "check-warm-rust-cache-reuse-enforced: skipped on $(uname -s), as the check it watches is" >&2
  exit 0
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d "${ONEPIPELINE_NODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/warm-rust-cache-reuse-enforced.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

tree="$scratch/tree"
mkdir -p "$tree/scripts" "$tree/crates/onetaskgraph-plugin-api" "$scratch/bin"
for script in check-warm-rust-cache-reuse.sh pr-build-commands.sh rust-coverage.sh; do
  cp "$root/scripts/$script" "$tree/scripts/"
done
printf '%s\n' '#!/usr/bin/env bash' 'echo "warm $*" >>"$WARM_RECORD"' >"$tree/scripts/warm-rust-cache.sh"
printf '%s\n' '#!/usr/bin/env bash' 'echo "    Finished \`release\` profile"' >"$tree/scripts/test-distribution.sh"
cat >"$tree/crates/onetaskgraph-plugin-api/project.json" <<'JSON'
{"targets": {"test": {"options": {"command": "cargo test --quiet -p onetaskgraph-plugin-api --all-features --locked"}}}}
JSON
# The stand-in: metadata names the one crate; a build compiles `serde` again when it is the
# control's instrumented build into its empty directory, or when it matches $REUSE_REBUILD.
cat >"$scratch/bin/cargo" <<'STUB'
#!/usr/bin/env bash
if [ "$1" = metadata ]; then
  echo '{"packages":[{"name":"onetaskgraph-plugin-api"}]}'
  exit 0
fi
if { [ "$1" = llvm-cov ] && [ "${CARGO_TARGET_DIR##*/}" = empty ] && [ -z "${REUSE_BLIND:-}" ]; } \
  || { [ -n "${REUSE_REBUILD:-}" ] && [[ "$*" == *"$REUSE_REBUILD"* ]]; }; then
  echo "   Compiling serde v1.0.0" >&2
fi
[ -n "${REUSE_SILENT:-}" ] && [[ "$*" == *"$REUSE_SILENT"* ]] && exit 0
echo "    Finished \`test\` profile" >&2
STUB
chmod +x "$scratch/bin/cargo"

# run <name> [VAR=value...]: run the check in the scratch tree with $REUSE_ARGS, its output in
# $scratch/<name>.log and every warmer invocation in $scratch/<name>.warm.
run() {
  local name="$1"
  shift
  : >"$scratch/$name.warm"
  # shellcheck disable=SC2086 # The arguments are the words they spell.
  env "$@" WARM_RECORD="$scratch/$name.warm" PATH="$scratch/bin:$PATH" \
    bash "$tree/scripts/check-warm-rust-cache-reuse.sh" ${REUSE_ARGS:-} >"$scratch/$name.log" 2>&1
}

status=0
REUSE_ARGS=--bogus run usage || status=$?
[ "$status" -eq 2 ] || { cat "$scratch/usage.log" >&2; fail "an unknown argument exited $status, not 2, its usage error" "exit 2 on any argument but --warmed alone"; }

run complete || { cat "$scratch/complete.log" >&2; fail "a complete warm was refused" "fix what the check reports above"; }
[ "$(cat "$scratch/complete.warm")" = "warm --instrumented" ] \
  || fail "the check did not warm once, in the Linux leg's mode, before its builds" "run scripts/warm-rust-cache.sh --instrumented unless given --warmed"

REUSE_ARGS=--warmed run warmed || { cat "$scratch/warmed.log" >&2; fail "--warmed over a complete warm was refused" "fix what the check reports above"; }
[ ! -s "$scratch/warmed.warm" ] || fail "--warmed ran the warmer again" "build nothing first under --warmed"

if run incomplete REUSE_REBUILD=llvm-cov; then
  fail "an instrumented build that compiled serde again passed" "keep the refusal at the end of scripts/check-warm-rust-cache-reuse.sh"
fi
grep -q "serde (by 'cargo llvm-cov --no-report nextest --package onetaskgraph-plugin-api" "$scratch/incomplete.log" \
  || { cat "$scratch/incomplete.log" >&2; fail "the refusal does not name the dependency and the build that compiled it" "list each as '<crate> (by '<build>')'"; }

if run blind REUSE_BLIND=1; then
  fail "a scan that saw no rebuild in the control's incomplete warm passed" "keep the control in scripts/check-warm-rust-cache-reuse.sh"
fi
grep -q "an incomplete warm passed" "$scratch/blind.log" \
  || { cat "$scratch/blind.log" >&2; fail "the control's refusal does not say the incomplete warm passed" "keep its message"; }

if run silent REUSE_SILENT="--package onetaskgraph-plugin-api --all-features --locked -E"; then
  fail "a build that printed no cargo 'Finished' line passed" "keep the Finished check in scripts/check-warm-rust-cache-reuse.sh"
fi
grep -q "printed no cargo 'Finished' line" "$scratch/silent.log" \
  || { cat "$scratch/silent.log" >&2; fail "the refusal of a silent build does not say it printed no Finished line" "keep its message"; }
exit 0
