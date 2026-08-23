#!/usr/bin/env bash
# The workspace project's own live lane.
#
# It has no credentialed tests of its own — the live journeys belong to the two hosted
# plugin crates. What it does own is the shape of the lane: `just test-live` is only a
# sweep if every project actually declares the target, and `.github/workflows/live.yml`
# is only a signal if each job carries exactly one credential under the one name the
# product reads. Both are asserted here so neither can be quietly removed.
# llmlint: ignore-file[live_tier_compiles_and_requires_credential] empty live lane passes by design
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import json
import sys
from pathlib import Path

# One name per credential, everywhere: the repository secret, the local secrets file, the
# configuration document's *_env default, and the variable the product reads. Nothing
# translates on the way in, so these are the names the workflow must hand through.
CREDENTIALS = {"live-linear": "LINEAR_API_KEY", "live-github-projects": "GH_PROJECTS_TOKEN"}

problems = []

for project_file in sorted(
    list(Path("crates").glob("*/project.json"))
    + list(Path("sdks").glob("*/project.json"))
    + [Path("workspace/project.json")]
):
    project = json.loads(project_file.read_text())
    if "test-live" not in project.get("targets", {}):
        problems.append(f"{project_file}: has no test-live target, so `just test-live` skips it")

workflow = Path(".github/workflows/live.yml")
if not workflow.exists():
    problems.append(".github/workflows/live.yml: is missing, so nothing runs the live lane")
else:
    text = workflow.read_text()
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
        "or restore the credential line in .github/workflows/live.yml under the name the "
        "product reads; then re-run 'just test-live' to confirm the sweep reaches every "
        "project again.",
        file=sys.stderr,
    )
    sys.exit(1)
PY
