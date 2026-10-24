"""Shared fixtures: the loopback fake GitHub and the environment a `gaps` subprocess runs in."""

from __future__ import annotations

import os
import sys
from collections.abc import Iterator
from pathlib import Path

import pytest

from tests import fake_github

PACKAGE_ROOT = Path(__file__).resolve().parents[1]
REPO = PACKAGE_ROOT.parents[1]


@pytest.fixture
def github() -> Iterator[tuple[fake_github.Board, str]]:
    """A fresh stand-in board and its loopback URL."""
    board = fake_github.Board()
    server, url = fake_github.serve(board)
    yield board, url
    server.shutdown()


def gaps_env(tmp_path: Path, **extra: str) -> dict[str, str]:
    """The environment `python -m gaps` runs in for a test: its own cache and logs."""
    env = {k: v for k, v in os.environ.items() if not k.startswith(("GH_", "SPIKE_"))}
    env |= {
        "XDG_CACHE_HOME": str(tmp_path / "cache"),
        "SPIKE_GAPS_LOGS": str(tmp_path / "logs"),
        "PYTHONPATH": os.pathsep.join([str(PACKAGE_ROOT), str(PACKAGE_ROOT.parent / "harness")]),
    }
    return env | extra


PYTHON = sys.executable
