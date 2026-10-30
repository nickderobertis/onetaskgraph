#!/usr/bin/env bash
# Drive scripts/check-warm-rust-cache-reuse.sh --warmed with the real cargo, cargo-llvm-cov and
# cargo-nextest, as the warmer's Linux leg does. It runs, unchanged, in a scratch workspace
# beside the real scripts/warm-rust-cache.sh, scripts/pr-build-commands.sh and
# scripts/rust-coverage.sh: one member crate, named as the check's control expects, depending
# on one crate outside the workspace, so cargo compiles a dependency without the registry.
# Cargo's output is coloured, as CARGO_TERM_COLOR=always colours it on the hosted runner.
#
# After the real warmer (--instrumented, the Linux leg's coverage build included) fills a
# scratch target directory, the check has to pass; after the dependency's source changes, so
# that every build compiles it again, it has to fail naming that dependency.
# scripts/check-warm-rust-cache-reuse-enforced.sh covers every other case behind a stand-in.
#
#   scripts/check-warm-rust-cache-reuse-real.sh
#
# The check proves the Linux leg and skips elsewhere, so this does too. Quiet on success.
set -euo pipefail

# Nothing here may open a live session; it builds only the scratch crate's tests.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

fail() {
  echo "check-warm-rust-cache-reuse-real: $1" >&2
  echo "check-warm-rust-cache-reuse-real: next: $2" >&2
  exit 1
}

if [ "$(uname -s)" != Linux ]; then
  echo "check-warm-rust-cache-reuse-real: skipped on $(uname -s), as the check it drives is" >&2
  exit 0
fi

for tool in cargo-llvm-cov cargo-nextest; do
  command -v "$tool" >/dev/null || fail "$tool is not on PATH" "install it ('cargo install --locked $tool'), as the Linux check job's install-action does"
done

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d "${ONEPIPELINE_NODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/warm-rust-cache-reuse-real.XXXXXX")" \
  || fail "could not create a scratch directory" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

tree="$scratch/tree"
crate="$tree/crates/onetaskgraph-plugin-api"
dep="$scratch/reuse-probe-dep"
mkdir -p "$tree/scripts" "$tree/.cargo" "$crate/src" "$dep/src" \
  || fail "could not create the scratch workspace in $scratch" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
for script in check-warm-rust-cache-reuse.sh warm-rust-cache.sh pr-build-commands.sh rust-coverage.sh; do
  cp "$root/scripts/$script" "$tree/scripts/" || fail "could not copy scripts/$script into $scratch" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
done
# A scratch tree outside this clone would otherwise build with whatever rustc is the default
# and without -D warnings, which change every unit's fingerprint from the leg's.
{ cp "$root/rust-toolchain.toml" "$tree/" && cp "$root/.cargo/config.toml" "$tree/.cargo/"; } \
  || fail "could not copy rust-toolchain.toml and .cargo/config.toml into $scratch" "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
# The release build is the distribution journey's, which this scratch workspace has none of.
printf '%s\n' '#!/usr/bin/env bash' 'echo "    Finished \`release\` profile"' >"$tree/scripts/test-distribution.sh"
cat >"$tree/Cargo.toml" <<'TOML'
[workspace]
members = ["crates/onetaskgraph-plugin-api"]
resolver = "2"
TOML
cat >"$crate/Cargo.toml" <<'TOML'
[package]
name = "onetaskgraph-plugin-api"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
reuse-probe-dep = { path = "../../../reuse-probe-dep" }
TOML
cat >"$crate/src/lib.rs" <<'RS'
pub fn answer() -> u32 {
    reuse_probe_dep::answer()
}

#[test]
fn answers() {
    assert_eq!(answer(), 42);
}
RS
cat >"$crate/project.json" <<'JSON'
{"targets": {"test": {"options": {"command": "cargo test --quiet -p onetaskgraph-plugin-api --all-features --locked"}}}}
JSON
cat >"$dep/Cargo.toml" <<'TOML'
[package]
name = "reuse-probe-dep"
version = "0.1.0"
edition = "2021"
publish = false
TOML
printf '%s\n' 'pub fn answer() -> u32 {' '    42' '}' >"$dep/src/lib.rs"
(cd "$tree" && cargo generate-lockfile --offline --quiet) \
  || fail "could not lock the scratch workspace" "run 'cargo generate-lockfile --offline' in a copy of $tree and fix what it reports"

# check <name>: the check over the scratch target directory, its output in $scratch/<name>.log.
check() {
  CARGO_TARGET_DIR="$scratch/target" CARGO_TERM_COLOR=always \
    bash "$tree/scripts/check-warm-rust-cache-reuse.sh" --warmed >"$scratch/$1.log" 2>&1
}

CARGO_TARGET_DIR="$scratch/target" CARGO_TERM_COLOR=always \
  bash "$tree/scripts/warm-rust-cache.sh" --instrumented >"$scratch/warm.log" 2>&1 \
  || { cat "$scratch/warm.log" >&2; fail "the real warmer failed over the scratch workspace" "fix what it reports above"; }

check warmed || { cat "$scratch/warmed.log" >&2; fail "the check refused a warm build of the same inputs" "make scripts/check-warm-rust-cache-reuse.sh read what cargo printed, ANSI escapes and all"; }

# An input change only the dependency sees: every build then has to compile it again.
printf '%s\n' 'pub fn answer() -> u32 {' '    40 + 2' '}' >"$dep/src/lib.rs"
if check changed; then
  cat "$scratch/changed.log" >&2
  fail "the check passed although the dependency's source changed after the warm" "make scripts/check-warm-rust-cache-reuse.sh match cargo's 'Compiling' lines, ANSI escapes and all"
fi
grep -q "^reuse-probe-dep (by 'cargo llvm-cov --no-report nextest --package onetaskgraph-plugin-api " "$scratch/changed.log" \
  || { cat "$scratch/changed.log" >&2; fail "the refusal does not name the dependency the coverage build compiled again" "list each as '<crate> (by '<build>')'"; }
exit 0
