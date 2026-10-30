#!/usr/bin/env bash
# Build what test-linux and test-windows build, and run nothing.
#
#   scripts/warm-rust-cache.sh [--instrumented]
#
# Exits 0 once everything is built, 1 when a build fails, 2 on any other argument.
#
# .github/workflows/rust-cache-warm.yml runs this on every push to main. The cache keeps
# dependency units only, so what matters is that these are the very units the jobs compile —
# scripts/check-warm-rust-cache.sh holds them to the shapes scripts/pr-build-commands.sh reads:
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
case "$#:${1:-}" in
  0:) ;;
  1:--instrumented) instrumented=true ;;
  *)
    echo "warm-rust-cache: takes no argument or --instrumented alone, not '$*'" >&2
    echo "warm-rust-cache: next: run it with none, or with --instrumented on Linux" >&2
    exit 2
    ;;
esac

# Nothing here opens a live session, and nothing may: it builds the live crates' tests.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run it from a checkout of this repository"
cd "$ROOT"

crates="$(cargo metadata --no-deps --format-version 1 --locked \
  | python3 -c 'import json, sys
document = json.load(sys.stdin)
packages = document.get("packages") if isinstance(document, dict) else None
if not isinstance(packages, list) or not packages or not all(
    isinstance(package, dict) and isinstance(package.get("name"), str) and package["name"].strip()
    for package in packages
):
    raise SystemExit("cargo metadata did not answer with a packages array of named packages")
print("\n".join(package["name"] for package in packages))
' \
  | tr -d '\r')" || fatal "could not list the workspace's crates" "run 'cargo metadata --no-deps' and fix what it reports"
[ -n "$crates" ] || fatal "the workspace lists no crate" "run 'cargo metadata --no-deps' and fix what it reports"

next="fix what it reports above, then rerun; the pull-request job it warms would fail the same way"
for crate in $crates; do
  cargo test --quiet --no-run --locked --all-features -p "$crate" \
    || fatal "building $crate's test units with every feature failed" "$next"
  cargo test --quiet --no-run --locked -p "$crate" \
    || fatal "building $crate's test units with its default features failed" "$next"
  if "$instrumented"; then
    cargo llvm-cov --no-report nextest --package "$crate" --all-features --locked \
      -E 'none()' --no-tests=pass \
      || fatal "building $crate's instrumented test units failed" "$next"
  fi
done

bash scripts/test-distribution.sh --release-build-only \
  || fatal "the distribution journey's release build failed" "$next"
