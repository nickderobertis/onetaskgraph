#!/usr/bin/env bash
# DO NOT MERGE. See .github/workflows/demo-warm.yml. Every report line starts `demo|`.
set -euo pipefail
report() { echo "demo|$*"; }
seconds() { local s=$SECONDS; "$@" >"$RUNNER_TEMP/step.log" 2>&1 || { tail -40 "$RUNNER_TEMP/step.log"; return 1; }; echo $((SECONDS - s)); }
report "restored target: $(du -sh target 2>/dev/null | cut -f1)"
lint_lines() { bash scripts/pr-build-commands.sh | grep -E '^cargo (clippy|check) ' ; }
run_lines() { local line; while IFS= read -r line; do $line </dev/null; done < "$1"; }
lint_lines >"$RUNNER_TEMP/lint-lines"
report "lint compile lines: $(wc -l <"$RUNNER_TEMP/lint-lines")"
report "lint lines cold, serial, empty target: $(CARGO_TARGET_DIR="$RUNNER_TEMP/cold" seconds run_lines "$RUNNER_TEMP/lint-lines") s"
rm -rf "$RUNNER_TEMP/cold"
report "warm --instrumented over main's family: $(seconds bash scripts/warm-rust-cache.sh --instrumented) s"
report "reuse check after warm: $(seconds just warm-cache-reuse-check-warmed) s"
report "built target before cleanup: $(du -sh target | cut -f1)"
df -h / | tail -1 | sed 's/^/demo|disk: /'
node scripts/demo-archive-size.mjs | tee "$RUNNER_TEMP/archive.log" | grep -E 'Cache Size|Cleaning|key' | sed 's/^/demo|save: /' || true
report "cleaned target: $(du -sh target | cut -f1)"
report "lint lines over the cleaned tree, serial, as a pull request restores it: $(seconds run_lines "$RUNNER_TEMP/lint-lines") s"
