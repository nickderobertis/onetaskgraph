#!/usr/bin/env bash
# Drift gate for release-targets.toml: the shape it is written in, and whether it
# still says what this repository really publishes.
#
# A consumer sequencing work across repositories reads this document to learn
# which artifact to wait on, and it reads it without knowing anything else about
# this repository. Three things can go wrong with that, and this holds all three.
#
# **The shape.** The document is the canonical release-target schema, which
# nickderobertis/onevcs defines in its docs/contract.md and reads with `onevcs
# release declaration`. Where that reader is installed this loads the real file
# back through it — the one authority on whether a consumer can read this
# document at all. Where it is not, the restatement below holds the document to
# the same shape, so a malformed declaration cannot land on a machine that does
# not carry the reader. `ONETASKGRAPH_RELEASE_READER_REQUIRED=1` turns the skip
# into a failure, which is the pairing this repository already gives a check whose
# third-party input may be absent.
#
# **The short names.** `crate`, `pypi`, `sdk-pypi`, `npm` and `sdk-npm` are how a
# consumer in another repository names one of these artifacts, and that consumer
# cannot see this file to notice one moved. So the map from short name to
# identifier is spelled here as well as there, deliberately: this is the drift
# gate that makes the second spelling safe, and a rename that does not come
# through both is a rename that silently breaks a wait somewhere else.
#
# **The contents.** A hand-written inventory is exactly what goes stale in
# silence — a repository publishing something it declares no target for grants no
# hold at all, and nobody learns the hold stopped happening. So the published set
# is DERIVED from the release configuration itself rather than transcribed:
#
#   crates — the crate names the publish-crates job of .github/workflows/release.yml
#            iterates over.
#   pypi   — the project names the publish-python job passes `uv publish --check-url`.
#   npm    — the specs scripts/publish-npm.sh publishes, plus the per-platform
#            carrier manifests under npm/platforms/ that it sends first.
#
# It fails in both directions: a name this repository publishes that no target
# declares or covers, and a name declared or covered that it does not publish.
#
# Quiet on success. On failure it names each drift and the fix.
set -euo pipefail

fatal() {
  echo "check-release-targets: $1" >&2
  echo "check-release-targets: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run this from a checkout of this repository, as 'just distribution-check' does"
readonly ROOT
cd "$ROOT" || fatal "could not enter $ROOT" "check that directory's permissions, then rerun"

# Everything this check writes goes here rather than into the tree it is checking:
# a gate that leaves a file behind is a gate that fails the next thing to read the
# working tree.
work="$(mktemp -d)" || fatal \
  "could not create a working directory" \
  "check the temporary directory's permissions and free space, then rerun"
trap 'rm -rf "$work"' EXIT

# The reader that actually consumes this document, run over the real file. It is
# the whole point of writing one, so where it is installed it is not optional.
reader_status=0
if command -v onevcs >/dev/null 2>&1; then
  if ! onevcs release declaration "$ROOT" --json > "$work/declaration.json" 2> "$work/declaration.err"; then
    cat "$work/declaration.err" >&2
    fatal "the canonical reader refused release-targets.toml (its refusal is above)" \
      "fix the field it names; a document this reader refuses is one a consumer cannot read at all"
  fi
elif [ "${ONETASKGRAPH_RELEASE_READER_REQUIRED:-0}" = 1 ]; then
  fatal "onevcs is not on PATH and ONETASKGRAPH_RELEASE_READER_REQUIRED=1 asked for it" \
    "install onevcs (cargo install onevcs, or pip install onevcs-cli) and rerun"
else
  reader_status=1
  echo "check-release-targets: onevcs is not on PATH; the canonical reader did not load this document (set ONETASKGRAPH_RELEASE_READER_REQUIRED=1 to make that a failure)" >&2
fi

python3 - "$reader_status" "$work/declaration.json" <<'PY' || fatal \
  "release-targets.toml no longer matches this repository (each drift is above)" \
  "declare the artifact this repository publishes as a [[target]] or a covers entry, or remove the target for what it no longer publishes — then rerun"
import json
import re
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python older than 3.11
    print("  this python3 has no tomllib, so release-targets.toml cannot be read", file=sys.stderr)
    print("  install Python 3.11 or newer and rerun", file=sys.stderr)
    raise SystemExit(1)

reader_ran = sys.argv[1] == "0"
READER_OUTPUT = Path(sys.argv[2])
DECLARATION = Path("release-targets.toml")
WORKFLOW = Path(".github/workflows/release.yml")
NPM_PUBLICATION = Path("scripts/publish-npm.sh")

# The canonical schema, version 2, restated. Every key it declares, per table:
# a key nobody declared is the finding, because a misspelled `manifset` read as
# an absent `manifest` publishes an answer nobody wrote.
# llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] The schema is
# defined by the `onevcs` crate, which is this repository's CONSUMER rather than
# its dependency — taking it as one to read the definition would invert that, and
# `just check` is offline and pinned, so it cannot be fetched at check time
# either. What keeps the restatement honest is two things: `schema_version`, read
# and refused by number here, so a document brought up to a later schema goes red
# until this is brought up with it; and the real reader above, which this defers
# to wherever it is installed.
SCHEMA_VERSION = 2
TOP_KEYS = {"schema_version", "probe"}
TARGET_KEYS = {"id", "name", "what", "published_by", "manifest", "covers"}
REQUIRED_TARGET_KEYS = {"id", "name", "what", "published_by"}
RETIRED_KEYS = {"id", "why"}
ID_SYNTAX = re.compile(
    r"^[a-z0-9-]+:(@[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9][A-Za-z0-9._-]*|[A-Za-z0-9][A-Za-z0-9._@/-]*)$"
)
NAME_SYNTAX = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
MAX_PROSE = 400
# llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

# The short names other repositories wait on, and the artifact each one is. This
# is the second spelling the header explains: a consumer names `sdk-pypi` in its
# own plan and cannot see this file, so a short name that moved without moving
# here is a wait that resolves against nothing.
EXPECTED_NAMES = {
    "crate": "crate:onetaskgraph",
    "pypi": "pypi:onetaskgraph-cli",
    "sdk-pypi": "pypi:onetaskgraph-sdk",
    "npm": "npm:@onetaskgraph/cli",
    "sdk-npm": "npm:@onetaskgraph/sdk",
}

problems = []


def read(path, what):
    """One input file, reported by name rather than as a traceback if it will not open."""
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as problem:
        print(f"  could not read {path}: {problem}", file=sys.stderr)
        print(f"  restore {what} and rerun", file=sys.stderr)
        raise SystemExit(1)


try:
    with open(DECLARATION, "rb") as handle:
        declared = tomllib.load(handle)
except (OSError, tomllib.TOMLDecodeError) as problem:
    print(f"  {DECLARATION} could not be read as TOML: {problem}", file=sys.stderr)
    print("  fix that file; it is the one thing a consumer reads to learn what to wait on", file=sys.stderr)
    raise SystemExit(1)

# ---------------------------------------------------------------------------
# The shape.
# ---------------------------------------------------------------------------
version = declared.get("schema_version")
if version != SCHEMA_VERSION:
    problems.append(
        f"{DECLARATION} declares schema_version {version!r}; this check is written against "
        f"{SCHEMA_VERSION}. Bring the restatement in this script up to the version the "
        "document declares, or put the document back on the one it was written for."
    )

for key in sorted(set(declared) - TOP_KEYS - {"target", "retired"}):
    problems.append(f"{DECLARATION} carries the top-level key '{key}', which schema version {SCHEMA_VERSION} does not declare")

# The document is input rather than something this wrote, so every value is held
# to the type the schema gives it before anything reads it as one. Refused here
# rather than dereferenced: `name = ["crate"]` would otherwise reach a comparison
# as a list and `manifest = 1` a path as an integer, and each would end this check
# on a traceback naming neither the file nor the field.
def typed(value, kind, where, key):
    """The value, when it has the type the schema gives it; None once refused."""
    if isinstance(value, kind) and not (kind is int and isinstance(value, bool)):
        return value
    problems.append(
        f"{where} holds {key} as {type(value).__name__}, and schema version "
        f"{SCHEMA_VERSION} gives it {kind.__name__}"
    )
    return None


targets = declared.get("target", [])
if not isinstance(targets, list):
    problems.append(f"{DECLARATION}'s target is not the array of tables the schema declares")
    targets = []
if not targets:
    problems.append(
        f"{DECLARATION} declares no [[target]] at all, which a consumer cannot tell from "
        "nobody having said anything"
    )

seen_names, seen_ids, covered = {}, {}, {}
for position, target in enumerate(list(targets), start=1):
    where = f"{DECLARATION} [[target]] #{position}"
    if not isinstance(target, dict):
        problems.append(f"{where} is not a table")
        continue
    for key in sorted(set(target) - TARGET_KEYS):
        problems.append(f"{where} carries the key '{key}', which schema version {SCHEMA_VERSION} does not declare")
    for key in sorted(REQUIRED_TARGET_KEYS - set(target)):
        problems.append(f"{where} is missing the required key '{key}'")
    identifier = typed(target.get("id"), str, where, "id") if "id" in target else None
    name = typed(target.get("name"), str, where, "name") if "name" in target else None
    if isinstance(identifier, str):
        if not ID_SYNTAX.match(identifier):
            problems.append(f"{where} has the id '{identifier}', which is not <registry>:<name>")
        if identifier in seen_ids:
            problems.append(f"{where} takes the id '{identifier}', which [[target]] #{seen_ids[identifier]} already has")
        seen_ids[identifier] = position
    if isinstance(name, str):
        if not NAME_SYNTAX.match(name):
            problems.append(f"{where} has the short name '{name}', which is not a target name")
        if name in seen_names:
            problems.append(f"{where} takes the short name '{name}', which [[target]] #{seen_names[name]} already has")
        seen_names[name] = position
    for key in ("what", "published_by"):
        if key not in target:
            continue
        value = typed(target.get(key), str, where, key)
        if isinstance(value, str) and (not value.strip() or "\n" in value or len(value) > MAX_PROSE):
            problems.append(f"{where}'s {key} is not one non-blank line of at most {MAX_PROSE} characters")
    for entry in typed(target.get("covers", []), list, where, "covers") or []:
        if not isinstance(entry, str) or not ID_SYNTAX.match(entry):
            problems.append(f"{where} covers '{entry}', which is not <registry>:<name>")
            continue
        if entry in covered:
            problems.append(f"{where} covers '{entry}', which [[target]] #{covered[entry]} already covers")
        covered[entry] = position
    if "manifest" in target:
        value = typed(target.get("manifest"), str, where, "manifest")
        if not isinstance(value, str):
            pass
        elif value.startswith("/") or value.startswith("\\") or ".." in value.replace("\\", "/").split("/") or re.match(r"^[A-Za-z]:", value):
            problems.append(f"{where}'s manifest '{value}' is not a path inside this repository")
        elif not Path(value).is_file():
            problems.append(f"{where} names the manifest '{value}', which this repository does not carry")

for entry, position in sorted(covered.items()):
    if entry in seen_ids:
        problems.append(f"{DECLARATION} both declares '{entry}' as a target and covers it under [[target]] #{position}")

for position, retired in enumerate(declared.get("retired", []) or [], start=1):
    if not isinstance(retired, dict):
        problems.append(f"{DECLARATION} [[retired]] #{position} is not a table")
        continue
    for key in sorted(set(retired) - RETIRED_KEYS):
        problems.append(f"a [[retired]] entry carries the key '{key}', which schema version {SCHEMA_VERSION} does not declare")
    for key in sorted(RETIRED_KEYS - set(retired)):
        problems.append(f"a [[retired]] entry is missing the required key '{key}'")
    if retired.get("id") in seen_ids:
        problems.append(f"{DECLARATION} retires '{retired['id']}' and declares it as a target")

probe = declared.get("probe")
if probe is not None and not isinstance(probe, str):
    problems.append(f"{DECLARATION}'s probe is a {type(probe).__name__} where the schema gives it a path")
    probe = ""
if probe is None:
    problems.append(
        f"{DECLARATION} names no probe, so every target it declares is one nothing can ask "
        "a registry about"
    )
elif not Path(probe).is_file():
    problems.append(f"{DECLARATION} names the probe '{probe}', which this repository does not carry")

# The short names, in both directions.
for name, identifier in EXPECTED_NAMES.items():
    position = seen_names.get(name)
    if position is None:
        problems.append(
            f"{DECLARATION} declares no target named '{name}'. Another repository waits on that "
            "short name and cannot see this file; if it really moved, move it in both places."
        )
    elif targets[position - 1].get("id") != identifier:
        problems.append(
            f"{DECLARATION} gives the short name '{name}' the id "
            f"'{targets[position - 1].get('id')}', and a consumer waiting on '{name}' expects "
            f"'{identifier}'"
        )
for name in sorted(set(seen_names) - set(EXPECTED_NAMES)):
    problems.append(
        f"{DECLARATION} declares the short name '{name}', which this check does not know. A new "
        "target is a new thing consumers may wait on: add it here in the same change."
    )

# ---------------------------------------------------------------------------
# The contents, derived from the real release configuration.
# ---------------------------------------------------------------------------
workflow = read(WORKFLOW, "the release workflow")
published = {}


def publishes(identifier, where):
    published.setdefault(identifier, where)


crate_line = re.search(r"for crate in ([^;]+); do", workflow)
if crate_line is None:
    problems.append(
        f"{WORKFLOW}'s publish-crates job no longer iterates a crate list this can read, so "
        "what it publishes cannot be derived"
    )
else:
    for crate in crate_line.group(1).split():
        publishes(f"crate:{crate}", f"{WORKFLOW}'s publish-crates job")

pypi_projects = re.findall(r"uv publish --check-url https://pypi\.org/simple/([^/]+)/", workflow)
if not pypi_projects:
    problems.append(
        f"{WORKFLOW}'s publish-python job names no project through `uv publish --check-url`, so "
        "what it publishes cannot be derived"
    )
for project in pypi_projects:
    publishes(f"pypi:{project}", f"{WORKFLOW}'s publish-python job")

npm_publication = read(NPM_PUBLICATION, "the npm publication")
npm_specs = re.findall(r'publish_if_absent "([^"@][^"]*|@[^"]+)@\$[A-Za-z_]+"', npm_publication)
if not npm_specs:
    problems.append(
        f"{NPM_PUBLICATION} publishes no package this can read, so what reaches npm cannot be derived"
    )
for spec in npm_specs:
    publishes(f"npm:{spec}", NPM_PUBLICATION)

carriers = sorted(Path("npm/platforms").glob("*/package.json"))
if not carriers:
    problems.append("npm/platforms carries no package manifest, so the per-platform packages cannot be derived")
for carrier in carriers:
    try:
        publishes(f"npm:{json.loads(carrier.read_text(encoding='utf-8'))['name']}", carrier)
    except (OSError, ValueError, KeyError) as problem:
        problems.append(f"{carrier} does not name a package this can read: {problem}")

accounted = set(seen_ids) | set(covered)
for identifier, where in sorted(published.items()):
    if identifier not in accounted:
        problems.append(
            f"{where} publishes '{identifier}', which {DECLARATION} neither declares nor covers — "
            "a consumer waiting on it has nothing to wait on"
        )
for identifier in sorted(accounted - set(published)):
    problems.append(
        f"{DECLARATION} names '{identifier}', which nothing in this repository publishes — a "
        "consumer would wait for a release that never comes"
    )

# A declared target's manifest must be the manifest its own name comes from.
MANIFEST_NAME = {
    ".toml": lambda document: (document.get("package") or document.get("project") or {}).get("name"),
    ".json": lambda document: document.get("name"),
}
for target in targets:
    if not isinstance(target, dict):
        continue
    manifest, identifier = target.get("manifest"), target.get("id")
    # Both are refused by name above where they are not strings; this reconciles
    # the ones that are, rather than reading a non-path as one.
    if not isinstance(manifest, str) or not isinstance(identifier, str) or not Path(manifest).is_file():
        continue
    suffix = Path(manifest).suffix
    try:
        if suffix == ".toml":
            with open(manifest, "rb") as handle:
                document = tomllib.load(handle)
        else:
            document = json.loads(Path(manifest).read_text(encoding="utf-8"))
    except (OSError, ValueError, tomllib.TOMLDecodeError) as problem:
        problems.append(f"{manifest}, which '{identifier}' names, could not be read: {problem}")
        continue
    name = MANIFEST_NAME.get(suffix, lambda _: None)(document)
    if name != identifier.partition(":")[2]:
        problems.append(
            f"{DECLARATION} says '{identifier}' takes its name and version from {manifest}, "
            f"which names '{name}'"
        )

if problems:
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    raise SystemExit(1)

# What the canonical reader saw, against what this read: the same targets, or the
# reader was answering about a different document.
if reader_ran:
    try:
        reported = json.loads(READER_OUTPUT.read_text(encoding="utf-8"))
        theirs = [(target["id"], target["name"]) for target in reported["target"]]
    except (OSError, ValueError, TypeError, KeyError) as problem:
        print(f"  the canonical reader answered something this cannot read: {problem}", file=sys.stderr)
        print("  run `onevcs release declaration . --json` and see what it printed", file=sys.stderr)
        raise SystemExit(1)
    mine = [(target["id"], target["name"]) for target in targets]
    if mine != theirs:
        print(f"  the canonical reader read {theirs} out of {DECLARATION}, and this read {mine}", file=sys.stderr)
        print("  the two disagree about what this repository declares; re-run `onevcs release declaration .`", file=sys.stderr)
        raise SystemExit(1)
PY
