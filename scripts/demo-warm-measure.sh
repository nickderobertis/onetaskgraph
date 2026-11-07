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
toolchain="$(dirname "$(rustup which cargo)")"
bash scripts/pr-build-commands.sh | grep -E '^cargo (clippy|check) ' >"$RUNNER_TEMP/lint-lines"
run_lines() { local line; while IFS= read -r line; do $line </dev/null; done <"$RUNNER_TEMP/lint-lines"; }
report "lint compile lines: $(wc -l <"$RUNNER_TEMP/lint-lines")"
report "warm --instrumented over main's family: $(seconds bash scripts/warm-rust-cache.sh --instrumented) s"
report "built target before cleanup: $(du -sh target | cut -f1)"
node scripts/demo-archive-size.mjs | grep -E '^key |Saving|Failed to save|File Size|Cache Size|Archive' | sed 's/^/demo|save: /' || true
export PATH="$toolchain:$PATH"
report "cleaned target: $(du -sh target | cut -f1)"
report "the same tar and zstd over the cleaned tree, for comparison: $(archive) bytes"
