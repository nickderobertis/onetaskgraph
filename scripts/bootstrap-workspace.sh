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
# llmlint: ignore[changed_behavior_has_e2e] Reaching this bootstrap refusal requires replacing the authoritative workflow pin; the release checks mutate and reject pin drift without making session setup install an invented version.
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

# The canonical release-target reader, warmed into the uv cache at the version
# scripts/check-release-targets.sh pins. That check runs it with `--offline`, because a
# required check does not reach the network — so somebody has to have fetched it once,
# and this is that once. The pin is READ from the check rather than restated, the way the
# release-plz pin above is read from the workflow that owns it.
reader_package="$(sed -n 's/^readonly READER_PACKAGE="\([^"]*\)"$/\1/p' scripts/check-release-targets.sh | head -n1)"
reader_version="$(sed -n 's/^readonly READER_VERSION="\([^"]*\)"$/\1/p' scripts/check-release-targets.sh | head -n1)"
if [ -z "$reader_package" ] || [ -z "$reader_version" ]; then
  echo "bootstrap-workspace: scripts/check-release-targets.sh no longer spells the canonical reader's pin where this can read it" >&2
  echo "bootstrap-workspace: next: restore the READER_PACKAGE and READER_VERSION lines in that script, then rerun 'just bootstrap'" >&2
  exit 1
fi
# A failure here is reported and does not stop bootstrap, for the same reason the judged
# tier's is below: this needs the network, and `just check` still runs without it — the
# reader check falls back to a capable onevcs on PATH and, failing that, says it could not
# resolve one rather than passing quietly.
# llmlint: ignore-block[changed_behavior_has_e2e] Its three paths are the presence of uv
# and the reachability of PyPI, both host state outside this repository — the same reason
# the release-plz installer above carries this directive, and the same remedy: what the
# warm is FOR is verified where it is used, by scripts/check-release-targets.sh resolving
# the pinned reader offline and refusing to pass on an incapable one.
if command -v uvx >/dev/null 2>&1; then
  if ! warm_output="$(uvx --from "$reader_package==$reader_version" onevcs --version 2>&1)"; then
    echo "bootstrap-workspace: $reader_package $reader_version did not fetch (${warm_output##*$'\n'}); 'just check' still runs, and next action is to fix that and rerun 'just bootstrap'" >&2
  fi
else
  echo "bootstrap-workspace: uv is not installed, so $reader_package $reader_version was not warmed; 'just check' still runs, and next action is to install uv and rerun 'just bootstrap'" >&2
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
