#!/usr/bin/env bash
# Fail when two sources spell the metadata slot's delimiters differently.
#
# `docs/metadata.md` settles one encoding for caller metadata that a backend has no field
# for: a canonical-JSON `<!-- onetaskgraph.metadata ... -->` comment in the item's own
# free text. Linear puts it in the description and github-projects at the end of the issue
# body, and that is one encoding used twice rather than two — a second encoding is the
# thing the document exists to prevent.
#
# Neither can import the other's constants: a plugin crate depends on the contract crate
# and nothing else of this workspace. So each restates the delimiters, and this reconciles
# them. Drift is quiet — each source round-trips its own writes perfectly well under its
# own spelling — so nothing else in the gate would notice one document describing two
# encodings.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 <<'PY'
import pathlib
import re
import sys


def fail(problem, action):
    """A named problem and a concrete next action, which is all a guard owes its reader."""
    print(f"check-metadata-slot-encoding: {problem}", file=sys.stderr)
    print(f"check-metadata-slot-encoding: {action}", file=sys.stderr)
    sys.exit(1)


# Forward slashes on every platform: python renders a path with the running platform's
# separator, and a guard that names `crates\...` on one runner cannot be asserted against.
spellings = {}
for source in sorted(pathlib.Path("crates").glob("*/src/**/*.rs")):
    try:
        text = source.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        fail(
            f"could not read {source.as_posix()}: {error}",
            "restore that file as UTF-8 Rust source — this check cannot tell one slot "
            "encoding from another it cannot read — then re-run 'just check'.",
        )
    found = {
        name: literal
        for name, literal in re.findall(
            r'const\s+(METADATA_OPEN|METADATA_CLOSE)\s*:[^=]*=\s*"((?:[^"\\]|\\.)*)"', text
        )
    }
    if found:
        spellings[source.as_posix()] = found

if len(spellings) < 2:
    fail(
        f"only {len(spellings)} source spells the metadata slot, so there is nothing to "
        "reconcile and this check has stopped watching what it was written for",
        "point it at wherever the delimiters moved, or delete it in the same change that "
        "leaves one spelling of them.",
    )

agreed = sorted(spellings.items())[0]
disagreeing = [
    (path, found) for path, found in sorted(spellings.items()) if found != agreed[1]
]
if disagreeing:
    for path, found in disagreeing:
        print(
            f"check-metadata-slot-encoding: {path} spells the slot {found}, and "
            f"{agreed[0]} spells it {agreed[1]}",
            file=sys.stderr,
        )
    fail(
        "one metadata slot encoding is spelled two ways",
        "bring them to one spelling — docs/metadata.md settles a single encoding for "
        "every source that needs the slot, and two of them is the thing that document "
        "exists to prevent.",
    )
PY
