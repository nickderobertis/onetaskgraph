#!/usr/bin/env bash
# Run the credential-gated live lane, for one named project or for every project.
#
# A project name is checked against the real set before it reaches Nx, so a typo says which
# names exist instead of failing somewhere inside the orchestrator — and so a value typed
# at this boundary cannot become part of a command by accident.
#
# Usage: scripts/test-live.sh [project ...]
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

known="$(./scripts/nx.sh show projects --json 2>/dev/null \
  | python3 -c 'import json,sys; print("\n".join(sorted(json.load(sys.stdin))))')"

if [ "$#" -eq 0 ]; then
  exec ./scripts/nx.sh run-many -t test-live --all
fi

for project in "$@"; do
  if ! printf '%s\n' "$known" | grep -qxF -- "$project"; then
    echo "test-live: $project is not a project of this workspace. Known projects:" >&2
    printf '  %s\n' $known >&2
    exit 1
  fi
done

exec ./scripts/nx.sh run-many -t test-live --projects="$(IFS=,; echo "$*")"
