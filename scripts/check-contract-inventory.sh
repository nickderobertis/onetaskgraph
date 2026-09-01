#!/usr/bin/env bash
# Reconcile AGENTS.md's plugin-contract inventory against what the api crate really
# exports, so the two cannot drift apart in silence.
#
# `Health` remains the one named exception so resolving AGENTS.md's open contract question
# requires removing the executable exception in the same change.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

AGENTS_MD="$ROOT/AGENTS.md" API_LIB="$ROOT/crates/onetaskgraph-plugin-api/src/lib.rs" python3 <<'PY'
import os
import re
import sys

agents_md = os.environ["AGENTS_MD"]
api_lib = os.environ["API_LIB"]

# Exported names that AGENTS.md's inventory does not list, each with the reason it is
# absent. An export missing from BOTH this map and the inventory is drift and fails.
EXCEPTIONS = {
    "Health": (
        "the approved enumeration is exhaustive and omits it, while `TaskSource::health` "
        "returns it and that trait is in this crate. AGENTS.md records this as an open "
        "contract question; resolving it belongs to the contract's owner, not to this "
        "repository"
    ),
    "unwritable": (
        "a free function, not one of the contract's types — the inventory enumerates the "
        "traits and types a plugin author implements against, and this is the one refusal "
        "`WriteSupport::Unsupported` obliges every unwritten source to answer a write with"
    ),
    "documentless": (
        "a free function, not one of the contract's types — the inventory enumerates the "
        "traits and types a plugin author implements against, and this is the one refusal "
        "`Capabilities.documents: Support::Unsupported` obliges every document-free source "
        "to answer a document read with"
    ),
    "SOURCE_NAME_PATTERN": (
        "a const, not one of the contract's types — the inventory enumerates the traits "
        "and types a plugin author implements against"
    ),
}

document = open(agents_md, encoding="utf-8").read()

# The api-crate bullet of "## The plugin contract": from its own name to the sentence
# that closes it. Anchoring on both ends means a reformatted section fails loudly here
# rather than silently shrinking the set this check compares.
bullet = re.search(
    r"- \*\*`onetaskgraph-plugin-api`\*\*(.*?)"
    r"\*\*It depends on no other crate of this workspace\.\*\*",
    document,
    re.DOTALL,
)
if bullet is None:
    print(
        "check-contract-inventory: could not find the `onetaskgraph-plugin-api` inventory "
        "bullet in AGENTS.md.",
        file=sys.stderr,
    )
    print(
        "check-contract-inventory: it is the bullet under '## The plugin contract' ending "
        "'**It depends on no other crate of this workspace.**' — restore it, or update the "
        "anchors in this script if the section was deliberately reshaped.",
        file=sys.stderr,
    )
    raise SystemExit(1)

# Type names only: the inventory also backticks the crate names either side of it.
documented = {
    name
    for name in re.findall(r"`([A-Za-z_][A-Za-z0-9_]*)`", bullet.group(1))
    if name[0].isupper()
}

source = open(api_lib, encoding="utf-8").read()
exported = set()
for item in re.findall(r"^pub use [a-z_]+::(?:\{(.*?)\}|([A-Za-z_][A-Za-z0-9_]*));",
                       source, re.DOTALL | re.MULTILINE):
    braced, single = item
    for name in (braced or single).split(","):
        name = name.strip()
        if name:
            exported.add(name)

if not exported:
    print(
        "check-contract-inventory: read no exports from crates/onetaskgraph-plugin-api/"
        "src/lib.rs — an empty set would make this check pass on anything.",
        file=sys.stderr,
    )
    print(
        "check-contract-inventory: the crate re-exports its public surface with `pub use "
        "<module>::{...};` lines; restore that shape or teach this script the new one.",
        file=sys.stderr,
    )
    raise SystemExit(1)

failures = []

for name in sorted(exported - documented - set(EXCEPTIONS)):
    failures.append(
        f"{name} is exported by onetaskgraph-plugin-api but is not in AGENTS.md's "
        f"inventory of that crate. Add it there, or record why it is absent in this "
        f"script's EXCEPTIONS."
    )

for name in sorted(documented - exported):
    failures.append(
        f"{name} is listed in AGENTS.md's inventory of onetaskgraph-plugin-api but that "
        f"crate does not export it. Export it, or correct the inventory."
    )

for name in sorted(set(EXCEPTIONS) & documented):
    failures.append(
        f"{name} is now in AGENTS.md's inventory but is still carried as an exception in "
        f"this script. If the contract's owner has settled it, delete its EXCEPTIONS entry "
        f"— and the open-question note in AGENTS.md if it has one — in the same change."
    )

for name in sorted(set(EXCEPTIONS) - exported):
    failures.append(
        f"{name} is carried as an exception in this script but onetaskgraph-plugin-api no "
        f"longer exports it. Drop its EXCEPTIONS entry."
    )

# `Health` is the open question, so the note recording it is part of the contract as
# written. Losing the note would leave the exception above as the only trace of a
# question the enumeration's silence must never be read as answering.
if "Health" in EXCEPTIONS and "Open contract question — `Health`" not in document:
    failures.append(
        "AGENTS.md no longer carries the 'Open contract question — `Health`' note, but "
        "`Health` is still an exception here. Restore the note, or — if the contract's "
        "owner has settled the question — add `Health` to the inventory and drop its "
        "EXCEPTIONS entry."
    )

if failures:
    print(
        "check-contract-inventory: AGENTS.md and onetaskgraph-plugin-api disagree about "
        "the plugin contract.",
        file=sys.stderr,
    )
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    raise SystemExit(1)
PY
