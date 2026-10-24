"""Async subprocesses with typed results: the CLI under test, onevcs, git and cargo."""

from __future__ import annotations

import asyncio
import time
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Completed:
    """One finished process."""

    argv: tuple[str, ...]
    returncode: int
    stdout: bytes
    stderr: bytes
    seconds: float

    @property
    def out(self) -> str:
        """Standard output as text."""
        return self.stdout.decode(errors="replace")

    @property
    def err(self) -> str:
        """Standard error as text."""
        return self.stderr.decode(errors="replace")


class ProcessFailed(RuntimeError):
    """A process that had to succeed did not."""


async def run(
    argv: list[str | Path],
    *,
    stdin: bytes | None = None,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    timeout: float = 600.0,
) -> Completed:
    """Run `argv` to completion, its output captured."""
    words = tuple(str(a) for a in argv)
    start = time.time()
    process = await asyncio.create_subprocess_exec(
        *words,
        stdin=asyncio.subprocess.PIPE if stdin is not None else asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        cwd=cwd,
        env=env,
    )
    try:
        stdout, stderr = await asyncio.wait_for(process.communicate(stdin), timeout)
    except TimeoutError:
        process.kill()
        await process.wait()
        raise
    return Completed(
        argv=words,
        returncode=process.returncode if process.returncode is not None else -1,
        stdout=stdout,
        stderr=stderr,
        seconds=time.time() - start,
    )


async def check(
    argv: list[str | Path],
    *,
    stdin: bytes | None = None,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
) -> Completed:
    """Run `argv` and refuse a non-zero exit, naming the command and its stderr's tail."""
    done = await run(argv, stdin=stdin, cwd=cwd, env=env)
    if done.returncode != 0:
        raise ProcessFailed(
            f"{' '.join(done.argv[:3])}… exited {done.returncode}: {done.err[-600:]}"
        )
    return done
