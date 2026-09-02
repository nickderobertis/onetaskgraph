#!/usr/bin/env bash
# The shape of the tests that reach a real API, now that nothing about them is special.
#
# Every part of it can be undone by an edit nothing else would complain about, so each is
# asserted here:
#
#   1. No `test-live` target, target default, recipe or workflow — the separate lane stays
#      gone, since re-adding one is how these tests quietly stop being required.
#   2. Every live test really runs where it now lives: tests are declared, and none carries
#      `#[ignore]`.
#   3. Each hosted plugin's journey opens its session through the one gate,
#      `onetaskgraph_live::Session::open`, so a precondition added there governs every path
#      to a real API rather than some.
#   4. .github/workflows/ci.yml hands each credential to exactly one lane of the matrix,
#      under the name the product reads, with that lane's nominations and the demand that
#      stops a missing credential passing green.
#   5. scripts/rust-coverage.sh clears them, because coverage re-runs the same tests.
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

# 1. The separate lane is gone, in every place it was declared.
for project_file in sorted(
    list(Path("crates").glob("*/project.json"))
    + list(Path("sdks").glob("*/project.json"))
    + [Path("workspace/project.json")]
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
        tagged[str(project_file.parent / "tests" / "live.rs").replace("\\", "/")] = project_file

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
    attributes = [line.strip() for line in source.splitlines() if line.lstrip().startswith("#[")]
    declared = sum(attribute.startswith(("#[test]", "#[tokio::test")) for attribute in attributes)
    ignored = sum(attribute.startswith("#[ignore") for attribute in attributes)
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
    if "Session::open(" not in source:
        problems.append(
            f"{relative}: never opens a session through onetaskgraph_live::Session::open. "
            "Every path by which these tests reach a real API goes through that one place, "
            "so a precondition added there governs all of them rather than some"
        )

# 4. The workflow hands each credential to exactly one lane, with what that lane needs.
workflow = read(WORKFLOW, "the workflow that runs the required check")
code = without_comments(workflow)
# Steps, so a credential can be attributed to the lane that carries it. A step begins at
# the one indentation `- name:` uses inside a job's `steps:` list.
steps = re.split(r"(?m)^      - (?=name:|uses:)", code)
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
        if DEMAND not in step:
            problems.append(
                f"{WORKFLOW}: hands {credential} to a step that does not set {DEMAND}, so "
                "that lane passes green when the credential is absent"
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

if problems:
    print("check-live-lane: these tests are no longer ordinary.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        "check-live-lane: fix the file named above. The arrangement is: no separate target, "
        "recipe or workflow; no `#[ignore]` on a live test; one gate through "
        "onetaskgraph_live::Session::open; the credentials and nominations on one lane of "
        "the matrix in .github/workflows/ci.yml with ONETASKGRAPH_LIVE_REQUIRED beside "
        "them; and scripts/rust-coverage.sh clearing them so coverage opens no second "
        "session. AGENTS.md records why each of those is what it is.",
        file=sys.stderr,
    )
    sys.exit(1)
PY
