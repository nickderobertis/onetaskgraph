#!/usr/bin/env bash
# Fail when two sources spell the metadata slot's delimiters differently.
#
# `docs/metadata.md` settles one slot for caller metadata that a backend has no field for: a
# canonical-JSON `<!-- onetaskgraph.metadata ... -->` comment at the end of the item's own
# free text. Linear puts it in the description and github-projects at the end of the issue
# body. The slot has two spellings and no more: the multi-line one, which every source that
# keeps a slot reads, and the one-line code span, which a source writes when its host does
# not keep the multi-line one byte for byte — Linear, whose Markdown normalization rewrites
# JSON inside an HTML comment everywhere but a code span (observed 2026-10-02 and recorded in
# that document). Each source writes the spelling its host preserves; a third spelling, or one
# of these two spelled differently by two sources, is the thing the document exists to prevent.
#
# Neither can import the other's constants: a plugin crate depends on the contract crate
# and nothing else of this workspace. So each restates the delimiters, and this reconciles
# them: every source spells the multi-line pair, `METADATA_OPEN` and `METADATA_CLOSE`, and
# spells it the same; a source spelling the code-span pair, `METADATA_OPEN_SPAN` and
# `METADATA_CLOSE_SPAN`, spells both, the same as any other that does, opening with the same
# marker as the multi-line pair. Drift is quiet — each source round-trips its own writes
# perfectly well under its own spelling — so nothing else in the gate would notice one
# document describing two encodings.
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
            r'const\s+(METADATA_OPEN(?:_SPAN)?|METADATA_CLOSE(?:_SPAN)?)\s*:[^=]*=\s*"((?:[^"\\]|\\.)*)"',
            text,
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

# Every source reads the multi-line spelling, so every one spells that pair.
for path, found in sorted(spellings.items()):
    missing = [name for name in ("METADATA_OPEN", "METADATA_CLOSE") if name not in found]
    if missing:
        fail(
            f"{path} spells the metadata slot without {', '.join(missing)}, the multi-line "
            "spelling every source that keeps a slot reads",
            "restore it there — docs/metadata.md settles that every source reads that "
            "spelling, whichever one it writes.",
        )
    span = [name for name in ("METADATA_OPEN_SPAN", "METADATA_CLOSE_SPAN") if name in found]
    if len(span) == 1:
        fail(
            f"{path} spells {span[0]} without its pair",
            "spell both halves of the code-span slot, or neither.",
        )

# Each constant, wherever it is spelled, is spelled one way.
for name in ("METADATA_OPEN", "METADATA_CLOSE", "METADATA_OPEN_SPAN", "METADATA_CLOSE_SPAN"):
    spelled = sorted(
        (path, found[name]) for path, found in spellings.items() if name in found
    )
    disagreeing = [(path, literal) for path, literal in spelled if literal != spelled[0][1]]
    if disagreeing:
        for path, literal in disagreeing:
            print(
                f"check-metadata-slot-encoding: {path} spells {name} {literal!r}, and "
                f"{spelled[0][0]} spells it {spelled[0][1]!r}",
                file=sys.stderr,
            )
        fail(
            f"the metadata slot's {name} is spelled two ways",
            "bring them to one spelling — docs/metadata.md settles one slot, in the two "
            "spellings it names, for every source that needs one.",
        )

# The two spellings are one slot: both open with the same marker.
for path, found in sorted(spellings.items()):
    if "METADATA_OPEN_SPAN" in found:
        marker = found["METADATA_OPEN"].replace("\\n", "")
        if not found["METADATA_OPEN_SPAN"].startswith(marker):
            fail(
                f"{path} opens its code-span slot with {found['METADATA_OPEN_SPAN']!r}, which "
                f"is not the marker {marker!r} the multi-line slot opens with",
                "open both spellings with the one marker, so a reader finds the slot in "
                "either.",
            )
PY
