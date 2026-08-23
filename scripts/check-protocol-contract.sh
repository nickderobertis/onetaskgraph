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
# Every one of those one-way checks is scoped to the *section that specifies the thing*,
# never to the document as a whole. A name that happens to occur in an unrelated paragraph
# is not a specification of it, and a check satisfied that way passes over exactly the drift
# it exists to catch: the section going stale while the name survives somewhere else. Each
# section is named below, so a section that is renamed or deleted fails loudly here instead
# of quietly widening the search back to the whole document.
#
# A method's signature is reconciled too, not only its name: every parameter the trait takes
# has to be named in that method's own section, and the type its `result` carries has to be
# spelled there. A method that gains an argument in Rust and keeps its documented shape is
# drift a name-only comparison cannot see.
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

# Where each thing is specified. Every one-way check below reads one of these sections
# rather than the whole document, so an incidental mention elsewhere cannot stand in for a
# specification. A heading that moves fails here by name.
CAPABILITIES_SECTION = "### 4.2 `Capabilities`"

METHOD_SECTIONS = {
    "health": "### 4.3 `health`",
    "get_task": "### 4.4 `get_task` and `get_project`",
    "get_project": "### 4.4 `get_task` and `get_project`",
    "query_tasks": "### 4.5 `query_tasks`",
    "query_projects": "### 4.6 `query_projects`",
    "labels": "### 4.7 `labels`",
    "task_dependencies": "### 4.8 `task_dependencies` and `project_dependencies`",
    "project_dependencies": "### 4.8 `task_dependencies` and `project_dependencies`",
}

ENUM_SECTIONS = {
    "Support": CAPABILITIES_SECTION,
    "DependencySupport": CAPABILITIES_SECTION,
    "StatusCategory": "### 4.5 `query_tasks`",
    "TextFields": "### 4.5 `query_tasks`",
    "ProjectFilter": "### 4.5 `query_tasks`",
    "DependencyKind": "### 4.8 `task_dependencies` and `project_dependencies`",
    "Direction": "### 4.8 `task_dependencies` and `project_dependencies`",
}

# `SourceError` is the one kebab-case enum with no entry above, because §5 specifies it as a
# table and the table is already compared BOTH ways — a stronger check than any one-way scan
# of the same section would be. Every other kebab-case enum must be placed, so a new one
# cannot arrive unchecked.
ENUMS_CHECKED_BY_A_TABLE = {"SourceError"}

failures = []


def section(heading):
    """The text under `heading`, up to the next heading of the same or higher level.

    The heading is an anchor for the same reason a table's header line is: a section that
    was renamed or removed fails here, rather than silently widening a check back to the
    whole document.
    """
    lines = document.splitlines()
    start = next((index for index, line in enumerate(lines) if line == heading), None)
    if start is None:
        refuse(
            f"could not find the section headed `{heading}` in docs/plugin-protocol.md.",
            "restore that section, or update the anchors in this script if the document "
            "was deliberately reshaped.",
        )
    level = len(heading) - len(heading.lstrip("#"))
    end = len(lines)
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line.startswith("#") and len(line) - len(line.lstrip("#")) <= level:
            end = index
            break
    return "\n".join(lines[start:end])


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


def signature(method):
    """One method's parameter names and the type its `Result` carries."""
    found = re.search(
        rf"\n    (?:async )?fn {method}\(\s*&self,?(.*?)\)\s*->\s*Result<(.+?), SourceError>",
        trait.group(1),
        re.DOTALL,
    )
    if found is None:
        refuse(
            f"could not read the signature of `TaskSource::{method}`.",
            "restore it, or teach this script the shape it has now.",
        )
    parameters = re.findall(r"(\w+)\s*:", found.group(1))
    carried = " ".join(found.group(2).split())
    # The document specifies what a lookup returns, not that Rust wraps it in an `Option`:
    # "a `Task`, or `null`" is the wire's way of saying `Option<Task>`.
    inner = re.fullmatch(r"Option<(.+)>", carried)
    return parameters, inner.group(1) if inner else carried


def spelled(name, text):
    """Whether `text` spells `name` as the wire spells it, or as the document marks a type."""
    return f'"{name}"' in text or f"`{name}`" in text


for method in sorted(methods - set(NOT_METHODS)):
    if method not in METHOD_SECTIONS:
        refuse(
            f"`TaskSource::{method}` has no section named in METHOD_SECTIONS, so its "
            "parameters and result would go unchecked.",
            "add the section of docs/plugin-protocol.md that specifies it.",
        )
    where = section(METHOD_SECTIONS[method])
    parameters, carried = signature(method)
    for parameter in parameters:
        if not spelled(parameter, where):
            failures.append(
                f"`TaskSource::{method}` takes `{parameter}`, but "
                f"\"{METHOD_SECTIONS[method]}\" never names it. Specify it there — a plugin "
                f"author writing from that section alone would never read it."
            )
    if not spelled(carried, where):
        failures.append(
            f"`TaskSource::{method}` returns `{carried}`, but "
            f"\"{METHOD_SECTIONS[method]}\" never names that type. Say what `result` "
            f"carries there."
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

ERROR_TABLE = "| `kind` | Other members | Meaning |"

compare(
    "SourceError kind",
    variants,
    first_column(ERROR_TABLE),
    {},
    "`SourceError`",
)


def documented_members():
    """Each error kind's members, read from the second column of the error table.

    A member is a backticked name followed by its type in brackets — `message` (string).
    That shape is what distinguishes a member from the other backticked words a cell
    carries, `null` among them.
    """
    lines = document.splitlines()
    start = lines.index(ERROR_TABLE)
    members = {}
    for line in lines[start + 2 :]:
        if not line.startswith("|"):
            break
        cells = line.split("|")
        kind = re.fullmatch(r"`([a-z][a-z_-]*)`", cells[1].strip())
        if kind:
            members[kind.group(1)] = set(re.findall(r"`(\w+)` \(", cells[2]))
    return members


# The members each variant really carries. The table's second column is structure, so it
# is read back the same way its first column is: a variant that gains or loses a field
# without the table following is drift a name-only comparison cannot see.
declared_members = {
    kebab_name: set(re.findall(r"^        (\w+):", body, re.MULTILINE))
    for kebab_name, body in (
        (re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower(), body)
        for name, body in re.findall(
            r"^    ([A-Z][A-Za-z]*) \{(.*?)^    \},", error_rs, re.DOTALL | re.MULTILINE
        )
    )
}
if declared_members.keys() != variants:
    refuse(
        "read a different set of `SourceError` variants when reading their fields than "
        "when reading their names, so one of the two patterns no longer fits the enum.",
        "teach this script the shape `SourceError` has now.",
    )

documented = documented_members()
for kind in sorted(variants & documented.keys()):
    for member in sorted(declared_members[kind] - documented[kind]):
        failures.append(
            f"`SourceError::{kind}` carries the member `{member}` but the error table's "
            f"row for it does not list it. Add it — a plugin author writing that envelope "
            f"from the table alone would omit it."
        )
    for member in sorted(documented[kind] - declared_members[kind]):
        failures.append(
            f"the error table's `{kind}` row lists the member `{member}` but "
            f"`SourceError::{kind}` does not carry it. Correct the table, or declare it."
        )


def kebab(name):
    """A variant name as `#[serde(rename_all = "kebab-case")]` writes it."""
    return re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower()




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
    if name in ENUMS_CHECKED_BY_A_TABLE:
        continue
    if name not in ENUM_SECTIONS:
        refuse(
            f"the kebab-case enum `{name}` has no section named in ENUM_SECTIONS, so its "
            "variants would go unchecked.",
            "add the section of docs/plugin-protocol.md that specifies it, or record it as "
            "one the tables already cover.",
        )
    where = section(ENUM_SECTIONS[name])
    for variant in declared:
        if f'"{variant}"' not in where:
            failures.append(
                f"`{name}` serializes the value \"{variant}\" but "
                f"\"{ENUM_SECTIONS[name]}\" never spells it. Specify it there — a plugin "
                f"author writing from that section alone would never emit or accept it."
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
handshake = section(CAPABILITIES_SECTION)
for field in fields:
    if not spelled(field, handshake):
        failures.append(
            f"`Capabilities` carries the field \"{field}\" but "
            f"\"{CAPABILITIES_SECTION}\" never names it. Specify it there — the handshake "
            f"is where a plugin author learns what to declare."
        )

if failures:
    for failure in failures:
        print(f"check-protocol-contract: {failure}", file=sys.stderr)
    raise SystemExit(1)
PY
