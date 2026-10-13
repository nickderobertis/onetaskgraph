"""Propagate the existing affected-selection decision and collect available telemetry."""
import asyncio
from dataclasses import dataclass
from enum import StrEnum
import os
import re
from typing import NewType
from pathlib import Path
import shlex
import sys
import uuid

LIVE_LABEL = "live-session"
SessionId = NewType("SessionId", str)


class Selection(StrEnum):
    RUN = "run"
    NOT_SELECTED = "not-selected"


def session_id(value: str) -> SessionId:
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,256}", value):
        raise ValueError("session identity must be a safe path component")
    return SessionId(value)


@dataclass(frozen=True)
class ProcessResult:
    status: int
    stdout: str
    stderr: str


async def execute(command: list[str], root: Path, *, capture: bool = False,
                  environment: dict[str, str] | None = None) -> ProcessResult:
    process = await asyncio.create_subprocess_exec(
        *command, cwd=root, env=environment,
        stdout=asyncio.subprocess.PIPE if capture else None,
        stderr=asyncio.subprocess.PIPE if capture else None,
    )
    stdout, stderr = await process.communicate()
    if process.returncode is None:
        raise RuntimeError("completed process has no exit status")
    return ProcessResult(process.returncode, (stdout or b"").decode(), (stderr or b"").decode())


async def select(policy: str, command: list[str], root: Path) -> int:
    result = await execute(["bash", policy, "--nx-exclusions"], root, capture=True)
    sys.stderr.write(result.stderr)
    exclusions = shlex.split(result.stdout) if result.status == 0 else []
    if len(exclusions) > 1 or any(not value.startswith("--exclude=") for value in exclusions):
        raise ValueError("selection helper returned malformed Nx exclusions")
    excluded = exclusions[0].removeprefix("--exclude=").split(",") if exclusions else []
    environment = os.environ.copy()
    selection = Selection.NOT_SELECTED if "onetaskgraph-github-projects" in excluded else Selection.RUN
    environment["GITHUB_LIVE_SELECTION"] = selection.value
    if selection is Selection.RUN and environment.get("GH_PROJECTS_TOKEN"):
        identity = session_id(environment.get("GITHUB_LIVE_SESSION_ID") or f"local-{uuid.uuid4().hex}")
        directory = Path(environment.get("GITHUB_LIVE_SESSION_DIR") or
                         root / "target/debug/telemetry/github-live-session" / identity)
        await asyncio.to_thread(directory.mkdir, parents=True, exist_ok=False)
        environment["GITHUB_LIVE_SESSION_ID"] = identity
        environment["GITHUB_LIVE_SESSION_DIR"] = str(await asyncio.to_thread(directory.resolve))
        if not environment.get("GITHUB_LIVE_SOURCE_REVISION"):
            revision = await execute(["git", "rev-parse", "HEAD"], root, capture=True)
            if revision.status:
                raise ValueError(f"source revision unavailable: {revision.stderr.strip()}")
            environment["GITHUB_LIVE_SOURCE_REVISION"] = revision.stdout.strip()
    return (await execute([*command, *exclusions], root, environment=environment)).status


async def budgets(root: Path) -> int:
    try:
        selected = Selection(os.environ.get("GITHUB_LIVE_SELECTION", Selection.RUN.value))
    except ValueError as error:
        raise ValueError("invalid propagated GitHub live selection") from error
    arguments = ["bun", "run", "onebudgetspec", "check",
                 "crates/onetaskgraph-github-projects-e2e/budgets.yaml"]
    if selected is Selection.NOT_SELECTED or not os.environ.get("GH_PROJECTS_TOKEN"):
        reason = "existing selection policy did not select the lane" if selected is Selection.NOT_SELECTED else "no GitHub credential was supplied"
        print(f"github-live-session-points unavailable: {reason}", flush=True)
        arguments.extend(["--exclude-label", LIVE_LABEL])
    return (await execute(arguments, root)).status


async def main(arguments: list[str]) -> int:
    root = (await asyncio.to_thread(Path(__file__).resolve)).parents[2]
    match arguments:
        case ["select", policy, command, *remaining]:
            return await select(policy, [command, *remaining], root)
        case ["budgets"]:
            return await budgets(root)
        case _:
            raise ValueError("expected select POLICY NX_COMMAND... or budgets")


if __name__ == "__main__":
    try:
        status = asyncio.run(main(sys.argv[1:]))
    except (OSError, ValueError) as error:
        print(f"GitHub live collection refused: {error}", file=sys.stderr)
        status = 1
    sys.exit(status)
