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

names = []
for path in sorted(Path("crates").glob("*/project.json")):
    try:
        project = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        # Named, with the cause: every caller of this script is a guard, and a guard that
        # dies on a traceback tells its reader nothing about which file to open.
        print(f"plugin-crates: could not read {path}: {error}", file=sys.stderr)
        print("plugin-crates: fix that file — it is a project.json and must be valid JSON.",
              file=sys.stderr)
        raise SystemExit(1) from None
    if "layer:plugin" in project.get("tags", []):
        names.append(project["name"])
names = sorted(names)
if not names:
    print("plugin-crates: no crate is tagged layer:plugin — the tag is how the engine-",
          file=sys.stderr)
    print("plugin-crates: isolation and affected-selection checks find the plugins.",
          file=sys.stderr)
    raise SystemExit(1)
print("\n".join(names))
'
