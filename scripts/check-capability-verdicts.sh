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
# A plugin whose capabilities are configured per source has no value of its own to
# reconcile, so its verdicts are Supported by construction — except for a field it FIXES
# rather than configures, which the map below names with its reason and which is then held
# to everything a literal plugin's field is held to.
#
# So the verdicts are not prose alone. This reads them out of each plugin's own
# documentation and reconciles them three ways: against the contract's field list, against
# the plugin's own declaration, and — for a field recorded unsupported — against
# docs/follow-ups.md. That last reconciliation runs both ways, because both halves fail
# silently: a capability nobody implemented with no follow-up is a gap read as a limit and
# left forever, and a follow-up describing work already done is an open gap over closed
# work. An unsupported verdict therefore says which it is, in one word — `unimplemented`
# owes an entry there and `unsupportable` must not have one.
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

# The fields a `configured` plugin FIXES rather than reads from its configuration, each
# with the reason. `configured` above means "every field is whatever something else
# decided, so there is no value of this plugin's own to reconcile" — and one field of
# `in-memory` is not that, which would otherwise leave two bad options: a verdict row
# claiming a capability the plugin does not have, or a configuration key that could claim
# one it cannot serve.
#
# A fixed field is held to exactly what a `literal` plugin's field is held to: its verdict
# must be Unsupported, must say `unimplemented` or `unsupportable`, and an unimplemented one
# must be tracked in docs/follow-ups.md. What is not reconciled here is the declaration
# itself, because it is not in the file this check reads — the shared journey
# `every_row_declares_exactly_what_its_plugin_reports` is what holds `in-memory`'s real
# declaration against the table, at the boundary a user reads it from.
FIXED = {
    ("in-memory", "documents"): (
        "this source holds no documents at all, so a `CapabilityConfig` key that could "
        "declare it native would let a configuration claim something no code there can "
        "serve — which is capability rule 1's failure, and the one this product has "
        "already shipped once. The value is fixed in `impl From<&CapabilityConfig> for "
        "Capabilities` (crates/onetaskgraph-in-memory/src/config.rs)"
    ),
}

HEADING = "# What this source declares, field by field"
VERDICT_ROW = re.compile(r"^//!\s*\|\s*`(\w+)`\s*\|\s*\*\*(Supported|Unsupported)(.*)$")
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
    refuse(
        "`Capabilities` parsed with no fields at all, so there is nothing to reconcile "
        "any plugin's verdicts against.",
        "restore the `pub <field>: <type>` lines of `pub struct Capabilities` in "
        "crates/onetaskgraph-plugin-api/src/capability.rs, or — if that struct has been "
        "reshaped deliberately — teach this check the new shape.",
    )

as_str = re.search(
    r"pub fn as_str\(self\) -> &'static str \{(.*?)\n    \}", registry_source, re.DOTALL
)
if as_str is None:
    refuse(
        "could not find `PluginKind::as_str` in the plugin registry, which is where this "
        "check learns which plugins owe a verdict table.",
        "restore that function in crates/onetaskgraph-core/src/registry.rs, or — if the "
        "registry now names its kinds elsewhere — point this check at that instead.",
    )
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

for (kind, field), reason in sorted(FIXED.items()):
    if kind not in PLUGINS:
        refuse(
            f"FIXED names the plugin `{kind}`, which this check has no verdict table for.",
            "correct the plugin name, or drop the entry.",
        )
    if PLUGINS[kind][1] != "configured":
        refuse(
            f"FIXED names `{field}` of `{kind}`, which is not a `configured` plugin — a "
            "literal plugin's fields are already reconciled against its declaration.",
            "drop the entry.",
        )
    if field not in contract_fields:
        refuse(
            f"FIXED names `{field}`, which is not a field of the contract's `Capabilities`.",
            "correct the field name, or drop the entry.",
        )
    if not reason.strip():
        refuse(
            f"FIXED carries `{field}` of `{kind}` with no reason.",
            "say why that plugin fixes the field rather than configuring it, or drop the "
            "entry.",
        )

problems = []
unsupported = {}
# Every field declared unsupported, and the verdict text that says why.
owed = {}

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
    reasons = {}
    for line in source.splitlines():
        row = VERDICT_ROW.match(line)
        if row is not None:
            verdicts[row.group(1)] = row.group(2)
            reasons[row.group(1)] = row.group(3).lower()

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
            if (kind, field) not in FIXED:
                if verdict != "Supported":
                    problems.append(
                        f"{kind}: `{field}` is recorded {verdict}, but this plugin reports "
                        "whatever its configuration or its hosted source declares rather "
                        "than a value of its own. If this plugin fixes this field instead, "
                        "record it in FIXED in this script with the reason."
                    )
            elif verdict != "Unsupported":
                problems.append(
                    f"{kind}: FIXED records `{field}` as a value this plugin fixes rather "
                    f"than configures, and a fixed field is fixed at Unsupported — but its "
                    f"verdict reads {verdict}. Drop the FIXED entry if this plugin now "
                    "configures the field."
                )
            else:
                unsupported.setdefault(kind, set()).add(field)
                owed[(kind, field)] = reasons.get(field, "")
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
            owed[(kind, name)] = reasons.get(name, "")

def crate_of(kind):
    """The crate directory the plugin `kind` lives in."""
    return PLUGINS[kind][0].split("/")[1]


# Which (plugin, field) pairs docs/follow-ups.md says are tracked.
tracked = set()
for line in follow_ups.splitlines():
    entry = re.match(r"^Unsupported fields: `([\w-]+)` (.+)$", line.strip())
    if entry is None:
        continue
    crate, spelled = entry.group(1), entry.group(2)
    kind = next((kind for kind in PLUGINS if crate_of(kind) == crate), None)
    if kind is None:
        problems.append(
            f"docs/follow-ups.md names `{crate}`, which is not a crate this check knows "
            "a plugin in."
        )
        continue
    for field in re.findall(r"`(\w+)`", spelled):
        tracked.add((kind, field))

# Both directions. A follow-up describing a capability that has since been implemented is
# an open gap over closed work; an unimplemented capability with no follow-up is a gap
# nothing will ever come back to. Which of the two an unsupported verdict is has to be
# stated, in the verdict itself, or neither direction can be checked at all.
for kind, field in sorted(tracked):
    if field not in unsupported.get(kind, set()):
        problems.append(
            f"docs/follow-ups.md still records `{field}` of `{crate_of(kind)}` as "
            "unsupported, but that plugin now declares it supported."
        )

for (kind, field), reason in sorted(owed.items()):
    crate, path = crate_of(kind), PLUGINS[kind][0]
    says_unimplemented = "unimplemented" in reason
    says_unsupportable = "unsupportable" in reason
    if says_unimplemented == says_unsupportable:
        problems.append(
            f"{kind}: `{field}` is recorded Unsupported in {path} without saying which "
            "it is — a verdict must call it `unimplemented` or `unsupportable`, and "
            "exactly one of the two."
        )
    elif says_unimplemented and (kind, field) not in tracked:
        problems.append(
            f"{kind}: `{field}` is recorded Unsupported and unimplemented, but no "
            "`Unsupported fields:` line in docs/follow-ups.md names it — an unimplemented "
            "capability nobody tracks is a gap that will be read as a limit."
        )
    elif says_unsupportable and (kind, field) in tracked:
        problems.append(
            f"{kind}: `{field}` is recorded Unsupported and unsupportable, so "
            "docs/follow-ups.md must not track it as work owed."
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
