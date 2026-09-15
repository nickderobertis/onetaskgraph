"""Fixtures shared by every test of the SDK."""

from __future__ import annotations

import asyncio
import subprocess
import sys
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client
from onetaskgraph_sdk._generated.effective_config import OriginEnvironment

WORKSPACE = Path(__file__).parents[3]


@pytest.fixture(scope="session")
def binary() -> Path:
    """Build and return the real workspace executable."""
    subprocess.run(
        ["cargo", "build", "--quiet", "-p", "onetaskgraph", "--bin", "onetaskgraph"],
        cwd=WORKSPACE,
        check=True,
    )
    suffix = ".exe" if sys.platform == "win32" else ""
    return (WORKSPACE / "target" / "debug" / f"onetaskgraph{suffix}").resolve()


@pytest.fixture(scope="session")
def ambient_configuration(binary: Path, tmp_path_factory: pytest.TempPathFactory) -> list[str]:
    """Name every variable of this process the binary reads as its environment layer.

    The binary answers rather than a prefix written here, so what is removed is what it
    reads, and it is asked from an empty directory so no file layer is reported beside it.
    """
    effective = asyncio.run(Client(binary, cwd=tmp_path_factory.mktemp("ambient")).config_show())
    return [
        setting.origin.root.variable
        for setting in effective.settings
        if isinstance(setting.origin.root, OriginEnvironment)
    ]


@pytest.fixture(autouse=True)
def no_ambient_configuration(
    ambient_configuration: list[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """Remove the ambient environment-layer configuration before each test.

    A client hands the binary its own environment, so a shell that exports a source adds
    it to every query these tests make, and an assertion on the first row reads that
    source instead of the one the test configured. Only those variables are removed,
    because clearing the whole environment would take `PATH` and the coverage variables
    with it.
    """
    for name in ambient_configuration:
        monkeypatch.delenv(name, raising=False)
