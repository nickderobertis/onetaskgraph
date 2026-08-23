#!/usr/bin/env bash
# Reconcile docs/plugin-protocol.md against the traits it mirrors, so the specification
# an out-of-tree plugin is written from cannot drift from the Rust contract in silence.
#
# That document is normative: someone writes a plugin in another language from it alone,
# with no compiler between them and the trait. Everything it states about names is therefore
# a restatement of Rust, and this reconciles the three kinds of restatement it makes.
#
# Its two tables are checked BOTH ways, because a table's rows are structure this script can
# read: the method table in "## 4. The methods" against `TaskSource`'s own methods, and the
# error table in "## 5. The error envelope" against `SourceError`'s variants. A name either
# side has and the other does not fails.
#
# Its prose enumerations — the `Capabilities` fields, and the serialized variants of the
# contract's kebab-case enums — are checked one way, declared-to-documented: every name Rust
# serializes has to appear in the document, spelled the way the wire spells it. One way,
# because prose has no rows to read back; that direction is the one the ordinary failure
# takes anyway — a variant added or renamed in Rust and the document left as it was, after
# which a plugin author implements a protocol this engine no longer speaks.
#
# `kind` and `capabilities` are deliberately not protocol methods of their own: both are
# settled by the handshake, which the document says in the same section. They are carried
# below as named exceptions so that decision stays machine-checked rather than looking
# like an omission.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DOCUMENT="$ROOT/docs/plugin-protocol.md" \
API_SRC="$ROOT/crates/onetaskgraph-plugin-api/src" \
python3 <<'PY'
import os
import re
import sys


def refuse(problem, next_action):
    print(f"check-protocol-contract: {problem}", file=sys.stderr)
    print(f"check-protocol-contract: {next_action}", file=sys.stderr)
    raise SystemExit(1)


def read(path, what):
    """One input file, reported by name rather than as a traceback if it will not open."""
    try:
        return open(path, encoding="utf-8").read()
    except OSError as problem:
        refuse(
            f"could not read {path}: {problem.strerror}.",
            f"restore {what}, or point this check at where it moved to.",
        )


def api(module):
    """One module of `onetaskgraph-plugin-api`."""
    return read(os.path.join(os.environ["API_SRC"], module), f"the api crate's `{module}`")


document = read(os.environ["DOCUMENT"], "the protocol document")
source_rs = api("source.rs")
error_rs = api("error.rs")
contract_rs = error_rs + api("capability.rs") + api("query.rs") + api("work.rs")

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


def kebab(name):
    """A variant name as `#[serde(rename_all = "kebab-case")]` writes it."""
    return re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower()


def mentioned(quoted):
    """Whether the document spells `quoted` the way the wire spells it."""
    return f'"{quoted}"' in document


# Every kebab-case enum the contract crate declares, so a new one is covered the day it
# lands rather than the day somebody remembers to list it here.
enums = re.findall(
    r'#\[serde\(rename_all = "kebab-case"\)\]\s*pub enum (\w+) \{(.*?)\n\}',
    contract_rs,
    re.DOTALL,
)
if not enums:
    refuse(
        "found no kebab-case enums in the api crate — an empty set would make this "
        "check pass on anything.",
        "restore them, or teach this script the shape they have now.",
    )

for name, body in enums:
    declared = [kebab(variant) for variant in re.findall(r"^    ([A-Z]\w*)[,({]", body, re.MULTILINE)]
    if not declared:
        refuse(
            f"read no variants from the `{name}` enum of the api crate.",
            "restore its variants, or teach this script the shape they have now.",
        )
    for variant in declared:
        if not mentioned(variant):
            failures.append(
                f"`{name}` serializes the value \"{variant}\" but docs/plugin-protocol.md "
                f"never spells it. Specify it there — a plugin author writing from that "
                f"document alone would never emit or accept it."
            )

capabilities = re.search(r"pub struct Capabilities \{(.*?)\n\}", contract_rs, re.DOTALL)
if capabilities is None:
    refuse(
        "could not read the `Capabilities` struct from the api crate.",
        "restore it, or teach this script the shape it has now.",
    )
fields = re.findall(r"^    pub (\w+):", capabilities.group(1), re.MULTILINE)
if not fields:
    refuse(
        "read no fields from the `Capabilities` struct of the api crate.",
        "restore its fields, or teach this script the shape they have now.",
    )
for field in fields:
    if f"`{field}`" not in document and not mentioned(field):
        failures.append(
            f"`Capabilities` carries the field \"{field}\" but docs/plugin-protocol.md "
            f"never names it. Specify it there — the handshake is where a plugin author "
            f"learns what to declare."
        )

if failures:
    for failure in failures:
        print(f"check-protocol-contract: {failure}", file=sys.stderr)
    raise SystemExit(1)
PY
