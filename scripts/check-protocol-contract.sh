#!/usr/bin/env bash
# Reconcile docs/plugin-protocol.md against the traits it mirrors.
#
# That document is normative — someone writes a plugin in another language from it alone,
# with no compiler between them and the trait — so every name it states is a restatement of
# Rust, and drift in either direction reaches that author as a protocol this engine does not
# speak. Both directions therefore fail here: names Rust has and the document lacks, and
# names the document has and Rust no longer declares.
#
# Its tables have rows to read back. Its prose does not, so the document's own markup stands
# in for them: a backticked JSON string is a serialized value, and a backticked snake_case
# word in the `Capabilities` section is a field. A value one section borrows from another is
# recorded below with the enum that declares it, rather than the reverse direction being
# widened to accept it.
#
# Every check is scoped to the section that *specifies* the thing, never to the whole
# document: a name occurring in an unrelated paragraph is not a specification of it, and a
# check satisfied that way passes over the drift it exists to catch. The sections are named
# below, so a renamed or deleted one fails loudly instead of quietly widening the search.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DOCUMENT="$ROOT/docs/plugin-protocol.md" \
API_SRC="$ROOT/crates/onetaskgraph-plugin-api/src" \
TRANSPORT="$ROOT/crates/onetaskgraph-core/src/subprocess/connection.rs" \
DEADLINE_SOURCE="$ROOT/crates/onetaskgraph-core/src/subprocess/source.rs" \
SUBPROCESS_CONFIG="$ROOT/crates/onetaskgraph-core/src/subprocess/plugin.rs" \
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
    except (OSError, UnicodeDecodeError) as problem:
        refuse(
            f"could not read {path}: {problem}.",
            f"restore {what} as readable UTF-8 text, or point this check at where it "
            "moved to.",
        )


def api(module):
    """One module of `onetaskgraph-plugin-api`."""
    return read(os.path.join(os.environ["API_SRC"], module), f"the api crate's `{module}`")


document = read(os.environ["DOCUMENT"], "the protocol document")
source_rs = api("source.rs")
error_rs = api("error.rs")
contract_rs = error_rs + api("capability.rs") + api("query.rs") + api("work.rs") + api("write.rs")

# Trait methods the protocol deliberately does not carry as methods of its own, each with
# the reason. A method missing from BOTH this map and the document's table is drift.
NOT_METHODS = {
    "kind": "settled by the handshake response's `kind` field",
    "capabilities": "settled by the handshake response's `capabilities` field",
    "writes": "settled by the handshake response's `writes` field, which §3.3 specifies",
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
    "write_task": "### 4.9 `write_task` and `write_project`",
    "write_project": "### 4.9 `write_task` and `write_project`",
    "delete_task": "### 4.10 `delete_task` and `delete_project`",
    "delete_project": "### 4.10 `delete_task` and `delete_project`",
}

ENUM_SECTIONS = {
    "WriteSupport": "### 3.3 `writes`",
    "Support": CAPABILITIES_SECTION,
    "DependencySupport": CAPABILITIES_SECTION,
    "StatusCategory": "### 4.5 `query_tasks`",
    "TextFields": "### 4.5 `query_tasks`",
    "ProjectFilter": "### 4.5 `query_tasks`",
    "DependencyKind": "### 4.8 `task_dependencies` and `project_dependencies`",
    "ItemKind": "### 4.8 `task_dependencies` and `project_dependencies`",
    "Direction": "### 4.8 `task_dependencies` and `project_dependencies`",
}

# Values a section spells although the enum declaring them is specified elsewhere, each with
# that enum. §4.8 cannot say what a forward-only plugin is asked for without naming the two
# `DependencySupport` values §4.2 specifies. Recorded here rather than the reverse check
# accepting any value declared anywhere, so a value no enum declares any more still fails.
ENUM_VALUE_CROSS_REFERENCES = {
    "### 4.8 `task_dependencies` and `project_dependencies`": {
        "both-directions": "DependencySupport",
        "forward-only": "DependencySupport",
    },
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

variants_of = {}
for name, body in enums:
    declared = [kebab(variant) for variant in re.findall(r"^    ([A-Z]\w*)[,({]", body, re.MULTILINE)]
    if not declared:
        refuse(
            f"read no variants from the `{name}` enum of the api crate.",
            "restore its variants, or teach this script the shape they have now.",
        )
    variants_of[name] = set(declared)
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

# The other direction, once per section rather than once per enum, because several enums
# share a section and a value spelled there belongs to whichever of them declares it.
for heading in sorted(set(ENUM_SECTIONS.values())):
    specified = {name for name, where in ENUM_SECTIONS.items() if where == heading}
    borrowed = ENUM_VALUE_CROSS_REFERENCES.get(heading, {})
    declared_here = set().union(*(variants_of[name] for name in specified))
    for value, owner in sorted(borrowed.items()):
        if owner not in variants_of:
            refuse(
                f"\"{heading}\" is recorded as borrowing \"{value}\" from `{owner}`, which "
                "the api crate does not declare as a kebab-case enum.",
                "correct the enum named in ENUM_VALUE_CROSS_REFERENCES, or drop the entry.",
            )
        if value not in variants_of[owner]:
            failures.append(
                f"\"{heading}\" is recorded as borrowing the value \"{value}\" from "
                f"`{owner}`, which no longer declares it. Drop that entry, and correct the "
                "section if it still spells the value."
            )
        elif value in declared_here:
            failures.append(
                f"\"{heading}\" is recorded as borrowing the value \"{value}\", but an enum "
                f"that section specifies declares it. Drop that entry."
            )
    # A backticked JSON string is how this document spells a serialized value; anything
    # else backticked in these sections is a field or a type, which the checks above cover.
    for value in sorted(set(re.findall(r'`"([a-z0-9-]+)"`', section(heading)))):
        if value not in declared_here and value not in borrowed:
            failures.append(
                f"\"{heading}\" spells the value \"{value}\", but no enum that section "
                f"specifies declares it. Correct the section, or record where the value "
                "comes from in ENUM_VALUE_CROSS_REFERENCES — a plugin author writing from "
                "it would emit a value this engine does not accept."
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
# The other direction. That section specifies one struct, so a backticked snake_case word in
# it is a field of that struct — a serialized value is quoted, and a type is capitalized.
for named in sorted(set(re.findall(r"`([a-z][a-z0-9_]*)`", handshake))):
    if named not in fields:
        failures.append(
            f"\"{CAPABILITIES_SECTION}\" names \"{named}\" as a field of `Capabilities`, "
            f"which does not carry it. Correct the section, or declare the field — a plugin "
            f"author writing from it would declare something the engine never reads."
        )

# The wire structs, member by member.
#
# The checks above reconcile the NAMES a plugin author reads — methods, parameters, results,
# enum values, `Capabilities` fields — but a struct's members are structure, and a field Rust
# gains without the document following is drift no name comparison sees. It reaches that
# author as a field the engine sends or expects and their plugin never handles.
#
# Forward only, and deliberately: these sections carry method names, cross-referenced fields
# and JSON keys of the envelope around the payload, so "every backticked word here is a
# member of this struct" is not true of them the way it is of §4.2, which specifies one
# struct and nothing else. The direction dropped is the weaker one — a member the document
# invents is visible to the author writing from it, while one it omits is not.
STRUCT_SECTIONS = {
    "PageRequest": "### 4.1 Common parameter shapes",
    "Page": "### 4.1 Common parameter shapes",
    "Health": "### 4.3 `health`",
    "TaskQuery": "### 4.5 `query_tasks`",
    "TextQuery": "### 4.5 `query_tasks`",
    "LabelFilter": "### 4.5 `query_tasks`",
    "ProjectQuery": "### 4.6 `query_projects`",
    "DependencyEdge": "### 4.8 `task_dependencies` and `project_dependencies`",
    "DependencyEndpoint": "### 4.8 `task_dependencies` and `project_dependencies`",
    "ItemWrite": "### 4.9 `write_task` and `write_project`",
}

def wire_members(struct):
    """The members a hand-written `Serialize` puts on the wire, which no field scan sees.

    A struct that serialises itself keeps its own shape private and writes a local `Wire`
    struct instead — `DependencyEndpoint` does, because its id is one of two variants and
    the wire is one string. Reading only `pub` fields would reconcile the half of such a
    type that happens to be public and silently ignore the rest, which is the drift this
    gate exists to catch rather than an exception to it.
    """
    impl = re.search(
        r"impl Serialize for %s \{(.*?)\n\}" % re.escape(struct),
        source_rs + contract_rs,
        re.DOTALL,
    )
    if impl is None:
        return []
    wire = re.search(r"struct Wire(?:<[^>]*>)? \{(.*?)\n\s*\}", impl.group(1), re.DOTALL)
    if wire is None:
        refuse(
            f"`{struct}` writes its own `Serialize`, but this script cannot find the "
            f"`Wire` struct that says what it puts on the wire.",
            "restore it, or teach this script the shape it has now — otherwise the "
            "serialized members of this type go unreconciled while the check still passes.",
        )
    return re.findall(r"^\s*(\w+):", wire.group(1), re.MULTILINE)


for struct, heading in STRUCT_SECTIONS.items():
    # Across both, because `Health` is declared beside the trait that returns it while
    # the rest are in the contract modules.
    declaration = re.search(
        r"pub struct %s(?:<[^>]*>)? \{(.*?)\n\}" % re.escape(struct),
        source_rs + contract_rs,
        re.DOTALL,
    )
    if declaration is None:
        refuse(
            f"could not read the `{struct}` struct from the api crate.",
            "restore it, or teach this script the shape it has now — a struct this "
            "document specifies cannot go unreconciled.",
        )
    members = re.findall(r"^    pub (\w+):", declaration.group(1), re.MULTILINE)
    members += wire_members(struct)
    if not members:
        refuse(
            f"read no fields from the `{struct}` struct of the api crate.",
            "restore its fields, or teach this script the shape they have now.",
        )
    specified = section(heading)
    for member in members:
        if not spelled(member, specified):
            failures.append(
                f"`{struct}` carries the field \"{member}\" but \"{heading}\" never "
                f"names it. Specify it there — a plugin author writing from that section "
                f"would never handle it."
            )

# The framing limit is a number rather than a name, so neither of the two scans above
# would ever notice it drifting. It is normative — a plugin author reads it and sizes their
# writes by it — and it is also a constant this engine enforces, so the two have to be one
# value. Read both and compare.
FRAMING_SECTION = "## 1. Framing"

transport = read(os.environ["TRANSPORT"], "the transport that enforces the framing limit")
declared = re.search(r"pub const MAX_LINE: u64 = (\d+) \* 1024 \* 1024;", transport)
if declared is None:
    refuse(
        "could not read `MAX_LINE` from the transport.",
        "restore it as `pub const MAX_LINE: u64 = <n> * 1024 * 1024;`, or teach this "
        "script the shape it has now — the document states this number and the two "
        "cannot be allowed to drift.",
    )
stated = re.search(r"\*\*(\d+) MiB\*\*", section(FRAMING_SECTION))
if stated is None:
    refuse(
        f'"{FRAMING_SECTION}" no longer states a maximum line length in **<n> MiB**.',
        "restore it — a plugin author sizes their writes by that number, and this check "
        "is what keeps it the one this engine actually enforces.",
    )
if declared.group(1) != stated.group(1):
    failures.append(
        f'"{FRAMING_SECTION}" states a maximum line length of {stated.group(1)} MiB but '
        f"`MAX_LINE` enforces {declared.group(1)} MiB. Make them one value: a plugin "
        f"written to the document would have its connection closed for obeying it."
    )

# The configured request deadline is another normative number shared by the engine and
# an author implementing from this document. Keep its one Rust declaration and prose in
# step for the same reason as the framing ceiling above.
deadline_source = read(os.environ["DEADLINE_SOURCE"], "the subprocess deadline declaration")
deadline_config = read(os.environ["SUBPROCESS_CONFIG"], "the subprocess configuration")
declared_deadline = re.search(
    r"pub const DEFAULT: Self = Self\(NonZeroU64::new\(([\d_]+)\)", deadline_source
)
stated_deadline = re.search(r"when omitted it is (\d+) milliseconds", section(FRAMING_SECTION))
if declared_deadline is None or stated_deadline is None:
    refuse(
        "could not reconcile the default subprocess deadline between Rust and §1.",
        "keep `RequestDeadline::DEFAULT` and the `when omitted it is <n> milliseconds` "
        "sentence in their checked forms, or update this check with both shapes.",
    )
if declared_deadline.group(1).replace("_", "") != stated_deadline.group(1):
    failures.append(
        f"§1 states a {stated_deadline.group(1)} millisecond default deadline but "
        f"RequestDeadline::DEFAULT is {declared_deadline.group(1)} milliseconds. Update "
        "the Rust default or the documented default so they agree."
    )

declared_field = re.search(r"pub (deadline_ms): NonZeroU64,", deadline_config)
documented_field = re.search(r"positive integer `(deadline_ms)`", section(FRAMING_SECTION))
if declared_field is None or documented_field is None:
    refuse(
        "could not reconcile the positive `deadline_ms` configuration field with §1.",
        "keep `SubprocessConfig.deadline_ms: NonZeroU64` and §1's positive-integer "
        "wording aligned, or update this check with both shapes.",
    )
if declared_field.group(1) != documented_field.group(1):
    failures.append(
        f"§1 documents `{documented_field.group(1)}` but SubprocessConfig declares "
        f"`{declared_field.group(1)}`. Rename one so the configuration contract agrees."
    )

if failures:
    for failure in failures:
        print(f"check-protocol-contract: {failure}", file=sys.stderr)
    raise SystemExit(1)
PY
