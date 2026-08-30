#!/usr/bin/env bash
# Refuse a store fixture under which the wrong answer and the right answer read alike.
#
# Both defects repaired in this repository's last release were invisible under the fixtures
# that existed. One wrote a project's *id* into the field its *title* belongs in; the other
# discarded a project query outright. Neither test could have failed: every fixture held one
# project, and every native id was its own title and its own file stem, so `id` and `title`
# were the same string and "all the projects" and "the one project the query selects" were
# the same row.
#
# So a fixture that stands in for a store owes two properties, and this check reads them
# back off the fixture rather than trusting that somebody kept them:
#
#   1. **Discriminating identity.** At least one item whose identifier differs from its
#      title, so writing one where the other belongs changes what a test sees.
#   2. **Discriminating filters.** At least two distinct values of everything the code under
#      test filters on, so a filter that is dropped, inverted or ignored returns a different
#      set than one that is applied.
#
# The manifest below says, per fixture, where its items are and what the code that reads it
# filters on. A fixture with no entry is not silently skipped: every JSON file under a
# `tests/fixtures` directory must be named here or recorded as not standing in for a store,
# so a new one cannot arrive unchecked.
set -euo pipefail

fatal() {
  echo "check-store-fixtures: $1" >&2
  echo "check-store-fixtures: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just check' does"
readonly ROOT
cd "$ROOT" || fatal \
  "could not enter $ROOT to read its fixtures" \
  "check that directory's permissions, then rerun"

# A refusal of its own rather than python's: an unhandled exception exits 1 too, so a
# refusal spelled 1 would be indistinguishable from the scan having died partway through.
status=0
python3 - <<'PY' || status=$?
import json
import pathlib
import sys

# Where each store fixture keeps its items, and what the code that reads it filters on.
#
# `items` is the path to the list of things the fixture stands in for. `id` and `title` are
# paths inside one item. `facets` names each thing a query over this store selects by, with
# the path to its value inside an item — a path reaching a list contributes every element.
# A path segment of `[]` walks into a list.
MANIFEST = {
    "crates/onetaskgraph-linear/tests/fixtures/issues.json": {
        "items": ["data", "issues", "nodes"],
        "id": ["id"],
        "title": ["title"],
        "facets": {
            "status": ["state", "name"],
            "label": ["labels", "nodes", [], "name"],
            "project": ["project", "id"],
        },
    },
    "crates/onetaskgraph-linear/tests/fixtures/projects.json": {
        "items": ["data", "projects", "nodes"],
        "id": ["id"],
        "title": ["name"],
        "facets": {
            "status": ["status", "name"],
            "label": ["labels", "nodes", [], "name"],
        },
    },
    "crates/onetaskgraph-linear/tests/fixtures/labels.json": {
        "items": ["data", "issueLabels", "nodes"],
        "id": ["id"],
        "title": ["name"],
        # The list IS the label table, so the label a query filters by is the item here and
        # rule 2 is the count of items rather than a facet inside one.
        "facets": {},
        "least": 2,
    },
    "crates/onetaskgraph-github-projects/tests/fixtures/project.json": {
        "items": ["data", "owner", "projectV2", "items", "nodes"],
        "id": ["content", "id"],
        "title": ["content", "title"],
        "facets": {
            "status": ["fieldValues", "nodes", [], "name"],
            "label": ["content", "labels", "nodes", [], "name"],
            "project": ["content", "parent", "id"],
        },
    },
    "crates/onetaskgraph-linear/tests/fixtures/issue-relations.json": {
        "items": ["data", "issue", "relations", "nodes"],
        "id": ["relatedIssue", "id"],
        # A relation's far end is read for its id and its direction, never for a title.
        "title": None,
        "facets": {},
        "least": 1,
    },
    "crates/onetaskgraph-linear/tests/fixtures/project-relations.json": {
        "items": ["data", "project", "relations", "nodes"],
        "id": ["relatedProject", "id"],
        "title": None,
        "facets": {},
        "least": 1,
    },
    "crates/onetaskgraph-github-projects/tests/fixtures/dependencies.json": {
        "items": ["data", "node", "blockedBy", "nodes"],
        "id": ["id"],
        # A far end is read for its id and its kind, never for a title, so there is no
        # title here to differ from the id and rule 1 does not reach this fixture.
        "title": None,
        "facets": {},
        "least": 1,
    },
}

# Files under a `tests/fixtures` directory that do not stand in for a store, each with the
# reason. Recorded rather than skipped by pattern, so a store fixture cannot be added under
# a name a pattern happens to exclude.
NOT_A_STORE = {}

root = pathlib.Path(".")
problems = []


def refuse(problem, action):
    print(f"check-store-fixtures: {problem}", file=sys.stderr)
    print(f"check-store-fixtures: next: {action}", file=sys.stderr)
    raise SystemExit(1)


def walk(value, path):
    """Every value `path` reaches, which is more than one where the path enters a list."""
    if not path:
        return [value] if value is not None else []
    head, rest = path[0], path[1:]
    if head == []:
        if not isinstance(value, list):
            return []
        return [reached for element in value for reached in walk(element, rest)]
    if not isinstance(value, dict) or head not in value:
        return []
    return walk(value[head], rest)


def one(value, path):
    """The single value `path` reaches, or `None`."""
    reached = walk(value, path)
    return reached[0] if len(reached) == 1 else None


discovered = sorted(
    str(path)
    for path in root.glob("crates/*/tests/fixtures/*.json")
)
if not discovered:
    refuse(
        "found no fixture files at all under crates/*/tests/fixtures/.",
        "restore them, or teach this check where they moved to — an empty set would make "
        "it pass on anything.",
    )

for path in discovered:
    if path not in MANIFEST and path not in NOT_A_STORE:
        problems.append(
            f"{path} is a fixture this check has never been told about. Add it to MANIFEST "
            "in scripts/check-store-fixtures.sh with the two properties a store fixture "
            "owes, or to NOT_A_STORE with why it stands in for something else."
        )

for path, spec in sorted(MANIFEST.items()):
    file = root / path
    if not file.is_file():
        problems.append(
            f"{path} is named in MANIFEST but is not there. Restore it, or drop its entry "
            "from scripts/check-store-fixtures.sh."
        )
        continue
    try:
        document = json.loads(file.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        problems.append(f"{path} could not be read as JSON: {error}.")
        continue

    items = walk(document, spec["items"])
    items = items[0] if len(items) == 1 and isinstance(items[0], list) else items
    if not isinstance(items, list) or not items:
        problems.append(
            f"{path} has no items at {'/'.join(str(step) for step in spec['items'])}, so "
            "every property below would be satisfied by there being nothing to check."
        )
        continue

    least = spec.get("least", 2)
    if len(items) < least:
        problems.append(
            f"{path} holds {len(items)} item(s) and a store fixture needs at least "
            f"{least}: with fewer, a query that selects one and a query that selects all "
            "of them answer with the same rows."
        )

    if spec["title"] is not None:
        discriminating = [
            item
            for item in items
            if one(item, spec["id"]) is not None
            and one(item, spec["title"]) is not None
            and one(item, spec["id"]) != one(item, spec["title"])
        ]
        if not discriminating:
            problems.append(
                f"{path} carries no item whose identifier differs from its title, so "
                "writing the identifier where the title belongs is a change no assertion "
                "over this fixture can see. Give at least one item a title that is not its "
                "id."
            )

    for facet, facet_path in sorted(spec["facets"].items()):
        values = {
            json.dumps(value, sort_keys=True) for value in walk(document, spec["items"] + [[]] + facet_path)
        }
        if len(values) < 2:
            spelled = "/".join(str(step) for step in facet_path)
            problems.append(
                f"{path} carries {len(values)} distinct {facet} value(s) at {spelled}, and "
                "the code that reads it filters by that: with one value, a filter that is "
                "applied and a filter that is dropped return the same rows. Give it at "
                "least two."
            )

if problems:
    for problem in problems:
        print(f"check-store-fixtures: {problem}", file=sys.stderr)
    print(
        "check-store-fixtures: next: enrich the fixture(s) above until the wrong answer and "
        "the right answer stop reading alike.",
        file=sys.stderr,
    )
    raise SystemExit(1)
PY

exit "$status"
