#!/usr/bin/env bash
# SPIKE (serde-yaml-replacement, not for landing): the byte-for-byte suites the task names —
# rendered, priority, status_and_delivery, delivery, update and copy, in every crate that has
# one — run against a build whose typed YAML goes through serde-saphyr (`yaml-spike` defaults
# to it, which matters because the e2e sandbox strips every ONETASKGRAPH_* variable), and then
# every test that failed is run again on today's serde_norway build, so a failure is
# attributed to the crate swap only when it passes there.
#
#   spikes/saphyr-diff/journeys.sh <out-dir>
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
out="${1:?usage: journeys.sh <out-dir>}"
mkdir -p "$out"
cd "$repo"

spike="--features onetaskgraph-core/yaml-spike"
suites=(
  "onetaskgraph-e2e|--test e2e|rendered:: priority:: delivery:: update:: copy::"
  "onetaskgraph-core|--test rendered --test priority --test delivery --test update --test copy|"
  "onetaskgraph-local-md|--test priority --test status_and_delivery|"
)

# $1 label, $2 extra cargo flags (the feature, or nothing).
run_all() {
  local label="$1" flags="$2" failed="$out/journeys-$1.failed"
  : >"$failed"
  # The one binary the e2e journeys spawn, built as onetaskgraph:build builds it.
  # shellcheck disable=SC2086
  cargo test --quiet -p onetaskgraph --all-features $flags --locked --no-run >"$out/journeys-$label-build.log" 2>&1 \
    || { echo "journeys: the $label binary did not build; see $out/journeys-$label-build.log" >&2; return 1; }
  for suite in "${suites[@]}"; do
    IFS='|' read -r package targets filters <<<"$suite"
    local log="$out/journeys-$label-$package.log"
    # shellcheck disable=SC2086
    cargo test -p "$package" $flags --locked $targets -- $filters >"$log" 2>&1
    grep -E '^test .* \.\.\. FAILED$' "$log" | sed -E 's/^test (.*) \.\.\. FAILED$/\1/' \
      | while IFS= read -r name; do printf '%s|%s|%s\n' "$package" "$targets" "$name"; done >>"$failed"
    printf 'journeys (%s) %s: %s\n' "$label" "$package" "$(grep -E '^test result:' "$log" | awk '{p+=$4; f+=$6} END {print p" passed, "f" failed"}')"
  done
}

run_all saphyr "$spike"
if [ ! -s "$out/journeys-saphyr.failed" ]; then
  echo "journeys: every named suite passes on serde-saphyr"
  exit 0
fi
echo "journeys: $(wc -l <"$out/journeys-saphyr.failed") tests fail on serde-saphyr; running those on serde_norway"

# Today's build, and only the tests that failed above, each by its exact name.
cargo test --quiet -p onetaskgraph --all-features --locked --no-run >"$out/journeys-norway-build.log" 2>&1
: >"$out/journeys-norway.failed"
while IFS='|' read -r package targets name; do
  # shellcheck disable=SC2086
  if cargo test -p "$package" --locked $targets -- --exact "$name" >"$out/journeys-norway-one.log" 2>&1 \
    && grep -qE '^test result: ok\. 1 passed' "$out/journeys-norway-one.log"; then
    printf 'journeys: FAILS on serde-saphyr, passes on serde_norway: %s %s\n' "$package" "$name"
  else
    printf 'journeys: fails on both (not the swap): %s %s\n' "$package" "$name"
    printf '%s|%s|%s\n' "$package" "$targets" "$name" >>"$out/journeys-norway.failed"
  fi
done <"$out/journeys-saphyr.failed"
echo "journeys: logs in $out/journeys-*.log"
