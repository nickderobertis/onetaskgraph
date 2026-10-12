#!/usr/bin/env bash
# SPIKE (serde-yaml-replacement, not for landing): the one command that runs the differential
# harness and prints every difference between serde_norway and serde-saphyr, by document kind.
#
#   spikes/saphyr-diff/run.sh            # the harness over real documents and synthetic cases,
#                                        # once per serde-saphyr option profile (default, compat)
#   spikes/saphyr-diff/run.sh --journeys # and the byte-for-byte suites on a serde-saphyr build
#   spikes/saphyr-diff/run.sh --deny     # and cargo-deny / cargo-tree over the candidates
#   SAPHYR_DIFF_ONLY=front-matter spikes/saphyr-diff/run.sh   # the template walks alone
#
# Real documents are read only: the plan store is copied into the scratch directory first, and
# every write lands in a copy. Inputs default to this host's; override with the variables below.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
store="${SAPHYR_DIFF_STORE:-$HOME/projects/ai-orchestrator/.plans}"
scan="${SAPHYR_DIFF_SCAN:-$HOME/projects}"
scratch="${SAPHYR_DIFF_SCRATCH:-${ONEPIPELINE_NODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/saphyr-diff.$$}"
out="${SAPHYR_DIFF_OUT:-$scratch/out}"
mkdir -p "$scratch" "$out"

# Unique content only: a file copied into many run directories is one document.
dedupe() { while IFS= read -r f; do printf '%s %s\n' "$(sha256sum "$f" | cut -c1-64)" "$f"; done | sort -u -k1,1 | cut -d' ' -f2-; }

{ find "$scan" -maxdepth 2 -name onetaskgraph.yaml -not -path "*/node_modules/*" 2>/dev/null || true; } >"$scratch/configs.raw"
find "$repo/screenshots" -name onetaskgraph.yaml >>"$scratch/configs.raw"
dedupe <"$scratch/configs.raw" >"$scratch/configs.txt"
{ find "$scan" -name "*.j2" -not -path "*/node_modules/*" -not -path "*/target/*" 2>/dev/null || true; } \
  | while IFS= read -r f; do if [ "$(head -1 "$f")" = "---" ]; then printf '%s\n' "$f"; fi; done \
  | dedupe >"$scratch/templates.txt"
{ find "$scan" -name "*answers*.y*ml" -not -path "*/node_modules/*" -not -path "*/target/*" 2>/dev/null || true; } \
  | dedupe >"$scratch/answers.txt"
printf 'inputs: %s configurations, %s templates, %s answers files (unique by content); store %s\n' \
  "$(wc -l <"$scratch/configs.txt")" "$(wc -l <"$scratch/templates.txt")" "$(wc -l <"$scratch/answers.txt")" "$store"

cd "$repo"
# Every profile: `default` is serde-saphyr as it ships; `compat` sets the options that
# reproduce today's behaviour where an option can (src/main.rs names each and why).
cargo build -q --release -p saphyr-diff
# serde_json with `preserve_order`, built as a workspace of its own so the feature cannot
# unify into the build above; the harness refuses its answers unless the feature is on there.
cargo build -q --release --locked --manifest-path "$here/../saphyr-ordered/Cargo.toml"
if cargo tree -q --all-features -e features -i serde_json | grep -q preserve_order; then
  echo "the workspace build has serde_json preserve_order; next: find what turned it on" >&2
  exit 1
fi
echo "baseline: this workspace's serde_json is built without preserve_order"
# SAPHYR_DIFF_ONLY=front-matter re-measures that kind alone.
only=()
if [ -n "${SAPHYR_DIFF_ONLY:-}" ]; then only=(--only "$SAPHYR_DIFF_ONLY"); fi
for profile in ${SAPHYR_DIFF_PROFILES:-default compat}; do
  mkdir -p "$out/$profile"
  "$repo/target/release/saphyr-diff" --profile "$profile" \
    --store "$store" --scratch "$scratch/work-$profile" --out "$out/$profile" \
    --configs "$scratch/configs.txt" --templates "$scratch/templates.txt" --answers "$scratch/answers.txt" \
    --ordered "$repo/target/release/saphyr-ordered" ${only[@]+"${only[@]}"}
  echo "report ($profile): $out/$profile/report.txt, $out/$profile/diffs.json"
done

for flag in "$@"; do
  case "$flag" in
    --journeys) "$here/journeys.sh" "$out" ;;
    --deny) "$here/deny.sh" "$out" ;;
    *) echo "unknown flag $flag; next: use --journeys and/or --deny" >&2; exit 2 ;;
  esac
done
