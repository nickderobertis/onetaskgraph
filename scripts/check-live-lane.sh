#!/usr/bin/env bash
# The workspace project's own live lane.
#
# It has no credentialed tests of its own — the live journeys belong to the two hosted
# plugin crates. What it does own is the shape of the lane: `just test-live` is only a
# sweep if every project actually declares the target, `.github/workflows/live.yml`
# is only a signal if each job carries exactly one credential under the one name the
# product reads, and the lane is only non-required while its tests carry `#[ignore]` and
# its target passes `--include-ignored`. All three are asserted here so none can be
# quietly removed.
# llmlint: ignore-file[live_tier_compiles_and_requires_credential] empty live lane passes by design
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import json
import sys
from pathlib import Path

# Which job gets which credential — this lane's own shape. The names themselves are the
# contract, which `scripts/check-credential-names.sh` reconciles across every place that
# restates them, this map included.
CREDENTIALS = {"live-linear": "LINEAR_API_KEY", "live-github-projects": "GH_PROJECTS_TOKEN"}

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


problems = []

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
            "make it the map of target names Nx reads, so this check can see whether "
            "`test-live` is among them.",
        )
    if "test-live" not in targets:
        problems.append(f"{project_file}: has no test-live target, so `just test-live` skips it")
        continue

    # The live lane is non-required by decision, and `#[ignore]` is what makes that true
    # rather than stated: `check` runs `cargo test -p <crate>`, which runs every test
    # target the crate has, so an un-ignored live test is part of a required — and cached —
    # check on any machine exporting the credential. Both halves are asserted, because
    # either one alone silently stops the lane running at all.
    live_test = project_file.parent / "tests" / "live.rs"
    if not live_test.exists():
        continue
    source = read(live_test, "that crate's live lane")
    # Attributes only: the doc comments beside them explain the rule, and a check that
    # could not tell those apart would make the explanation impossible to write down.
    attributes = [line.strip() for line in source.splitlines() if line.lstrip().startswith("#[")]
    declared = sum(attribute.startswith(("#[test]", "#[tokio::test")) for attribute in attributes)
    ignored = sum(attribute.startswith("#[ignore") for attribute in attributes)
    if declared and ignored < declared:
        problems.append(
            f"{live_test}: declares {declared} live test(s) but only {ignored} carry "
            "`#[ignore]`, so `cargo test -p <crate>` runs them inside the everyday gate"
        )
    target = targets["test-live"] if isinstance(targets["test-live"], dict) else {}
    options = target.get("options", {})
    command = options.get("command", "") if isinstance(options, dict) else ""
    if declared and "--include-ignored" not in command:
        problems.append(
            f"{project_file}: its test-live command does not pass `--include-ignored`, "
            "so the ignored live tests never run anywhere"
        )

workflow = Path(".github/workflows/live.yml")
if not workflow.exists():
    problems.append(".github/workflows/live.yml: is missing, so nothing runs the live lane")
else:
    text = read(workflow, "the live-lane workflow")
    # Scan the workflow's instructions, not its prose. The comments deliberately name the
    # wrong spelling in order to explain why the right one is right, and a check that
    # cannot tell those apart would make the explanation impossible to write down.
    code = "\n".join(line.split("#", 1)[0] for line in text.splitlines())
    for job, secret in CREDENTIALS.items():
        if f"{job}:" not in code:
            problems.append(f"{workflow}: has no {job} job")
        if f"secrets.{secret}" not in code:
            problems.append(
                f"{workflow}: never passes {secret}. There is one name per credential "
                "everywhere and this workflow translates nothing."
            )
    for job, secret in CREDENTIALS.items():
        other = next(v for k, v in CREDENTIALS.items() if k != job)
        # Each job gets exactly one credential: a job that can see both could pass while
        # exercising the wrong one.
        if f"{secret}: ${{{{ secrets.{other} }}}}" in code:
            problems.append(f"{workflow}: {job} is handed the wrong credential")
    # The long name USED as an env key or a secret reference is the break; the workflow
    # also names it in prose, explaining why the short one is correct, and that comment
    # is the reason a reader does not "fix" it.
    for wrong in ("GITHUB_PROJECTS_TOKEN:", "secrets.GITHUB_PROJECTS_TOKEN"):
        if wrong in code:
            problems.append(
                f"{workflow}: uses GITHUB_PROJECTS_TOKEN. GitHub Actions refuses to create "
                "any secret whose name begins with GITHUB_, which is why the short name "
                "GH_PROJECTS_TOKEN is the one that can exist in both CI and the local "
                "secrets file."
            )

if problems:
    print("check-live-lane: the live lane's shape is broken.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        "check-live-lane: add the missing test-live target to the project.json named above, "
        "restore the credential line in .github/workflows/live.yml under the name the "
        "product reads, or put `#[ignore]` back on the live test and `--include-ignored` "
        "back on the target that runs it; then re-run 'just test-live' to confirm the "
        "sweep reaches every project again.",
        file=sys.stderr,
    )
    sys.exit(1)
PY
