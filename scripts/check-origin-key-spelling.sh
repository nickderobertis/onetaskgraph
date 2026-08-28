#!/usr/bin/env bash
# Fail when a plugin spells the copy origin's metadata key differently from the engine.
#
# The engine owns that key and spells it once, as `GlobalId::ORIGIN_KEY`. A plugin cannot
# import it: no plugin crate may depend on the engine, at any depth, which is the whole
# reason `scripts/check-plugin-isolation.sh` exists. So a source that routes the key —
# `github-projects` keeps it in a board text field of the same name — has to restate the
# literal, and a restated contract with nothing reconciling it drifts in silence: the
# engine writes one key, the plugin reads another, and the only symptom is a copy that
# creates a second item every run instead of finding the one it made last time.
#
# This reconciles them. Every `onetaskgraph.`-prefixed literal in a plugin crate that
# names an origin must be the engine's own spelling of it.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

AUTHORITY="$ROOT/crates/onetaskgraph-core/src/global_id.rs" \
python3 <<'PY'
import os
import pathlib
import re
import sys

authority_path = os.environ["AUTHORITY"]


def fail(problem, action):
    """A named problem and a concrete next action, which is all a guard owes its reader."""
    print(f"check-origin-key-spelling: {problem}", file=sys.stderr)
    print(f"check-origin-key-spelling: {action}", file=sys.stderr)
    sys.exit(1)


try:
    authority_source = open(authority_path, encoding="utf-8").read()
except (OSError, UnicodeDecodeError) as error:
    fail(
        f"could not read the engine's own spelling: {error}",
        f"restore {authority_path} — it is where `GlobalId::ORIGIN_KEY` is defined — "
        "then re-run 'just check'.",
    )

found = re.search(
    r'ORIGIN_KEY:\s*&\'static\s+str\s*=\s*"([^"]+)"',
    authority_source,
)
if found is None:
    fail(
        f"{authority_path} defines no `ORIGIN_KEY` literal",
        "spell it as `pub const ORIGIN_KEY: &'static str = \"...\";` there, or update this "
        "check to read it wherever it moved.",
    )
authority = found.group(1)

# Forward slashes on every platform: python renders a path with the running platform's
# separator, and a guard that names `crates\...` on one runner cannot be asserted against.
problems = []
for crate in sorted(pathlib.Path("crates").iterdir()):
    if not crate.is_dir() or crate.name == "onetaskgraph-core":
        continue
    for source in sorted(crate.glob("src/**/*.rs")):
        for number, line in enumerate(
            source.read_text(encoding="utf-8").splitlines(), start=1
        ):
            # Two ways to say the same thing, because either alone has a hole. A
            # literal naming an origin catches the engine renaming the key while a
            # plugin keeps the old spelling; a constant whose own name says origin
            # catches a plugin renaming the key the engine still writes.
            for literal in re.findall(r'"(onetaskgraph\.[a-z_.]+)"', line):
                if "origin" in literal and literal != authority:
                    problems.append(
                        f"{source.as_posix()}:{number} spells {literal!r}, and the "
                        f"engine's `GlobalId::ORIGIN_KEY` is {authority!r}"
                    )
            declared = re.search(r'const\s+(\w*ORIGIN\w*)\s*:[^=]*=\s*"([^"]*)"', line)
            if declared is not None and declared.group(2) != authority:
                problems.append(
                    f"{source.as_posix()}:{number} defines {declared.group(1)} as "
                    f"{declared.group(2)!r}, and the engine's `GlobalId::ORIGIN_KEY` is "
                    f"{authority!r}"
                )

if problems:
    for problem in problems:
        print(f"check-origin-key-spelling: {problem}", file=sys.stderr)
    fail(
        "a plugin names the copy origin under a key the engine does not write",
        "bring each spelling above to the engine's, or move the key and every spelling of "
        "it in the same change — a copy cannot find what it wrote under another name.",
    )
PY
