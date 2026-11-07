#!/usr/bin/env bash
# DO NOT MERGE. See .github/workflows/demo-warm.yml. Every report line starts `demo|`.
set -euo pipefail
report() { echo "demo|$*"; }
seconds() { local s=$SECONDS; "$@" >"$RUNNER_TEMP/step.log" 2>&1 || { tail -40 "$RUNNER_TEMP/step.log"; return 1; }; echo $((SECONDS - s)); }
# The archive actions/cache makes of rust-cache's paths: its own tar and zstd flags.
archive() {
  local paths=(target "$HOME/.cargo/registry/index" "$HOME/.cargo/registry/cache" "$HOME/.cargo/git/db" "$HOME/.cargo/bin")
  local present=() path
  for path in "${paths[@]}"; do [ -e "$path" ] && present+=("$path"); done
  tar --posix -cf - -P "${present[@]}" | zstd -T0 --long=30 | wc -c
}
report "restored target: $(du -sh target | cut -f1)"
report "archive of the restored family, calibrating against main's listed 1306075170: $(archive) bytes"
toolchain="$(dirname "$(rustup which cargo)")"
bash scripts/pr-build-commands.sh | grep -E '^cargo (clippy|check) ' >"$RUNNER_TEMP/lint-lines"
run_lines() { local line; while IFS= read -r line; do $line </dev/null; done <"$RUNNER_TEMP/lint-lines"; }
report "lint compile lines: $(wc -l <"$RUNNER_TEMP/lint-lines")"
report "lint lines cold, serial, empty target: $(CARGO_TARGET_DIR="$RUNNER_TEMP/cold" seconds run_lines) s"
rm -rf "$RUNNER_TEMP/cold"
report "warm --instrumented over main's family: $(seconds bash scripts/warm-rust-cache.sh --instrumented) s"
report "reuse check after warm: $(seconds just warm-cache-reuse-check-warmed) s"
report "built target before cleanup: $(du -sh target | cut -f1)"
node scripts/demo-archive-size.mjs | grep -E '^key |Cleaning |Saving|Failed to save' | sed 's/^/demo|save: /' || true
export PATH="$toolchain:$PATH"
report "cleaned target: $(du -sh target | cut -f1)"
report "archive this leg would save, after rust-cache's own cleanup: $(archive) bytes"
report "lint lines over the cleaned tree, serial, as a pull request restores it: $(seconds run_lines) s"
