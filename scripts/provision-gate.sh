#!/usr/bin/env bash
# Make this worktree able to run `just gate`, or say exactly what is missing.
#
# The pre-push hook runs the complete gate, and a fresh worktree has none of what that
# gate needs: no node_modules, so Nx is absent; no synced Python environment, so the SDK's
# targets have no interpreter. Discovering that from the hook reads as a *push rejection* —
# the person sees "the gate failed" and goes looking at their change, when nothing about
# their change is wrong. Every publication from one orchestration host was refused for
# twenty-nine minutes on exactly that.
#
# So the rule this script exists to keep: a step that requires provisioning provisions it,
# or declines out loud naming what is missing. The two halves are deliberately different.
#
#   * What this worktree can heal itself — the locked Node install and the SDK's Python
#     environment — it heals, because both are one command over files already committed
#     here and neither asks anything of the machine.
#   * What only the machine can supply — a Rust toolchain, bun, uv, the three cargo
#     subcommands the gate invokes — it refuses to guess at. Installing a toolchain from a
#     git hook is how a hook starts writing outside the tree it was asked about, so each
#     absent one is named with the command that installs it and nothing is attempted.
#
# Exit codes: 0 provisioned; 69 (EX_UNAVAILABLE) something is missing that this script must
# not install for you; 74 (EX_IOERR) provisioning was attempted and failed.
#
# Usage: scripts/provision-gate.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || {
  echo "provision-gate: could not resolve this repository's root from ${BASH_SOURCE[0]}" >&2
  echo "provision-gate: next: run it from a checkout of this repository" >&2
  exit 74
}
readonly ROOT
cd "$ROOT" || {
  echo "provision-gate: could not enter $ROOT" >&2
  echo "provision-gate: next: check that directory's permissions, then rerun" >&2
  exit 74
}

# Every absent prerequisite, one per line, as "<tool> — <how to install it>". Collected
# rather than reported at the first: somebody provisioning a machine wants the whole list,
# not one push per missing tool.
missing=""

require() {
  command -v "$1" >/dev/null 2>&1 || missing="${missing}${1} — $2
"
}

# The gate's own command surface. `just` is not here: the hook cannot invoke this script
# and then the gate without it, so it is the hook's own precondition and the hook says so.
require bun "install it from https://bun.sh — Nx orchestrates every target in this workspace"
require cargo "install the Rust toolchain from https://rustup.rs"
require uv "install it from https://docs.astral.sh/uv/ — it drives the Python SDK's targets"
require python3 "install Python 3; every check under scripts/ reads its data through it"
require cargo-llvm-cov "cargo binstall cargo-llvm-cov — 'just coverage' measures with it"
require cargo-deny "cargo binstall cargo-deny — 'just deny' is the supply-chain gate"
require cargo-machete "cargo binstall cargo-machete — 'just deny' runs it for unused dependencies"

if [ -n "$missing" ]; then
  echo "provision-gate: this worktree is missing what the gate needs, and installing a" >&2
  echo "provision-gate: toolchain is not something a git hook should do for you:" >&2
  printf '%s' "$missing" | sed 's/^/provision-gate:   /' >&2
  echo "provision-gate: next: install the tools above, then push again." >&2
  exit 69
fi

# The locked Node install, through the one script that owns it — scripts/nx.sh heals a
# worktree that has never had one and then runs Nx, so asking it for a version both
# provisions and proves the result works.
if ! nx_output="$(scripts/nx.sh --version 2>&1)"; then
  printf '%s\n' "$nx_output" >&2
  echo "provision-gate: the locked Node install did not produce a working Nx." >&2
  echo "provision-gate: next: run 'bun install' by hand and fix what it reports." >&2
  exit 74
fi

# The Python SDK's environment. Skipped once it exists, because `uv sync` reaches PyPI and
# a push should not depend on that once the worktree is provisioned. Both interpreter
# layouts are tested: uv writes `bin/` everywhere but Windows, where it writes `Scripts/`.
if [ ! -x sdks/python/.venv/bin/python ] && [ ! -f sdks/python/.venv/Scripts/python.exe ]; then
  if ! sync_output="$(uv sync --frozen --project sdks/python 2>&1)"; then
    printf '%s\n' "$sync_output" >&2
    echo "provision-gate: the Python SDK's locked environment did not sync." >&2
    echo "provision-gate: next: run 'uv sync --frozen --project sdks/python' and fix what it reports." >&2
    exit 74
  fi
fi
