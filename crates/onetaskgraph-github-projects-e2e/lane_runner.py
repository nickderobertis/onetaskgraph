"""Propagate the existing affected-selection decision and collect available telemetry."""
import os
from pathlib import Path
import shlex
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
LIVE_LABEL = "live-session"


def select(policy: str, command: list[str]) -> int:
    result = subprocess.run(
        ["bash", policy, "--nx-exclusions"], cwd=ROOT, capture_output=True, text=True
    )
    sys.stderr.write(result.stderr)
    exclusions = shlex.split(result.stdout) if result.returncode == 0 else []
    if len(exclusions) > 1 or any(not value.startswith("--exclude=") for value in exclusions):
        raise ValueError("selection helper returned malformed Nx exclusions")
    excluded = exclusions[0].removeprefix("--exclude=").split(",") if exclusions else []
    environment = os.environ.copy()
    environment["GITHUB_LIVE_SELECTION"] = (
        "not-selected" if "onetaskgraph-github-projects" in excluded else "run"
    )
    if environment["GITHUB_LIVE_SELECTION"] == "run" and environment.get("GH_PROJECTS_TOKEN"):
        identity = environment.get("GITHUB_LIVE_SESSION_ID") or f"local-{uuid.uuid4().hex}"
        directory = Path(environment.get("GITHUB_LIVE_SESSION_DIR") or
                         ROOT / "target/debug/telemetry/github-live-session" / identity)
        directory.mkdir(parents=True, exist_ok=False)
        environment["GITHUB_LIVE_SESSION_ID"] = identity
        environment["GITHUB_LIVE_SESSION_DIR"] = str(directory.resolve())
        if not environment.get("GITHUB_LIVE_SOURCE_REVISION"):
            environment["GITHUB_LIVE_SOURCE_REVISION"] = subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
            ).strip()
    return subprocess.run([*command, *exclusions], cwd=ROOT, env=environment).returncode


def budgets() -> int:
    selected = os.environ.get("GITHUB_LIVE_SELECTION", "run")
    if selected not in ("run", "not-selected"):
        raise ValueError("invalid propagated GitHub live selection")
    arguments = ["bun", "run", "onebudgetspec", "check",
                 "crates/onetaskgraph-github-projects-e2e/budgets.yaml"]
    if selected == "not-selected" or not os.environ.get("GH_PROJECTS_TOKEN"):
        reason = "existing selection policy did not select the lane" if selected == "not-selected" else "no GitHub credential was supplied"
        print(f"github-live-session-points unavailable: {reason}", flush=True)
        arguments.extend(["--exclude-label", LIVE_LABEL])
    return subprocess.run(arguments, cwd=ROOT).returncode


if __name__ == "__main__":
    try:
        if len(sys.argv) >= 4 and sys.argv[1] == "select":
            status = select(sys.argv[2], sys.argv[3:])
        elif sys.argv[1:] == ["budgets"]:
            status = budgets()
        else:
            raise ValueError("expected select POLICY NX_COMMAND... or budgets")
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"GitHub live collection refused: {error}", file=sys.stderr)
        status = 1
    sys.exit(status)
