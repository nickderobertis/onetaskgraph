"""Fixtures shared by every test of the SDK."""

from __future__ import annotations

import asyncio
import os
import sys
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client
from onetaskgraph_sdk._generated.effective_config import OriginEnvironment

WORKSPACE = Path(__file__).parents[3]

CONFIGURATION_PREFIX = "ONETASKGRAPH_"
"""The prefix of every variable the binary reads as configuration, removed before each test."""


@pytest.fixture(scope="session")
def binary() -> Path:
    """Return the real workspace executable, which `onetaskgraph:build` produced.

    Nothing here builds it: scripts/check-workspace-config.sh says why a spawner never does.
    """
    return workspace_binary()


def workspace_binary() -> Path:
    """Resolve the executable, refusing with the target to run when it is absent."""
    suffix = ".exe" if sys.platform == "win32" else ""
    binary = (WORKSPACE / "target" / "debug" / f"onetaskgraph{suffix}").resolve()
    if not binary.is_file():
        pytest.fail(
            f"{binary} is missing; run `scripts/nx.sh run onetaskgraph:build` from the "
            "workspace root, which is what every Nx target that spawns it depends on"
        )
    return binary


@pytest.fixture(scope="session")
def configuration_prefix(binary: Path, tmp_path_factory: pytest.TempPathFactory) -> str:
    """Return the prefix removed below, once the binary has shown it reads a variable under it.

    A source exported under the prefix, and nothing else under it, has to come back as the one
    environment-layer setting the binary reports, from a directory with no configuration file;
    otherwise the prefix removed here is not the binary's, and every test fails saying so.
    """
    probe = f"{CONFIGURATION_PREFIX}SOURCES__PREFIX_PROBE__PLUGIN"
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.startswith(CONFIGURATION_PREFIX)
    }
    environment[probe] = "in-memory"
    client = Client(binary, cwd=tmp_path_factory.mktemp("prefix"), environment=environment)
    effective = asyncio.run(client.config_show())
    read = [
        setting.origin.root.variable
        for setting in effective.settings
        if isinstance(setting.origin.root, OriginEnvironment)
    ]
    assert read == [probe], (
        f"the binary read {read} as its environment layer rather than {probe}; "
        "set CONFIGURATION_PREFIX to the prefix it reads"
    )
    return CONFIGURATION_PREFIX


@pytest.fixture(autouse=True)
def no_ambient_configuration(configuration_prefix: str, monkeypatch: pytest.MonkeyPatch) -> None:
    """Remove every variable under the binary's configuration prefix before each test.

    A client, and every subprocess a test starts, hands the binary this process's
    environment, so a shell that exports a source adds it to every query these tests make, and
    an assertion on the first row reads that source instead of the one the test configured.
    Every variable under the prefix goes, as the binary's own journeys remove them, and nothing
    wider: clearing the whole environment would take `PATH` and the coverage variables with it.
    """
    for name in list(os.environ):
        if name.startswith(configuration_prefix):
            monkeypatch.delenv(name)
