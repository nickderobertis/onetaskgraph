#!/usr/bin/env bash
# Every call site the distribution journey reaches that starts a Python interpreter, run
# through the real scripts and nothing else of that journey.
#
# check-relative-interpreter.sh compares this targeted pass with the full journey.
#
# Each script runs from the root of the tree it works on. A scratch clone gets the probe
# interpreter planted at the same relative path, so the relative entry resolves from that
# root exactly as it does from this checkout's — the shape the defect needs.
set -euo pipefail

fail() {
  echo "test-python-call-sites: $1" >&2
  echo "test-python-call-sites: next: $2" >&2
  exit 1
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
[ -r scripts/scratch-clone.sh ] || fail "scripts/scratch-clone.sh is missing" \
  "restore it with 'git checkout -- scripts/scratch-clone.sh', then rerun"
# shellcheck source=scripts/scratch-clone.sh
source scripts/scratch-clone.sh
python_bin="$(command -v python3)" || fail "no python3 on PATH" "install python3, or run 'just bootstrap', then rerun"
case $python_bin in
  /* | ?:[\\/]*) ;;
  *) python_bin="$(cd "$(dirname "$python_bin")" && pwd)/$(basename "$python_bin")" ;;
esac
binary=target/debug/onetaskgraph
# Git Bash's file test accepts an extensionless .exe path; Python's file reader does not.
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) binary="$binary.exe" ;;
esac
[ -f "$binary" ] || fail "no $binary, which the release staging body packages" \
  "run 'scripts/nx.sh run onetaskgraph:build', then rerun"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
clone="$tmp/repo"
scratch_clone "$root" "$clone" || fail "could not clone this repository into $clone" "check 'df -h' for free space, then rerun"
copy_tracked_files "$root" "$clone" || fail "could not copy the working tree into $clone" "check 'df -h' for free space, then rerun"
probe="${ONETASKGRAPH_RELATIVE_INTERPRETER_PROBE:-}"
if [ -n "$probe" ]; then
  mkdir -p "$clone/$probe" && cp -R "$probe/bin" "$clone/$probe/bin" \
    || fail "could not plant the probe interpreter in $clone" "check 'df -h' for free space, then rerun"
fi

site() {
  local where="$1" description="$2" output
  shift 2
  if ! output="$(cd "$where" && "$@" 2>&1)"; then
    printf '%s\n' "$output" | sed 's/^/    /' >&2
    fail "$description failed" "fix what it reports above; a 'No such file or directory' or an exit 127 means it resolved its interpreter in one directory and ran it from another"
  fi
}

# The journey's own, which all go through the one "$python_bin" it resolves.
site "$root" "scripts/test-distribution.sh's interpreter" bash scripts/test-distribution.sh --interpreter-only
# The scripts it runs, as it runs them, each in a tree it may change.
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' crates/onetaskgraph/Cargo.toml | head -n1)"
site "$clone" "scripts/set-version.sh --check" bash scripts/set-version.sh --check
site "$clone" "scripts/set-version.sh $version" bash scripts/set-version.sh "$version"
site "$clone" "scripts/check-workspace-config.sh" bash scripts/check-workspace-config.sh
site "$clone" "scripts/check-distribution-contract.sh" bash scripts/check-distribution-contract.sh
# And the release workflow's staging body, which test-release-products.py runs; it starts
# Python itself where its archive is a zip.
target="$(cd "$clone" && "$python_bin" -c '
import sys
sys.path.insert(0, "scripts")
from release_cd import host_target
print(host_target().target)
' | tr -d '\r')"
site "$clone" "the release workflow's staging body for $target" \
  "$python_bin" scripts/test-release-products.py --stage-only "$root/$binary" --target "$target"
