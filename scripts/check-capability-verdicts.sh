#!/usr/bin/env bash
# Reconcile each plugin's per-field capability verdicts with what it really declares.
#
# Every source plugin's module documentation records one verdict per field of the plugin
# contract's `Capabilities` — supported and proven, or unsupported and why. That list is
# prose, and prose drifts silently: a field flipped in `capabilities()` leaves a paragraph
# still claiming the old answer, and a field added to the contract leaves every plugin's
# list quietly one row short. Both are the failure this product has already shipped once —
# a capability declared and not applied, invisible because nothing reconciled the claim
# with the code.
#
# So the verdicts are not prose alone. This reads them out of each plugin's own
# documentation and reconciles them three ways: against the contract's field list, against
# the plugin's own declaration, and — for a field recorded unsupported — against the entry
# in docs/follow-ups.md that says so. Implementing an unsupported capability therefore
# fails here until its verdict and its follow-up are brought with it.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CONTRACT="$ROOT/crates/onetaskgraph-plugin-api/src/capability.rs" \
REGISTRY="$ROOT/crates/onetaskgraph-core/src/registry.rs" \
FOLLOW_UPS="$ROOT/docs/follow-ups.md" \
python3 <<'PY'
import os
import re
import sys

NAME = "check-capability-verdicts"

# Where each registered plugin kind records its verdicts, and whether its declaration is a
# literal in that same file or chosen elsewhere. `configured` means the plugin reports what
# something else decided — a document, for `in-memory`, or the program behind the pipe, for
# `subprocess` — so there is no literal here to compare values against and every field is
# supported by construction.
PLUGINS = {
    "github-projects": ("crates/onetaskgraph-github-projects/src/lib.rs", "literal"),
    "in-memory": ("crates/onetaskgraph-in-memory/src/lib.rs", "configured"),
    "linear": ("crates/onetaskgraph-linear/src/lib.rs", "literal"),
    "local-md": ("crates/onetaskgraph-local-md/src/lib.rs", "literal"),
    "subprocess": ("crates/onetaskgraph-core/src/subprocess/mod.rs", "configured"),
}

HEADING = "# What this source declares, field by field"
VERDICT_ROW = re.compile(r"^//!\s*\|\s*`(\w+)`\s*\|\s*\*\*(Supported|Unsupported)")
DECLARED_FIELD = re.compile(
    r"^\s*(\w+):\s*(?:Support::(\w+)|DependencySupport::(\w+)|.+),\s*$"
)


def read(path, what):
    """The file's text, or a named problem and a concrete next action."""
    try:
        return open(path, encoding="utf-8").read()
    except (OSError, UnicodeDecodeError) as error:
        print(f"{NAME}: could not read {what}: {error}", file=sys.stderr)
        print(
            f"{NAME}: restore {path} — it is one of the files this check reconciles — "
            "then re-run 'just check'.",
            file=sys.stderr,
        )
        raise SystemExit(1) from None


def refuse(*lines):
    for line in lines:
        print(f"{NAME}: {line}", file=sys.stderr)
    raise SystemExit(1)


contract_source = read(os.environ["CONTRACT"], "the contract's capability value")
registry_source = read(os.environ["REGISTRY"], "the plugin registry")
follow_ups = read(os.environ["FOLLOW_UPS"], "the tracked follow-ups")

# The contract's own field list, in its own order. Anchored on the struct so a field named
# in a doc comment elsewhere in that file is not mistaken for one.
struct = re.search(r"pub struct Capabilities \{(.*?)\n\}", contract_source, re.DOTALL)
if struct is None:
    refuse(
        "could not find `pub struct Capabilities` in the contract crate.",
        "restore it, or update this check to read the value plugins declare.",
    )
contract_fields = re.findall(r"^\s*pub (\w+):", struct.group(1), re.MULTILINE)
if not contract_fields:
    refuse("`Capabilities` appears to have no fields, which cannot be right.")

as_str = re.search(
    r"pub fn as_str\(self\) -> &'static str \{(.*?)\n    \}", registry_source, re.DOTALL
)
if as_str is None:
    refuse("could not find `PluginKind::as_str` in the plugin registry.")
registered = re.findall(r'=> "([^"]+)"', as_str.group(1))

missing = [kind for kind in registered if kind not in PLUGINS]
if missing:
    refuse(
        f"the registry knows {', '.join(missing)}, which this check has no verdict "
        "table for.",
        "add the plugin's module documentation section headed "
        f"`{HEADING}` — one row per field of `Capabilities` — and its path to PLUGINS in "
        "this script.",
    )
unregistered = [kind for kind in PLUGINS if kind not in registered]
if unregistered:
    refuse(
        f"this check names {', '.join(unregistered)}, which the registry no longer has.",
        "remove them from PLUGINS in this script.",
    )

problems = []
unsupported = {}

for kind in registered:
    path, shape = PLUGINS[kind]
    source = read(path, f"the {kind} plugin")
    if HEADING not in source:
        problems.append(
            f"{kind}: {path} has no section headed `{HEADING}`, so nothing records what "
            "each capability field is."
        )
        continue

    verdicts = {}
    for line in source.splitlines():
        row = VERDICT_ROW.match(line)
        if row is not None:
            verdicts[row.group(1)] = row.group(2)

    for field in contract_fields:
        if field not in verdicts:
            problems.append(
                f"{kind}: `{field}` is a field of the contract's `Capabilities` and "
                f"{path} records no verdict for it."
            )
    for field in verdicts:
        if field not in contract_fields:
            problems.append(
                f"{kind}: {path} records a verdict for `{field}`, which is not a field "
                "of the contract's `Capabilities`."
            )

    if shape == "configured":
        for field, verdict in verdicts.items():
            if verdict != "Supported":
                problems.append(
                    f"{kind}: `{field}` is recorded {verdict}, but this plugin reports "
                    "whatever its configuration or its hosted source declares rather "
                    "than a value of its own."
                )
        continue

    declaration = re.search(
        r"fn capabilities\(&self\) -> Capabilities \{\s*Capabilities \{(.*?)\n\s*\}",
        source,
        re.DOTALL,
    )
    if declaration is None:
        problems.append(
            f"{kind}: could not find the `Capabilities` literal `capabilities` returns "
            f"in {path}, so its verdicts cannot be reconciled with it."
        )
        continue
    for line in declaration.group(1).splitlines():
        field = DECLARED_FIELD.match(line)
        if field is None:
            continue
        name, support, dependency = field.group(1), field.group(2), field.group(3)
        if name not in verdicts:
            continue
        declared = "Unsupported" if support == "Unsupported" else "Supported"
        if verdicts[name] != declared:
            spelled = support or dependency or "a value of its own"
            problems.append(
                f"{kind}: `{name}` is recorded {verdicts[name]} in {path}, but "
                f"`capabilities` declares {spelled}."
            )
        if declared == "Unsupported":
            unsupported.setdefault(kind, set()).add(name)

# A follow-up describing a capability that has since been implemented is worse than none:
# it reads as an open gap over work already done. So every field a follow-up names must
# still be unsupported where it says it is.
for line in follow_ups.splitlines():
    entry = re.match(r"^Unsupported fields: `([\w-]+)` (.+)$", line.strip())
    if entry is None:
        continue
    crate, spelled = entry.group(1), entry.group(2)
    kind = next(
        (kind for kind, (path, _) in PLUGINS.items() if path.startswith(f"crates/{crate}/")),
        None,
    )
    if kind is None:
        problems.append(
            f"docs/follow-ups.md names `{crate}`, which is not a crate this check knows "
            "a plugin in."
        )
        continue
    for field in re.findall(r"`(\w+)`", spelled):
        if field not in unsupported.get(kind, set()):
            problems.append(
                f"docs/follow-ups.md still records `{field}` of `{crate}` as unsupported, "
                "but that plugin now declares it supported."
            )

if problems:
    for problem in problems:
        print(f"{NAME}: {problem}", file=sys.stderr)
    print(
        f"{NAME}: bring the plugin's verdict table, its `capabilities` declaration and "
        "docs/follow-ups.md to one answer — a capability implemented is a verdict, a "
        "declaration and a follow-up entry that all change together — then re-run "
        "'just check'.",
        file=sys.stderr,
    )
    raise SystemExit(1)
PY
