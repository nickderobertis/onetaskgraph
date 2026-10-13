"""Propagate the existing affected-selection decision and collect available telemetry."""
import asyncio
from dataclasses import dataclass
from enum import StrEnum
import os
import re
from typing import NewType
from pathlib import Path
import shlex
import shutil
import sys
import uuid

LIVE_LABEL = "live-session"
# Every text this step decodes, in the encoding its producer is made to write on every
# platform. Left to the default, Windows pipes carry the ANSI code page, where the selection
# helper's em dash is byte 0x97 and refuses to decode as UTF-8.
SELECTION_ENCODING = "utf-8"  # live-lane-selection.sh reconfigures both its streams to this
OBJECT_NAME_ENCODING = "ascii"  # `git rev-parse` prints a hexadecimal object name
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


def executable(name: str) -> str:
    if os.name == "nt" and name == "bash":
        # Search PATH ourselves: CreateProcess otherwise prefers System32's WSL
        # launcher. Git Bash is identified by its accompanying MSYS runtime.
        candidates = [Path(directory) / "bash.exe" for directory in os.get_exec_path()]
        git = shutil.which("git.exe")
        if git:
            git_root = Path(git).resolve().parent.parent
            candidates.extend([git_root / "bin/bash.exe", git_root / "usr/bin/bash.exe"])
        for candidate in candidates:
            runtimes = [candidate.parent / "msys-2.0.dll",
                        candidate.parent.parent / "usr/bin/msys-2.0.dll"]
            if candidate.is_file() and any(runtime.is_file() for runtime in runtimes):
                return str(candidate.resolve())
        raise ValueError("Git for Windows Bash executable with MSYS runtime not found on PATH")
    resolved = shutil.which(name)
    if resolved is None:
        raise ValueError(f"executable not found: {name}")
    return str(Path(resolved).resolve())


def decoded(raw: bytes | None, encoding: str | None) -> str:
    """Strictly decoded: a byte outside the producer's encoding refuses the step."""
    return raw.decode(encoding) if raw is not None and encoding is not None else ""


async def execute(command: list[str], root: Path, *, stdout_encoding: str | None = None,
                  stderr_encoding: str | None = None,
                  environment: dict[str, str] | None = None) -> ProcessResult:
    """Run a command; a stream is captured only when its producer's encoding is named."""
    process = await asyncio.create_subprocess_exec(
        executable(command[0]), *command[1:], cwd=root, env=environment,
        stdout=asyncio.subprocess.PIPE if stdout_encoding else None,
        stderr=asyncio.subprocess.PIPE if stderr_encoding else None,
    )
    stdout, stderr = await process.communicate()
    if process.returncode is None:
        raise RuntimeError("completed process has no exit status")
    return ProcessResult(process.returncode, decoded(stdout, stdout_encoding),
                         decoded(stderr, stderr_encoding))


async def select(policy: str, command: list[str], root: Path) -> int:
    result = await execute(["bash", policy, "--nx-exclusions"], root,
                           stdout_encoding=SELECTION_ENCODING,
                           stderr_encoding=SELECTION_ENCODING)
    # Re-emitted in the encoding it was written in, whatever this platform's stderr uses.
    sys.stderr.flush()
    sys.stderr.buffer.write(result.stderr.encode(SELECTION_ENCODING))
    sys.stderr.buffer.flush()
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
            # git's own diagnostics reach stderr directly; only the object name is decoded.
            revision = await execute(["git", "rev-parse", "HEAD"], root,
                                     stdout_encoding=OBJECT_NAME_ENCODING)
            name = revision.stdout.strip()
            if revision.status or not re.fullmatch(r"[0-9a-f]{40,64}", name):
                raise ValueError(f"source revision unavailable: git rev-parse HEAD exited "
                                 f"{revision.status} printing {name!r}")
            environment["GITHUB_LIVE_SOURCE_REVISION"] = name
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
