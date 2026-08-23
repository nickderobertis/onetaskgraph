#!/usr/bin/env bash
# Print the name of every plugin crate, one per line.
#
# The single source of truth for "which crates are plugins": the `layer:plugin` tag on
# each crate's project.json. Two checks depend on this list — engine isolation and the
# affected selections — and a list hard-coded in each of them is a list a newly added
# plugin silently escapes, which would leave the new crate unchecked by exactly the two
# checks that exist to catch it.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 -c '
import json
import sys
from pathlib import Path

names = sorted(
    project["name"]
    for path in Path("crates").glob("*/project.json")
    for project in [json.loads(path.read_text())]
    if "layer:plugin" in project.get("tags", [])
)
if not names:
    print("plugin-crates: no crate is tagged layer:plugin — the tag is how the engine-",
          file=sys.stderr)
    print("plugin-crates: isolation and affected-selection checks find the plugins.",
          file=sys.stderr)
    raise SystemExit(1)
print("\n".join(names))
'
