#!/usr/bin/env bash
# SPIKE (serde-yaml-replacement, not for landing): the byte-for-byte suites the task names —
# rendered, priority, status_and_delivery, delivery, update and copy, in every crate that has
# one — run against a build whose typed YAML goes through serde-saphyr (`yaml-spike` defaults
# to it, which matters because the e2e sandbox strips every ONETASKGRAPH_* variable), and then
# every test that failed is run again on today's serde_norway build, so a failure is
# attributed to the crate swap only when it passes there. Once per profile: `default`
# (`yaml-spike`, serde-saphyr's own options) and `compat` (`yaml-spike-compat`, the options
# that reproduce serde_norway where one can).
#
#   spikes/saphyr-diff/journeys.sh <out-dir>
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
out="${1:?usage: journeys.sh <out-dir>}"
mkdir -p "$out"
cd "$repo"

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
    local log="$out/journeys-$label-$package.log" own="$flags"
    # local-md does not depend on core: it takes the feature by its own name.
    if [ "$package" = onetaskgraph-local-md ]; then own="${flags//onetaskgraph-core\//}"; fi
    # shellcheck disable=SC2086
    cargo test -p "$package" $own --locked $targets -- $filters >"$log" 2>&1
    grep -E '^test .* \.\.\. FAILED$' "$log" | sed -E 's/^test (.*) \.\.\. FAILED$/\1/' \
      | while IFS= read -r name; do printf '%s|%s|%s\n' "$package" "$targets" "$name"; done >>"$failed"
    printf 'journeys (%s) %s: %s\n' "$label" "$package" "$(grep -E '^test result:' "$log" | awk '{p+=$4; f+=$6} END {print p" passed, "f" failed"}')"
  done
}

# $1 profile label, $2 the feature that selects it.
journeys() {
  local label="$1" flags="--features onetaskgraph-core/$2"
  run_all "$label" "$flags"
  if [ ! -s "$out/journeys-$label.failed" ]; then
    echo "journeys ($label): every named suite passes on serde-saphyr"
    return 0
  fi
  echo "journeys ($label): $(wc -l <"$out/journeys-$label.failed") tests fail on serde-saphyr; running those on serde_norway"
  # Today's build, and only the tests that failed above, each by its exact name.
  cargo test --quiet -p onetaskgraph --all-features --locked --no-run >"$out/journeys-norway-build.log" 2>&1
  while IFS='|' read -r package targets name; do
    # shellcheck disable=SC2086
    if cargo test -p "$package" --locked $targets -- --exact "$name" >"$out/journeys-norway-one.log" 2>&1 \
      && grep -qE '^test result: ok\. 1 passed' "$out/journeys-norway-one.log"; then
      printf 'journeys (%s): FAILS on serde-saphyr, passes on serde_norway: %s %s\n' "$label" "$package" "$name"
    else
      printf 'journeys (%s): fails on both (not the swap): %s %s\n' "$label" "$package" "$name"
    fi
  done <"$out/journeys-$label.failed"
}

journeys default yaml-spike
journeys compat yaml-spike-compat
echo "journeys: logs in $out/journeys-*.log"
