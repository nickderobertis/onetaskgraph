#!/usr/bin/env bash
# Build what test-linux and test-windows build, and run nothing.
#
#   scripts/warm-rust-cache.sh [--instrumented]
#
# .github/workflows/rust-cache-warm.yml runs this on every push to main and saves the Rust
# cache under the key those two pull-request jobs restore, because a pull request may restore
# only a cache saved on its own ref or on main, and they run on pull requests alone: without
# it every pull request compiled every dependency, and the distribution journey's release
# build — eight to ten minutes — on every run. The cache keeps dependency units only, so what
# matters is that these are the very units the jobs compile:
#
#   every workspace crate's test units, with every feature and with its defaults, as the
#   crates' `test` targets build them;
#   with --instrumented (Linux), the same under cargo-llvm-cov, exactly as
#   scripts/rust-coverage.sh builds them, with a filter that selects no test;
#   the distribution journey's release build, by its own --release-build-only.
set -euo pipefail

fatal() {
  echo "warm-rust-cache: $1" >&2
  echo "warm-rust-cache: next: $2" >&2
  exit 1
}

instrumented=false
case "${1:-}" in
  "") ;;
  --instrumented) instrumented=true ;;
  *) fatal "takes no argument but --instrumented, not '$1'" "run it with none, or with --instrumented on Linux" ;;
esac

# Nothing here opens a live session, and nothing may: it builds the live crates' tests.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run it from a checkout of this repository"
cd "$ROOT"

crates="$(cargo metadata --no-deps --format-version 1 --locked \
  | python3 -c 'import json, sys; print("\n".join(p["name"] for p in json.load(sys.stdin)["packages"]))' \
  | tr -d '\r')" || fatal "could not list the workspace's crates" "run 'cargo metadata --no-deps' and fix what it reports"
[ -n "$crates" ] || fatal "the workspace lists no crate" "run 'cargo metadata --no-deps' and fix what it reports"

for crate in $crates; do
  cargo test --quiet --no-run --locked --all-features -p "$crate"
  cargo test --quiet --no-run --locked -p "$crate"
  if "$instrumented"; then
    cargo llvm-cov --no-report nextest --package "$crate" --all-features --locked \
      -E 'none()' --no-tests=pass
  fi
done

bash scripts/test-distribution.sh --release-build-only
