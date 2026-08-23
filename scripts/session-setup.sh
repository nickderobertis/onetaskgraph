#!/usr/bin/env bash
# Idempotent, non-blocking session provisioner, wired into Claude Code's
# SessionStart hook (.claude/settings.json). Its job is to make a fresh web/cloud
# session able to run the documented `just` command surface with no manual steps.
#
# What it does, and why:
#   1. Ensures `just` — the entry point to every recipe (`just check`, ...). A
#      cloud session's image may ship your language toolchain but not `just`, and
#      there is no asdf there to read `.tool-versions`, so without this the very
#      first `just ...` call fails. `rust-just` packages the prebuilt `just`
#      binary on PyPI, so `uv tool install` fetches it with no Rust toolchain and
#      no github.com reachability (works in restricted-egress sessions where PyPI
#      is reachable). If your repo has no uv, install `just` your toolchain's way.
#   2. Verifies the rest of the toolchain and logs an actionable pointer for
#      anything missing rather than attempting an install a startup hook can't do
#      reliably — edit the `verify_prereqs` list to your stack's tools.
#   3. Hands off to the optional llmlint-tier installer (setup-llmlint.sh) if you
#      bundle one, kept separate so `just setup-llmlint` can run it standalone.
#
# CI provisions the toolchain itself (asdf/.tool-versions, setup-* actions), so
# this no-ops there. Every step tolerates failure and the script always exits 0 —
# a flaky install must never abort session startup. Also safe to run by hand.
# llmlint: ignore-file[tool_output_is_signal, boundary_inputs_validated] deliberate for a session-startup installer (see header): every step logs and continues rather than blocking startup, so a flaky install can't abort the hook; and `just` is installed from PyPI (`uv tool install rust-just`) whose wheels ship with Trusted Publishing + PEP 740 attestations, so no unvalidated external input is executed.
set -uo pipefail

# `just` floor — keep in lockstep with your `.tool-versions` pin. `rust-just` is
# the PyPI package that ships the `just` binary.
readonly JUST_MIN="1.51.0"
readonly BIN_DIR="$HOME/.local/bin"
# Capture the inherited PATH before we prepend BIN_DIR, so persist_session_env can
# tell whether BIN_DIR was already resolvable and only override PATH when it isn't.
readonly ORIG_PATH="${PATH}"

log() { printf 'session-setup: %s\n' "$*" >&2; }

# CI has its own provisioning; skip there rather than racing it. The SessionStart
# hook does not fire in CI — this is a guard for a hand/`just` run on a CI runner.
if [ -n "${CI:-}" ]; then
  log "CI detected; skipping (toolchain provisioned by the CI workflow)"
  exit 0
fi

export PATH="${BIN_DIR}:${PATH}"

# Install `just` via uv unless it already resolves. Swap this for your toolchain's
# installer if the repo has no uv (e.g. `npm i -g rust-just`, a release download).
ensure_just() {
  if command -v just >/dev/null 2>&1; then
    log "just present ($(just --version 2>/dev/null || echo unknown))"
    return 0
  fi
  if ! command -v uv >/dev/null 2>&1; then
    log "uv not found; cannot install just (install uv: https://docs.astral.sh/uv/)"
    return 0
  fi
  log "installing rust-just >= $JUST_MIN via uv tool"
  uv tool install --upgrade "rust-just>=$JUST_MIN" >&2 \
    || log "rust-just install failed (continuing)"
}

# Verify the rest of the toolchain and point at what's missing. This workspace is
# polyglot: uv drives the Python project, cargo the seven crates, bun both the
# TypeScript SDK and Nx itself (Nx orchestrates every language's targets).
verify_prereqs() {
  local tool
  for tool in uv cargo bun; do
    command -v "$tool" >/dev/null 2>&1 \
      || log "$tool not on PATH (normally provided by the cloud image or asdf/.tool-versions)"
  done
}

# Persist PATH so the freshly installed `just` resolves in every later Bash call.
# Only write an override when BIN_DIR was not already on the inherited PATH.
persist_session_env() {
  [ -n "${CLAUDE_ENV_FILE:-}" ] || { log "no CLAUDE_ENV_FILE (not a session); skipping env"; return 0; }
  case ":${ORIG_PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *) printf 'export PATH=%q\n' "${BIN_DIR}:${PATH}" >> "$CLAUDE_ENV_FILE"
       log "prepended ${BIN_DIR} to PATH for the session";;
  esac
}

ensure_just
verify_prereqs
persist_session_env

# Hand off to the optional llmlint-tier installer beside this script, if present.
setup_llmlint="$(dirname "$0")/setup-llmlint.sh"
if [ -x "$setup_llmlint" ]; then
  log "running setup-llmlint.sh"
  "$setup_llmlint" || log "setup-llmlint.sh reported an issue (continuing)"
fi

exit 0
