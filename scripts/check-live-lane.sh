#!/usr/bin/env bash
# The shape of the tests that reach a real API, now that nothing about them is special.
#
# Every part of it can be undone by an edit nothing else would complain about, so each is
# asserted here:
#
#   1. No `test-live` target, target default, recipe or workflow — the separate lane stays
#      gone, since re-adding one is how these tests quietly stop being required.
#   2. Each hosted plugin's cacheable `test` target is keyed on its credential, its
#      nominations and the demand, so a replay of a run that had no credential cannot stand
#      in for a run that has one.
#   3. Every live test really runs where it now lives: tests are declared, none carries
#      `#[ignore]`, and none is compiled out — a journey excluded by `cfg` runs exactly as
#      often as an ignored one and says even less about it.
#   4. Each hosted plugin's journey opens its session through the one gate,
#      `onetaskgraph_live::Session::open`, so a precondition added there governs every path
#      to a real API rather than some.
#   5. .github/workflows/ci.yml hands each credential to exactly one lane of the matrix,
#      under the name the product reads, with that lane's nominations and the demand that
#      stops a missing credential passing green — set to `1`, rather than merely mentioned,
#      because `ONETASKGRAPH_LIVE_REQUIRED: 0` is spelled the same way and is the hole.
#   6. scripts/rust-coverage.sh clears them, because coverage re-runs the same tests.
#   7. Every variable the live crate declares sits inside the namespace the product's
#      configuration layer reserves — the lane's variables share the product's own
#      `ONETASKGRAPH_` prefix, and one outside that namespace decodes to an unknown
#      setting and refuses every invocation of the binary on the lane that exports it.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import json
import re
import sys
from pathlib import Path

# Which credential each hosted plugin's session needs, and the nominations that bound where
# it may write. The names themselves are the contract, which
# `scripts/check-credential-names.sh` reconciles across every place that restates them,
# this map included. Which crates belong in it is not this map's own claim: a crate says so
# with a `live:` tag on its project.json, and the two are reconciled below both ways, so a
# plugin that gains a live session without a row here cannot merge unwatched.
SESSIONS = {
    "crates/onetaskgraph-linear/tests/live.rs": {
        "credential": "LINEAR_API_KEY",
        "nominations": ("LINEAR_WRITE_TEAM",),
    },
    "crates/onetaskgraph-github-projects/tests/live.rs": {
        "credential": "GH_PROJECTS_TOKEN",
        "nominations": ("GH_PROJECTS_OWNER", "GH_PROJECTS_NUMBER", "GH_PROJECTS_REPOSITORY"),
    },
}

WORKFLOW = Path(".github/workflows/ci.yml")
COVERAGE = Path("scripts/rust-coverage.sh")
DEMAND = "ONETASKGRAPH_LIVE_REQUIRED"
# The one run where the demand is legitimately not made, spelled as GitHub Actions spells
# it. A fork pull request receives no secrets at all, so a credential was never expected
# there; every other run is one where a missing credential is a misconfiguration. It is the
# whole condition rather than a word out of it — a guard merely *containing* "fork" would
# let any expression at all turn the demand off.
FORK = "github.event.pull_request.head.repo.fork"

# The two sides of assertion 7. The live crate declares its variables as `pub const`s; the
# engine's environment layer reserves one namespace under the configuration prefix, and a
# live variable outside it is read as a setting nobody declared.
LIVE_CRATE = Path("crates/onetaskgraph-live/src/lib.rs")
ENVIRONMENT_LAYER = Path("crates/onetaskgraph-core/src/config/environment_layer.rs")


def refuse(problem, next_action):
    """Stop with the exact problem and one concrete thing to do about it."""
    print(f"check-live-lane: {problem}", file=sys.stderr)
    print(f"check-live-lane: {next_action}", file=sys.stderr)
    sys.exit(1)


def read(path, what):
    """One input file, reported by name rather than as a traceback if it will not open."""
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as problem:
        refuse(
            f"could not read {path}: {problem}.",
            f"restore {what} as readable UTF-8 text, or point this check at where it "
            "moved to.",
        )


def without_comments(text):
    """A file's instructions without its explanations.

    Several of these files name a wrong spelling, or the removed target, deliberately in
    order to explain why the right one is right — and a check that could not tell an
    instruction from an explanation would make the explanation impossible to write down.
    """
    return "\n".join(line.split("#", 1)[0] for line in text.splitlines())


problems = []
tagged = {}
declared = {}

# 1. The separate lane is gone, in every place it was declared.
for project_file in sorted(
    list(Path("crates").glob("*/project.json"))
    + list(Path("sdks").glob("*/project.json"))
    + [Path("workspace/project.json"), Path("scripts/project.json")]
):
    text = read(project_file, "that project's configuration")
    try:
        project = json.loads(text)
    except json.JSONDecodeError as problem:
        refuse(
            f"{project_file} is not valid JSON: {problem}.",
            "fix that file — every other check that reads the project graph reads it too.",
        )
    if not isinstance(project, dict):
        refuse(
            f"{project_file} is valid JSON but not an object.",
            "make it the project configuration Nx reads — every other check that reads "
            "the project graph reads it too.",
        )
    targets = project.get("targets", {})
    if not isinstance(targets, dict):
        refuse(
            f"{project_file} has a `targets` that is not an object.",
            "make it the map of target names Nx reads, so this check can see which "
            "targets it declares.",
        )
    if "test-live" in targets:
        problems.append(
            f"{project_file}: declares a `test-live` target again. The tests that reach a "
            "real API are ordinary tests of `test`; a target beside it is a lane the "
            "required check does not run"
        )
    tags = project.get("tags", [])
    if any(isinstance(tag, str) and tag.startswith("live:") for tag in tags):
        relative = str(project_file.parent / "tests" / "live.rs").replace("\\", "/")
        tagged[relative] = project_file
        declared[relative] = targets.get("test", {})

for relative, project_file in sorted(tagged.items()):
    if relative not in SESSIONS:
        problems.append(
            f"{project_file}: is tagged as having a live session, but this check knows of "
            f"no {relative} — add its credential and its nominations to the SESSIONS map in "
            "this script, or the session it runs is one nothing here watches"
        )
for relative in sorted(set(SESSIONS) - set(tagged)):
    problems.append(
        f"{relative}: is watched here, but its project.json carries no `live:` tag — the tag "
        "is how a crate says it has a session at all, and without it the two lists agree by "
        "accident"
    )

# 2. A cached result cannot stand in for a run whose credential differs.
#
# `test` is a cacheable target keyed on files, and the live journey among those tests runs,
# skips or declines according to the environment rather than the tree. Without the
# environment in the key, a replay of a run that had no credential satisfies a run that has
# one: the live tests do not run and the target reports green, which is the arrangement this
# whole shape exists to remove, rebuilt inside the cache.
for relative, session in sorted(SESSIONS.items()):
    test = declared.get(relative)
    if test is None:
        continue
    project_file = tagged[relative]
    if not isinstance(test, dict):
        problems.append(
            f"{project_file}: has a `test` target that is not an object, so this check "
            "cannot read what its cache is keyed on"
        )
        continue
    keyed = {
        entry["env"]
        for entry in test.get("inputs", [])
        if isinstance(entry, dict) and isinstance(entry.get("env"), str)
    }
    for variable in (session["credential"], *session["nominations"], DEMAND):
        if variable not in keyed:
            problems.append(
                f"{project_file}: its `test` target does not name {variable} as an `env` "
                "input, so a cached run that read a different value for it would be "
                f"replayed instead of running {relative}"
            )

if "test-live" in json.loads(read(Path("nx.json"), "the Nx configuration")).get(
    "targetDefaults", {}
):
    problems.append(
        "nx.json: still defaults a `test-live` target, which no project declares"
    )
if re.search(r"(?m)^test-live", read(Path("justfile"), "the command surface")):
    problems.append(
        "justfile: declares a `test-live` recipe again, which is a second way to run these "
        "tests outside the required check"
    )
if Path(".github/workflows/live.yml").exists():
    problems.append(
        ".github/workflows/live.yml: exists again. These tests run in the ordinary `check` "
        "job; a workflow of their own is one auto-merge does not wait on"
    )

# 2 and 3. Each journey really runs where it now lives, and opens the one gate.
for relative, session in SESSIONS.items():
    path = Path(relative)
    if not path.exists():
        problems.append(f"{relative}: is missing, so this plugin has no live journey at all")
        continue
    source = read(path, "that crate's live journey")
    # Attributes only: the doc comments beside them explain the rule, and a check that
    # could not tell those apart would make the explanation impossible to write down.
    attributes = [
        line.strip()
        for line in source.splitlines()
        if line.lstrip().startswith(("#[", "#!["))
    ]
    declared = sum(attribute.startswith(("#[test]", "#[tokio::test")) for attribute in attributes)
    ignored = sum(attribute.startswith("#[ignore") for attribute in attributes)
    # A journey compiled out runs exactly as often as one marked `#[ignore]`, and it is the
    # quieter of the two: `cargo test` prints an ignored test as ignored and prints nothing
    # at all about a test that was never built. `#![cfg(...)]` takes the whole file,
    # `#[cfg(...)]` takes the item it sits on, and `#[cfg_attr(..., ignore)]` puts the
    # attribute above back under a condition — so all four spellings are refused rather than
    # the one that used to be reached for. Neither journey carries any of them today, so
    # this costs nothing until somebody adds one.
    excluded = [
        attribute
        for attribute in attributes
        if attribute.startswith(("#[cfg(", "#![cfg(", "#[cfg_attr(", "#![cfg_attr("))
    ]
    if not declared:
        problems.append(
            f"{relative}: declares no test, so nothing here reaches the real API at all"
        )
    if ignored:
        problems.append(
            f"{relative}: carries {ignored} `#[ignore]` attribute(s). That is what used to "
            "keep this journey out of every target but the separate lane, and the separate "
            "lane is gone — an ignored test here runs nowhere"
        )
    if excluded:
        problems.append(
            f"{relative}: carries a conditional-compilation attribute, {excluded[0]}. A "
            "journey the build leaves out runs nowhere, exactly as an `#[ignore]` here "
            "would, and reports even less about it — `cargo test` names an ignored test "
            "and says nothing at all about one that was never compiled. These tests are "
            "ordinary tests of the crate they live in: every platform builds them and the "
            "credential decides at run time whether they reach the API"
        )
    if "Session::open(" not in source:
        problems.append(
            f"{relative}: never opens a session through onetaskgraph_live::Session::open. "
            "Every path by which these tests reach a real API goes through that one place, "
            "so a precondition added there governs all of them rather than some"
        )

# What the demand has to be SET to, read out of the live crate rather than restated here.
# `onetaskgraph_live::required` is what actually decides, and it reads `0`, the empty string
# and an absent variable all as leaving the lane free to skip — so the name being present in
# a step says nothing on its own. Taking the value from that crate's own `DEMANDED` is what
# stops this check and the parser it stands for drifting into two spellings.
live_source = read(LIVE_CRATE, "the live crate that declares the lane's variables")
def declared_value(name):
    """One of the live crate's two `&str` constants, or a refusal naming it."""
    found = re.search(rf'(?m)^pub const {name}: &str = "([^"]+)";$', live_source)
    if not found:
        refuse(
            f"{LIVE_CRATE} no longer declares {name} as a string constant, so what "
            f"{DEMAND} may be set to cannot be read from the crate that decides it.",
            f"restore `pub const {name}: &str = \"...\";` there, or point this check at "
            "where the value moved to.",
        )
    return found.group(1)


DEMANDED = declared_value("DEMANDED")
NOT_DEMANDED = declared_value("NOT_DEMANDED")

# 4. The workflow hands each credential to exactly one lane, with what that lane needs.
workflow = read(WORKFLOW, "the workflow that runs the required check")
code = without_comments(workflow)
# Steps, so a credential can be attributed to the lane that carries it. A step begins at
# the one indentation `- name:` uses inside a job's `steps:` list.
steps = re.split(r"(?m)^      - (?=name:|uses:)", code)


def demanded(step):
    """What a step assigns [`DEMAND`], or `None` when it assigns nothing.

    The assignment rather than the name: the workflow also writes the name in prose above
    the line that sets it, and a step that merely mentions it demands nothing.
    """
    assignment = re.search(rf"(?m)^\s*{DEMAND}:[ \t]*(.+?)\s*$", step)
    return assignment.group(1) if assignment else None


def demands_a_credential(value):
    """Whether that value is the demand on every run but a fork pull request.

    Exactly two spellings satisfy it. The demanded value on its own is one. The other is
    the fork exception, and it is matched whole rather than by any word in it: GitHub
    Actions has no ternary, so the workflow spells that case `<FORK> && '<off>' || '<on>'`,
    where the `||` fallback is what every run that is not a fork pull request takes. Both
    values are the live crate's own — a fork branch of anything else, `2` included, is a
    value `onetaskgraph_live::required` refuses outright, which fails the lane for a reason
    that has nothing to do with what it was testing. The condition has to be [`FORK`]
    itself and the fallback has to be the demand, because a
    guard this recognised loosely would be one any other condition could be substituted
    into — turning the demand off for scheduled runs, or for a branch, while still reading
    as the fork exception. That is the same green-for-a-missing-credential hole one level
    further in.
    """
    if value.strip().strip("\"'") == DEMANDED:
        return True
    expression = re.fullmatch(r"\$\{\{(.+)\}\}", value.strip())
    if not expression:
        return False
    guarded, _, fallback = expression.group(1).rpartition("||")
    condition, _, off = guarded.partition("&&")
    # llmlint: ignore[live_tier_compiles_and_requires_credential] The fork branch this accepts is the ONE run where no credential was ever expected: GitHub supplies a fork pull request no secrets at all, so demanding one there would fail every outside contribution for something its author cannot supply. That decision is the repository's, recorded in AGENTS.md and at .github/workflows/ci.yml, which is the only place credentials enter this build; what this function does is hold the exception to that one condition and to the crate's own two values, so no OTHER run can be spelled into it. Every run the merge waits on takes the `|| DEMANDED` fallback, where an absent credential fails rather than skipping green — which is the demand this rule asks for.
    return (
        condition.strip() == FORK
        and off.strip().strip("\"'") == NOT_DEMANDED
        and fallback.strip().strip("\"'") == DEMANDED
    )

for relative, session in SESSIONS.items():
    credential = session["credential"]
    carrying = [step for step in steps if f"secrets.{credential}" in step]
    if not carrying:
        problems.append(
            f"{WORKFLOW}: never passes {credential}, so {relative} skips everywhere and "
            "proves nothing. There is one name per credential and nothing translates "
            "between spellings"
        )
    for step in carrying:
        if "matrix.os == 'ubuntu-latest'" not in step:
            problems.append(
                f"{WORKFLOW}: hands {credential} to a step that is not scoped to one lane "
                "of the platform matrix. Every lane carrying it opens a session of its own "
                "against the same shared fixture"
            )
        assigned = demanded(step)
        if assigned is None:
            problems.append(
                f"{WORKFLOW}: hands {credential} to a step that does not set {DEMAND}, so "
                "that lane passes green when the credential is absent"
            )
        elif not demands_a_credential(assigned):
            problems.append(
                f"{WORKFLOW}: hands {credential} to a step that sets {DEMAND} to "
                f"{assigned!r}, which is not the demand. `0`, the empty string and an "
                "absent variable all read the same way — the lane skips, and a skip is a "
                "conclusion branch protection accepts in place of success — so the name "
                f"being set is not enough. It has to be {DEMANDED} on every run but a fork "
                "pull request, which is the one run GitHub supplies no secrets to"
            )
        for nomination in session["nominations"]:
            if nomination not in step:
                problems.append(
                    f"{WORKFLOW}: hands {credential} to a step that does not name "
                    f"{nomination}, so that session skips — or, with {DEMAND}=1, fails — "
                    "for want of a nomination the lane refuses to discover for itself"
                )
# The long name USED as an env key or a secret reference is the break; the workflow also
# names it in prose, explaining why the short one is correct, and that comment is the reason
# a reader does not "fix" it.
for wrong in ("GITHUB_PROJECTS_TOKEN:", "secrets.GITHUB_PROJECTS_TOKEN"):
    if wrong in code:
        problems.append(
            f"{WORKFLOW}: uses GITHUB_PROJECTS_TOKEN. GitHub Actions refuses to create any "
            "secret whose name begins with GITHUB_, which is why the short name "
            "GH_PROJECTS_TOKEN is the one that can exist in both CI and the local secrets "
            "file."
        )

# 5. Coverage does not open the second session.
coverage = read(COVERAGE, "the coverage script every Rust crate routes through")
cleared = re.search(r"(?m)^unset ([^\n]+)$", without_comments(coverage))
cleared_names = set(cleared.group(1).split()) if cleared else set()
for name in {session["credential"] for session in SESSIONS.values()} | {DEMAND}:
    if name not in cleared_names:
        problems.append(
            f"{COVERAGE}: does not clear {name}. `just check` runs `test` and then "
            "`coverage`, and `cargo llvm-cov` re-runs the same integration tests — so this "
            "phase would open a second session against the shared external fixture the "
            "first one is still writing to"
        )

# 7. No live variable collides with the product's configuration prefix.
#
# `ONETASKGRAPH_LIVE_REQUIRED` is exported on the very step that generates the Python SDK
# and runs the journeys, and both of those drive the binary — so a live variable the
# configuration layer reads as a setting refuses every one of those invocations for an
# unknown field. That is not a hypothetical: it failed the required check once, in the
# generator, which reported it as a generator error.
layer_source = read(ENVIRONMENT_LAYER, "the engine's environment configuration layer")

prefix = re.search(
    r'(?m)^pub const ENVIRONMENT_PREFIX: &str = "([^"]+)";$', layer_source
)
namespace = re.search(
    r'(?m)^const RESERVED_NAMESPACE: &str = "([^"]+)";$', layer_source
)
if not prefix or not namespace:
    refuse(
        f"{ENVIRONMENT_LAYER} no longer declares both ENVIRONMENT_PREFIX and "
        "RESERVED_NAMESPACE as string constants, so what the configuration layer reads "
        "and what it leaves alone cannot be told apart here.",
        "restore both constants, or point this check at where the reservation moved to.",
    )

declared = dict(
    re.findall(r'(?m)^pub const (\w+): &str = "([^"]+)";$', live_source)
)
live_variables = {
    name: value
    for name, value in declared.items()
    if value.startswith(prefix.group(1))
}
if not live_variables:
    refuse(
        f"{LIVE_CRATE} declares no variable under {prefix.group(1)}, so this check is "
        "watching nothing.",
        "restore the lane's variables as `pub const NAME: &str = \"...\";`, or point "
        "this check at where they moved to.",
    )
for name, value in sorted(live_variables.items()):
    if not value.startswith(namespace.group(1)):
        problems.append(
            f"{LIVE_CRATE}: {name} is {value!r}, which is under the configuration prefix "
            f"{prefix.group(1)!r} but outside the namespace "
            f"{ENVIRONMENT_LAYER} reserves, {namespace.group(1)!r}. The configuration "
            "layer would read it as a setting called "
            f"{value[len(prefix.group(1)):].lower()!r} and refuse every invocation of the "
            "binary on the lane that exports it — the SDK generator and every journey "
            "included"
        )

if problems:
    print("check-live-lane: these tests are no longer ordinary.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        "check-live-lane: fix the file named above. The arrangement is: no separate target, "
        "recipe or workflow; no `#[ignore]` on a live test and none compiled out; one gate "
        "through "
        "onetaskgraph_live::Session::open; the credentials and nominations on one lane of "
        "the matrix in .github/workflows/ci.yml with ONETASKGRAPH_LIVE_REQUIRED set to 1 "
        "beside them; scripts/rust-coverage.sh clearing them so coverage opens no second "
        "session; and every live variable inside the namespace the engine's environment "
        "layer reserves. AGENTS.md records why each of those is what it is.",
        file=sys.stderr,
    )
    sys.exit(1)
PY
