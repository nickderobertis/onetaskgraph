#!/usr/bin/env bash
# Drift gate for release-targets.toml: the shape it is written in, and whether it
# still says what this repository really publishes. Three things can go wrong for a
# consumer reading that document, and this holds all three.
#
# **The shape.** It is the canonical release-target schema, which
# nickderobertis/onevcs defines and reads with `onevcs release declaration`. This
# loads the real file back through that reader — the one authority on whether a
# consumer can read it at all — running a PINNED reader rather than whichever build
# a machine puts first on PATH, because a verdict that follows PATH order is not a
# verdict about this repository. Where none can be resolved, the restatement below
# holds the document to the same shape, and
# `ONETASKGRAPH_RELEASE_READER_REQUIRED=1` turns that skip into a failure.
#
# **The short names.** They are what a consumer in another repository names, and it
# cannot see this file to notice one moved or a sixth appeared. So the map is
# spelled here as well as there, deliberately, and refused in either direction.
#
# **The contents.** A hand-written inventory goes stale in silence, so the published
# set is DERIVED from the release configuration rather than transcribed:
#
#   crates — the crate names the publish-crates job of .github/workflows/release.yml
#            iterates over.
#   pypi   — the project names the publish-python job passes `uv publish --check-url`.
#   npm    — the specs scripts/publish-npm.sh publishes, plus the per-platform
#            carrier manifests under npm/platforms/ that it sends first.
#
# It fails both ways: a name published that no target declares or covers, and a name
# declared or covered that nothing publishes.
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

# The one switch this check reads from the environment, held to the three values
# it has before anything branches on it — the same three
# `ONETASKGRAPH_LIVE_REQUIRED` is held to in
# crates/onetaskgraph-github-projects/tests/lane/mod.rs, and for the same reason.
# Every other value read as a plain `= 1` comparison means not-required, so a
# caller that asked for the canonical reader with `=true` or `=yes` would get the
# skip it was trying to turn off, and be told nothing.
reader_required="${ONETASKGRAPH_RELEASE_READER_REQUIRED:-0}"
case "$reader_required" in
  0 | 1) ;;
  *)
    fatal "ONETASKGRAPH_RELEASE_READER_REQUIRED is '$reader_required', and it must be 1, 0 or unset" \
      "set it to 1 to require the canonical reader, or unset it to let this skip where none can be resolved"
    ;;
esac

# The canonical reader, pinned and resolved deterministically rather than taken
# from whatever `onevcs` a machine happens to put first on PATH.
#
# This host carries five `onevcs` builds and the check's verdict followed PATH
# order between them, which is not a property of this repository at all. Worse
# than arbitrary: `onevcs` gained the npm scoped form in 0.16.1 — "accept an npm
# scoped name as a registry identifier", its CHANGELOG — and every build before it
# refuses `npm:@onetaskgraph/cli` as a name no registry serves, at every
# schema_version, whatever else the document says. So an older build on PATH
# reported this repository's scoped identifiers as defects in a document that is
# exactly what the schema documents, and pointed whoever read it at the wrong
# file. This repository really publishes `@onetaskgraph/cli`, its five carriers and
# `@onetaskgraph/sdk`; the scoped spelling is not the declaration's to give up, so
# the reader is what has to be nailed down.
#
# `uvx` runs one pinned version from the uv cache — offline, in about a quarter of
# a second, and identically on every machine that has uv, which this repository
# already requires for its Python workspace. That is the mechanism; the PATH build
# is the fallback for a machine without uv, and it is used only if it proves it can
# read what this document contains.
readonly READER_PACKAGE="onevcs-cli"
# The version this check was written and verified against. Moving it means running
# this check against the new build: it is the authority on whether a consumer can
# read release-targets.toml, so a version nobody watched read it is a version this
# repository has no evidence about.
readonly READER_VERSION="0.18.0"

# The argv prefix that runs the reader, and a phrase naming where it came from for
# the diagnostics below. Empty when no reader could be resolved at all.
reader_argv=""
reader_origin=""

# Ask a candidate whether it reads an npm scoped name, with a document that carries
# one and nothing else. A behaviour rather than a version comparison: a build's
# number is a second copy of a third party's history that goes stale in silence,
# and what this needs to know is not which release something is but whether it can
# read a name this repository really publishes under.
mkdir -p "$work/capability" || fatal \
  "could not write the reader's capability probe under $work" \
  "check the temporary directory's permissions and free space, then rerun"
cat > "$work/capability/release-targets.toml" <<'PROBE' || fatal \
  "could not write the reader's capability probe document" \
  "check the temporary directory's permissions and free space, then rerun"
schema_version = 2
[[target]]
id = "npm:@scope/name"
name = "scoped"
what = "A probe document, carrying one npm scoped identifier and nothing else."
published_by = "Nothing publishes it. It exists to ask a reader whether it reads a scoped name."
PROBE

reads_scoped_names() {
  "$@" release declaration "$work/capability" --json >/dev/null 2>&1
}

# The pin first, so the answer is the same on every machine that can produce it.
# `--offline` deliberately: a required check does not reach the network, so this
# uses the uv cache or steps aside for the fallback below.
if command -v uvx >/dev/null 2>&1 &&
  reads_scoped_names uvx --offline --from "$READER_PACKAGE==$READER_VERSION" onevcs; then
  reader_argv="uvx --offline --from $READER_PACKAGE==$READER_VERSION onevcs"
  reader_origin="the pinned $READER_PACKAGE $READER_VERSION, run from the uv cache"
elif command -v onevcs >/dev/null 2>&1 && reads_scoped_names onevcs; then
  reader_argv="onevcs"
  reader_origin="the onevcs on PATH ($(command -v onevcs), $(onevcs --version 2>/dev/null || echo 'version unknown'))"
fi

# The reader that actually consumes this document, run over the real file. It is
# the whole point of writing one, so where one is available it is not optional.
reader_status=0
if [ -n "$reader_argv" ]; then
  # Unquoted on purpose: this is an argv prefix of several words, and every word in
  # it is a literal set above rather than anything read from the environment.
  # shellcheck disable=SC2086
  if ! $reader_argv release declaration "$ROOT" --json > "$work/declaration.json" 2> "$work/declaration.err"; then
    cat "$work/declaration.err" >&2
    fatal "the canonical reader refused release-targets.toml (its refusal is above; the reader was $reader_origin)" \
      "fix the field it names; a document this reader refuses is one a consumer cannot read at all"
  fi
elif [ "$reader_required" = 1 ]; then
  fatal "no onevcs that reads an npm scoped identifier could be resolved, and ONETASKGRAPH_RELEASE_READER_REQUIRED=1 asked for one" \
    "install uv, so this can run the pinned $READER_PACKAGE $READER_VERSION, or put an onevcs of 0.16.1 or newer on PATH, then rerun"
else
  reader_status=1
  echo "check-release-targets: no onevcs that reads an npm scoped identifier could be resolved, so the canonical reader did not load this document; install uv for the pinned $READER_PACKAGE $READER_VERSION, or put an onevcs of 0.16.1 or newer on PATH (set ONETASKGRAPH_RELEASE_READER_REQUIRED=1 to make this a failure)" >&2
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

# The short names other repositories wait on, and the artifact each one is — the
# whole set, because it is frozen. This is the second spelling the header
# explains: a consumer names `sdk-pypi` in its own plan and cannot see this file,
# so a short name that moved without moving here is a wait that resolves against
# nothing, and a sixth name declared here is one nothing was told to wait on.
#
# llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This map is a
# restatement with no source to reconcile against, and that is what it is for
# rather than an oversight. The other side of the contract is a plan in a
# different repository — one this repository does not depend on, cannot fetch
# from an offline pinned gate, and must not read even if it could, since `onevcs`
# is this repository's CONSUMER. Deriving the map from release-targets.toml, the
# only in-tree authority, would make the comparison below circular and let an edit
# to the declaration alone pass green, which is the single failure this exists to
# catch. What makes the second spelling safe is instead that the set is frozen and
# refused in both directions here: a name that moved and a name that appeared both
# go red, so the map cannot drift without this file being edited on purpose.
EXPECTED_NAMES = {
    "crate": "crate:onetaskgraph",
    "pypi": "pypi:onetaskgraph-cli",
    "sdk-pypi": "pypi:onetaskgraph-sdk",
    "npm": "npm:@onetaskgraph/cli",
    "sdk-npm": "npm:@onetaskgraph/sdk",
}
# llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

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

retired_entries = declared.get("retired", [])
if not isinstance(retired_entries, list):
    problems.append(f"{DECLARATION}'s retired is not the array of tables the schema declares")
    retired_entries = []
retired_ids = {}
for position, retired in enumerate(retired_entries, start=1):
    where = f"{DECLARATION} [[retired]] #{position}"
    if not isinstance(retired, dict):
        problems.append(f"{where} is not a table")
        continue
    for key in sorted(set(retired) - RETIRED_KEYS):
        problems.append(f"{where} carries the key '{key}', which schema version {SCHEMA_VERSION} does not declare")
    for key in sorted(RETIRED_KEYS - set(retired)):
        problems.append(f"{where} is missing the required key '{key}'")
    # Held to the same types and the same syntax a target's are. An entry exists to
    # tell a consumer still naming something that it is gone, which it can only do
    # if it names it the way that consumer does.
    if "why" in retired:
        why = typed(retired.get("why"), str, where, "why")
        if isinstance(why, str) and (not why.strip() or "\n" in why or len(why) > MAX_PROSE):
            problems.append(f"{where}'s why is not one non-blank line of at most {MAX_PROSE} characters")
    if "id" not in retired:
        continue
    identifier = typed(retired.get("id"), str, where, "id")
    if not isinstance(identifier, str):
        continue
    if not ID_SYNTAX.match(identifier):
        problems.append(f"{where} has the id '{identifier}', which is not <registry>:<name>")
    if identifier in seen_ids:
        problems.append(f"{DECLARATION} retires '{identifier}' and declares it as a target")
    if identifier in retired_ids:
        problems.append(f"{where} retires '{identifier}', which [[retired]] #{retired_ids[identifier]} already retires")
    retired_ids[identifier] = position

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

# A short name that moved, in both directions: one declared here that no
# consumer knows, and one a consumer knows that is no longer declared.
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
        f"{DECLARATION} declares the short name '{name}', and the set is frozen at "
        f"{', '.join(sorted(EXPECTED_NAMES))}. Something else this repository publishes belongs "
        "in the covers list of the target whose release carries it, not under a sixth name no "
        "consumer was told to wait on."
    )

# What this repository really publishes, derived from the release configuration
# rather than transcribed, because a transcription is what goes stale in silence.
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
