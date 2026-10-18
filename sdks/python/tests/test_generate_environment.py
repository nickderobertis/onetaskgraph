"""The generator emits the build's contract whatever configuration its caller carries."""

from __future__ import annotations

import asyncio
import json
import sys
from pathlib import Path

import pytest

PACKAGE = Path(__file__).parents[1]

# A source its own plugin refuses — a field that plugin does not accept — is what a host
# exporting one source's settings into every process can hand the generator, and every verb
# refuses it, `schema` included.
STRAY = {
    "ONETASKGRAPH_SOURCES__STRAY__PLUGIN": "in-memory",
    "ONETASKGRAPH_SOURCES__STRAY__CONFIG__TEAM": "ENG",
}


async def schema(binary: Path) -> tuple[int | None, str]:
    """What `onetaskgraph schema` exits with and writes to stderr, run as the caller is."""
    process = await asyncio.create_subprocess_exec(
        str(binary),
        "schema",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    _, stderr = await process.communicate()
    return process.returncode, stderr.decode("utf-8")


def test_a_callers_configuration_does_not_reach_the_binary(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The real binary refuses the stray setting, and the generator emits the bundle anyway."""
    sys.path.insert(0, str(PACKAGE))
    import generate

    for name, value in STRAY.items():
        monkeypatch.setenv(name, value)
    code, stderr = asyncio.run(schema(generate.BINARY))
    assert code != 0, "the stray setting no longer makes the binary refuse"
    assert "sources.stray" in stderr

    bundle = generate.validate_schema_bundle(json.loads(generate.run_workspace_binary("schema")))
    assert "StatusMapping" in bundle["roots"]
