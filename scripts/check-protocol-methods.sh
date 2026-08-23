#!/usr/bin/env bash
# Reconcile docs/plugin-protocol.md against the traits it mirrors, so the specification
# an out-of-tree plugin is written from cannot drift from the Rust contract in silence.
#
# That document is normative: someone writes a plugin in another language from it alone,
# with no compiler between them and the trait. Two of its tables are therefore restatements
# of Rust that nothing else checks — the method table in "## 4. The methods", which names
# one protocol method per `TaskSource` method, and the error table in "## 5. The error
# envelope", which names every `SourceError` variant by its serialized `kind`. The ordinary
# failure is a trait method added or renamed and the document left as it was, after which a
# plugin author implements a protocol this engine no longer speaks — and the reverse, a
# method specified here that nothing answers.
#
# `kind` and `capabilities` are deliberately not protocol methods of their own: both are
# settled by the handshake, which the document says in the same section. They are carried
# below as named exceptions so that decision stays machine-checked rather than looking
# like an omission.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DOCUMENT="$ROOT/docs/plugin-protocol.md" \
SOURCE_RS="$ROOT/crates/onetaskgraph-plugin-api/src/source.rs" \
ERROR_RS="$ROOT/crates/onetaskgraph-plugin-api/src/error.rs" \
python3 <<'PY'
import os
import re
import sys

document = open(os.environ["DOCUMENT"], encoding="utf-8").read()
source_rs = open(os.environ["SOURCE_RS"], encoding="utf-8").read()
error_rs = open(os.environ["ERROR_RS"], encoding="utf-8").read()

# Trait methods the protocol deliberately does not carry as methods of its own, each with
# the reason. A method missing from BOTH this map and the document's table is drift.
NOT_METHODS = {
    "kind": "settled by the handshake response's `kind` field",
    "capabilities": "settled by the handshake response's `capabilities` field",
}

# The one protocol method with no trait method behind it: it stands for building the
# source and reading what it can do, which the method table's own row says.
HANDSHAKE_METHOD = "initialize"

failures = []


def refuse(problem, next_action):
    print(f"check-protocol-methods: {problem}", file=sys.stderr)
    print(f"check-protocol-methods: {next_action}", file=sys.stderr)
    raise SystemExit(1)


def first_column(header):
    """The backticked name in the first column of every row under `header`.

    The header line is the anchor, so a reshaped or deleted table fails loudly here
    rather than silently reducing the set this check compares to nothing.
    """
    lines = document.splitlines()
    try:
        start = lines.index(header)
    except ValueError:
        refuse(
            f"could not find the table headed `{header}` in docs/plugin-protocol.md.",
            "restore that table, or update the anchors in this script if the document "
            "was deliberately reshaped.",
        )

    names = set()
    # +2 skips the header and the `| --- |` separator beneath it.
    for line in lines[start + 2 :]:
        if not line.startswith("|"):
            break
        cell = line.split("|")[1].strip()
        if found := re.fullmatch(r"`([a-z][a-z_-]*)`", cell):
            names.add(found.group(1))
    if not names:
        refuse(
            f"the table headed `{header}` has no rows naming anything.",
            "restore its rows, or teach this script the shape it has now.",
        )
    return names


def compare(what, declared, documented, exceptions, where):
    for name in sorted(declared - documented - set(exceptions)):
        failures.append(
            f"{what} `{name}` is declared in {where} but docs/plugin-protocol.md does not "
            f"specify it. Specify it there, or record why it is absent in this script."
        )
    for name in sorted(documented - declared - set(exceptions)):
        failures.append(
            f"{what} `{name}` is specified by docs/plugin-protocol.md but {where} does not "
            f"declare it. Declare it, or correct the document."
        )
    for name in sorted(set(exceptions) & documented):
        failures.append(
            f"{what} `{name}` is now a row of docs/plugin-protocol.md's table but is still "
            f"carried as an exception here. Drop its entry if that was deliberate."
        )
    for name in sorted(set(exceptions) - declared):
        failures.append(
            f"{what} `{name}` is carried as an exception here but {where} no longer "
            f"declares it. Drop its entry."
        )


trait = re.search(r"pub trait TaskSource: Send \+ Sync \{(.*?)\n\}", source_rs, re.DOTALL)
if trait is None:
    refuse(
        "could not read the `TaskSource` trait from "
        "crates/onetaskgraph-plugin-api/src/source.rs — an empty set would make this "
        "check pass on anything.",
        "restore the trait, or teach this script the shape it has now.",
    )

methods = set(re.findall(r"^\s+(?:async )?fn ([a-z_]+)\(", trait.group(1), re.MULTILINE))
if not methods:
    refuse(
        "read no methods from the `TaskSource` trait.",
        "restore its methods, or teach this script the shape they have now.",
    )

compare(
    "trait method",
    methods,
    first_column("| Method | Trait method |") - {HANDSHAKE_METHOD},
    NOT_METHODS,
    "`TaskSource`",
)

# Variant names as they serialize: `#[serde(rename_all = "kebab-case")]` over the enum.
variants = {
    re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower()
    for name in re.findall(r"^    ([A-Z][A-Za-z]*) \{$", error_rs, re.MULTILINE)
}
if not variants:
    refuse(
        "read no `SourceError` variants from "
        "crates/onetaskgraph-plugin-api/src/error.rs.",
        "the enum's variants are struct variants; restore that shape, or teach this "
        "script the shape they have now.",
    )

compare(
    "SourceError kind",
    variants,
    first_column("| `kind` | Other members | Meaning |"),
    {},
    "`SourceError`",
)

if failures:
    for failure in failures:
        print(f"check-protocol-methods: {failure}", file=sys.stderr)
    raise SystemExit(1)
PY
