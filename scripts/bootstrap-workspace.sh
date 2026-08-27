#!/usr/bin/env bash
# Workspace-level setup a clean clone needs before any target can run.
#
# Activates the repository's own git hooks (so `git push` runs the complete gate rather
# than discovering it in CI) and installs the judged-lint toolchain. Everything here is
# idempotent, so `just bootstrap` is safe to re-run.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Point git at the tracked hooks. Without this the pre-push gate is a file nobody runs.
git config core.hooksPath .githooks

release_plz_pin="$(sed -n 's/.*release-plz@\([^ ,]*\).*/\1/p' .github/workflows/release-plz.yml | head -n1)"
[[ $release_plz_pin =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "bootstrap-workspace: the release workflow has no exact X.Y.Z release-plz pin ('$release_plz_pin')" >&2
  echo "bootstrap-workspace: next: restore the exact release-plz pin in .github/workflows/release-plz.yml" >&2
  exit 1
}
release_plz_version="$(release-plz --version 2>/dev/null || true)"
# llmlint: ignore-block[changed_behavior_has_e2e] Exercising either installer branch requires removing or replacing a host tool outside the repository; the real preparation check verifies the installed result and the workflow-pin drift gate verifies its source.
if [ "$release_plz_version" != "release-plz $release_plz_pin" ]; then
  install_output=""
  if command -v cargo-binstall >/dev/null 2>&1; then
    install_output="$(cargo binstall release-plz --version "$release_plz_pin" --no-confirm 2>&1)" || {
      printf '%s\n' "$install_output" >&2
      echo "bootstrap-workspace: release-plz $release_plz_pin installation failed" >&2
      echo "bootstrap-workspace: next: fix the installer diagnostic above and rerun 'just bootstrap'" >&2
      exit 1
    }
  else
    install_output="$(cargo install release-plz --version "$release_plz_pin" --locked 2>&1)" || {
      printf '%s\n' "$install_output" >&2
      echo "bootstrap-workspace: release-plz $release_plz_pin installation failed" >&2
      echo "bootstrap-workspace: next: fix the installer diagnostic above and rerun 'just bootstrap'" >&2
      exit 1
    }
  fi
fi
# llmlint: ignore-end[changed_behavior_has_e2e]

# The judged tier is out of `just check`, but a contributor should not have to discover
# how to install it. Through the documented recipe, so there is one way to do it.
#
# A failure here is reported and does not stop bootstrap, deliberately: this tier needs the
# network and a harness, `just check` and `just gate` do not use it, and a contributor who
# cannot install it must still be able to build and test. The `llmlint` PR check is what
# catches its absence for a change that is going to merge.
if ! setup_output="$(just setup-llmlint 2>&1)"; then
  printf '%s\n' "$setup_output" >&2
  echo "bootstrap-workspace: the judged-lint tier did not install (see above). 'just check'" >&2
  echo "bootstrap-workspace: and 'just gate' are unaffected; 'just lint-llm*' will not run." >&2
  echo "bootstrap-workspace: next action: fix the cause above, then 'just setup-llmlint'." >&2
fi
