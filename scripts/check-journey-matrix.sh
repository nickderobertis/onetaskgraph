#!/usr/bin/env bash
# Fail when a plugin the registry knows has no row in the journey table.
#
# Every journey this repository owes is written once and run against EVERY source kind,
# through one shared fixture table — so a plugin is never proven by a suite of its own
# writing. That only holds while the table actually covers the registry, and nothing makes
# it: a plugin lands, its own crate's tests go green, and the shared journeys quietly never
# run against it. Nobody finds out, because a table with a row missing looks exactly like a
# table.
#
# So the two are reconciled here, on every `just check`, in both directions. A plugin with
# no row fails, naming the plugin. A row naming a plugin the registry does not have fails
# too: that is a fixture for a kind no configuration can name, and it would sit there
# passing forever.
#
# A plugin whose source has not landed is expected to carry a `Fixture::Pending` row rather
# than no row at all, and that row is a journey of its own — it asserts the plugin refuses
# with its own message. This script does not distinguish the two, deliberately: what it
# guards is coverage of the registry, and which kind of row a plugin needs is the table's
# business.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

REGISTRY="$ROOT/crates/onetaskgraph-core/src/registry.rs" \
TABLE="$ROOT/crates/onetaskgraph/tests/e2e/fixtures.rs" \
python3 <<'PY'
import os
import re
import sys

registry_path = os.environ["REGISTRY"]
table_path = os.environ["TABLE"]

def read(path, what):
    """The file's text, or a named problem and a concrete next action.

    Every caller of this script is a guard, and a guard that dies on a traceback tells
    its reader the interpreter's story rather than which file to open.
    """
    try:
        return open(path, encoding="utf-8").read()
    except (OSError, UnicodeDecodeError) as error:
        print(f"check-journey-matrix: could not read {what}: {error}", file=sys.stderr)
        print(
            f"check-journey-matrix: restore {path} — it is where this check learns "
            "which plugins exist and which have journeys — then re-run 'just check'.",
            file=sys.stderr,
        )
        raise SystemExit(1) from None


registry_source = read(registry_path, "the plugin registry")
table_source = read(table_path, "the journey table")

# The registry spells each kind exactly once, in `PluginKind::as_str`. Anchoring on that
# function rather than on the whole file means a kind named in a doc comment is not
# mistaken for a registered one.
as_str = re.search(
    r"pub fn as_str\(self\) -> &'static str \{(.*?)\n    \}", registry_source, re.DOTALL
)
if as_str is None:
    print(
        "check-journey-matrix: could not find `PluginKind::as_str` in "
        f"{registry_path}.",
        file=sys.stderr,
    )
    print(
        "check-journey-matrix: that function is how this check learns which plugins the "
        "registry has — restore it, or update the anchor here if it was deliberately "
        "reshaped.",
        file=sys.stderr,
    )
    raise SystemExit(1)

registered = set(re.findall(r'=> "([a-z0-9-]+)"', as_str.group(1)))
covered = set(re.findall(r'plugin: "([a-z0-9-]+)"', table_source))

if not registered:
    print(
        f"check-journey-matrix: {registry_path} names no plugin at all, so this check has "
        "nothing to reconcile the journey table against.",
        file=sys.stderr,
    )
    print(
        "check-journey-matrix: restore the `=> \"kind\"` arms of `PluginKind::as_str` — "
        "one per plugin the binary can build — then re-run 'just check'.",
        file=sys.stderr,
    )
    raise SystemExit(1)
if not covered:
    print(
        f"check-journey-matrix: {table_path} has no rows — every journey runs against "
        "every source kind through that table, so an empty one runs them against nothing.",
        file=sys.stderr,
    )
    print(
        "check-journey-matrix: restore ROWS there — one `plugin: \"kind\"` entry per "
        f"plugin in {registry_path} — then re-run 'just check'.",
        file=sys.stderr,
    )
    raise SystemExit(1)

problems = []
for plugin in sorted(registered - covered):
    problems.append(
        f"{plugin}: the registry has this plugin and the journey table has no row for it. "
        f"Add one to ROWS in {table_path} — Fixture::Ready over the shared dataset once "
        "its source works, Fixture::Pending until then."
    )
for plugin in sorted(covered - registered):
    problems.append(
        f"{plugin}: the journey table has a row for a plugin the registry does not have, "
        "so no configuration can name it and those journeys prove nothing. Remove the "
        f"row from {table_path}, or register the plugin in {registry_path}."
    )

if problems:
    print(
        "check-journey-matrix: the journey table and the plugin registry disagree.",
        file=sys.stderr,
    )
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    raise SystemExit(1)
PY
