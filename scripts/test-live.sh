#!/usr/bin/env bash
# Run the credential-gated live lane, for one named project or for every project.
#
# A project name is checked against the real set before it reaches Nx, so a typo says which
# names exist instead of failing somewhere inside the orchestrator — and so a value typed
# at this boundary cannot become part of a command by accident.
#
# Usage: scripts/test-live.sh [project ...]
# llmlint: ignore-file[live_tier_compiles_and_requires_credential] empty live lane passes by design
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

stderr_file="$(mktemp)"
validation_file="$(mktemp)"
trap 'rm -f "$stderr_file" "$validation_file"' EXIT

project_list_failure() {
  local status="$1"
  echo "test-live: './scripts/nx.sh show projects --json' could not list this workspace's projects." >&2
  echo "test-live: stdout:" >&2
  if [ -n "$raw" ]; then
    printf '%s\n' "$raw" >&2
  else
    echo "<empty>" >&2
  fi
  echo "test-live: stderr:" >&2
  if [ -s "$stderr_file" ]; then
    cat "$stderr_file" >&2
  else
    echo "<empty>" >&2
  fi
  if [ -s "$validation_file" ]; then
    echo "test-live: validation:" >&2
    cat "$validation_file" >&2
  fi
  echo "test-live: run 'just bootstrap', then retry the command above." >&2
  exit "$status"
}

if ! raw="$(./scripts/nx.sh show projects --json 2>"$stderr_file")"; then
  # Status 1 means Nx could not produce an answer at all.
  project_list_failure 1
fi
if ! known="$(printf '%s' "$raw" | python3 -c '
import json
import sys

try:
    projects = json.load(sys.stdin)
except json.JSONDecodeError as error:
    print(f"project output is not valid JSON: {error}", file=sys.stderr)
    sys.exit(1)
if not isinstance(projects, list):
    print("project output is not a JSON array", file=sys.stderr)
    sys.exit(1)
if not projects:
    print("project output is an empty JSON array", file=sys.stderr)
    sys.exit(1)
if not all(
    isinstance(item, str)
    and item
    and "," not in item
    and not any(ord(character) < 32 or ord(character) == 127 for character in item)
    for item in projects
):
    print(
        "project output contains a non-string, empty, delimited, or control-character project name",
        file=sys.stderr,
    )
    sys.exit(1)
print("\n".join(sorted(projects)))
' 2>"$validation_file")"; then
  # Status 2 means Nx succeeded, but its answer violated the project-list contract.
  project_list_failure 2
fi

rm -f "$stderr_file" "$validation_file"
trap - EXIT

if [ "$#" -eq 0 ]; then
  # nx.sh buffers run-many output and emits one command line on success.
  exec ./scripts/nx.sh run-many -t test-live --all
fi

for project in "$@"; do
  if ! printf '%s\n' "$known" | grep -qxF -- "$project"; then
    echo "test-live: $project is not a project of this workspace. Known projects:" >&2
    while IFS= read -r known_project; do
      printf '  %s\n' "$known_project" >&2
    done <<< "$known"
    exit 1
  fi
done

# nx.sh buffers run-many output and emits one command line on success.
exec ./scripts/nx.sh run-many -t test-live --projects="$(IFS=,; echo "$*")"
