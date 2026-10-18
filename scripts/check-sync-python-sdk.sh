#!/usr/bin/env bash
# Drive scripts/sync-python-sdk.sh's two modes into a scratch environment and read what each
# leaves there: CI's (ONETASKGRAPH_SDK_WITHOUT_CLI=1) installs the SDK and its tooling and not
# the onetaskgraph-cli member; the ordinary one's plan installs that member, read from a dry run
# because building it is the maturin build CI's mode exists to skip; any other value is refused.
#
#   scripts/check-sync-python-sdk.sh
#
# Quiet on success.
set -euo pipefail

# uv's plan is read as text, which Nx's FORCE_COLOR would otherwise colour.
export NO_COLOR=1
unset FORCE_COLOR CLICOLOR_FORCE
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d "${ONEPIPELINE_NODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/sync-python-sdk.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
fail() {
  echo "check-sync-python-sdk: $1" >&2
  echo "check-sync-python-sdk: next: $2" >&2
  exit 1
}
command -v uv >/dev/null 2>&1 || fail "uv is not on PATH" "run 'just bootstrap', which installs it, then re-run"

venv="$scratch/venv"
sync() { UV_PROJECT_ENVIRONMENT="$venv" bash "$root/scripts/sync-python-sdk.sh" "$@"; }
installed() {
  uv pip list --python "$venv" --format=json 2>/dev/null \
    | python3 -c 'import json, sys; print("\n".join(p["name"].lower().replace("_", "-") for p in json.load(sys.stdin)))' \
    | tr -d '\r'
}

ONETASKGRAPH_SDK_WITHOUT_CLI=1 sync >"$scratch/ci.log" 2>&1 \
  || { cat "$scratch/ci.log" >&2; fail "CI's mode (ONETASKGRAPH_SDK_WITHOUT_CLI=1) failed" "fix what uv reports above"; }
packages="$(installed)"
for package in onetaskgraph-sdk pydantic pytest ruff ty; do
  printf '%s\n' "$packages" | grep -qx "$package" \
    || fail "CI's mode left $package out of the environment" "keep the SDK's own dependencies and dev group in scripts/sync-python-sdk.sh's CI mode"
done
! printf '%s\n' "$packages" | grep -qx onetaskgraph-cli \
  || fail "CI's mode installed onetaskgraph-cli, the maturin build it exists to skip" "keep --no-install-package onetaskgraph-cli on scripts/sync-python-sdk.sh's CI mode"

env -u ONETASKGRAPH_SDK_WITHOUT_CLI bash -c 'UV_PROJECT_ENVIRONMENT="$1" bash "$2" --dry-run' _ "$venv" "$root/scripts/sync-python-sdk.sh" >"$scratch/ordinary.log" 2>&1 \
  || { cat "$scratch/ordinary.log" >&2; fail "the ordinary mode's dry run failed" "fix what uv reports above"; }
sed $'s/\x1b\\[[0-9;]*m//g' "$scratch/ordinary.log" | grep -q '+ onetaskgraph-cli' \
  || { cat "$scratch/ordinary.log" >&2; fail "the ordinary mode would not install onetaskgraph-cli, so a contributor's 'uv run onetaskgraph' has nothing to run" "keep the plain 'uv sync --frozen' as scripts/sync-python-sdk.sh's ordinary mode"; }
installed | grep -qx onetaskgraph-cli \
  && fail "the ordinary mode's dry run changed the environment" "pass the options to 'uv sync' unchanged"

if ONETASKGRAPH_SDK_WITHOUT_CLI=yes sync >"$scratch/bad.log" 2>&1; then status=0; else status=$?; fi
[ "$status" = 2 ] && grep -q "ONETASKGRAPH_SDK_WITHOUT_CLI is 'yes'" "$scratch/bad.log" \
  || { cat "$scratch/bad.log" >&2; fail "ONETASKGRAPH_SDK_WITHOUT_CLI=yes exited $status, not 2 naming the value" "refuse any value but 1 or unset in scripts/sync-python-sdk.sh"; }
exit 0
