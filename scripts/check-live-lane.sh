#!/usr/bin/env bash
# The workspace project's own live lane.
#
# It has no credentialed tests of its own — the live journeys belong to the two hosted
# plugin crates. What it does own is the shape of the lane: `just test-live` is only a
# sweep if every project actually declares the target, and `.github/workflows/live.yml`
# is only a signal if each job carries exactly one credential under the one name the
# product reads. Both are asserted here so neither can be quietly removed.
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
    for job, secret in CREDENTIALS.items():
        if f"{job}:" not in text:
            problems.append(f"{workflow}: has no {job} job")
        if f"secrets.{secret}" not in text:
            problems.append(
                f"{workflow}: never passes {secret}. There is one name per credential "
                "everywhere and this workflow translates nothing."
            )
    for job, secret in CREDENTIALS.items():
        other = next(v for k, v in CREDENTIALS.items() if k != job)
        # Each job gets exactly one credential: a job that can see both could pass while
        # exercising the wrong one.
        if f"{secret}: ${{{{ secrets.{other} }}}}" in text:
            problems.append(f"{workflow}: {job} is handed the wrong credential")
    if "GITHUB_PROJECTS_TOKEN" in text:
        problems.append(
            f"{workflow}: names GITHUB_PROJECTS_TOKEN. GitHub Actions refuses any secret "
            "whose name begins with GITHUB_, which is why the short name GH_PROJECTS_TOKEN "
            "is the one that exists in both CI and the local secrets file."
        )

if problems:
    print("check-live-lane: the live lane's shape is broken.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    sys.exit(1)
PY
