"""Start and stop the guarded recording proxy (`gaps.guard`) for one run."""

from __future__ import annotations

import asyncio
import os
import sys
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from dataclasses import dataclass
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parents[1]
HARNESS = PACKAGE_ROOT.parent / "harness"
STARTUP = 10.0


@dataclass(frozen=True)
class Relay:
    """A running proxy: where to send requests, and where it logs them."""

    endpoint: str
    log: Path


@asynccontextmanager
async def relay(
    run_dir: Path,
    allowed: tuple[str, ...],
    upstream: str | None = None,
    issues: tuple[str, ...] = (),
) -> AsyncIterator[Relay]:
    """The guard, stopped when the block ends.

    It allows writes naming only `allowed` destinations and the issues it sees created, plus
    `issues`, an interrupted probe's leftovers.
    """
    port_file = run_dir / "proxy.port"
    port_file.unlink(missing_ok=True)
    log = run_dir / "proxy.jsonl"
    argv = [sys.executable, "-m", "gaps.guard", "--log", str(log), "--port-file", str(port_file)]
    for allowed_id in allowed:
        argv += ["--allow", allowed_id]
    for issue in issues:
        argv += ["--allow-issue", issue]
    if upstream is not None:
        argv += ["--upstream", upstream]
    env = dict(os.environ, PYTHONPATH=os.pathsep.join([str(PACKAGE_ROOT), str(HARNESS)]))
    process = await asyncio.create_subprocess_exec(*argv, env=env)
    try:
        port = await _port(port_file, process)
        yield Relay(endpoint=f"http://127.0.0.1:{port}/graphql", log=log)
    finally:
        if process.returncode is None:
            process.terminate()
            await process.wait()


async def _port(port_file: Path, process: asyncio.subprocess.Process) -> int:
    waited = 0.0
    while waited < STARTUP:
        if process.returncode is not None:
            raise RuntimeError(f"the proxy exited {process.returncode} before listening")
        if port_file.exists() and port_file.read_text().strip().isdigit():
            return int(port_file.read_text())
        await asyncio.sleep(0.05)
        waited += 0.05
    raise TimeoutError(f"the proxy did not listen within {STARTUP} s")
