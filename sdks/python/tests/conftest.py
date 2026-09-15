"""Fixtures shared by every test of the SDK."""

from __future__ import annotations

import os

import pytest


@pytest.fixture(autouse=True)
def no_ambient_configuration(monkeypatch: pytest.MonkeyPatch) -> None:
    """Remove the ambient `ONETASKGRAPH_` variables before each test.

    A client hands the binary its own environment, and the binary reads
    `ONETASKGRAPH_SOURCES__<NAME>__...` as a configuration layer: a shell that exports one
    adds a source to every query these tests make, and an assertion on the first row reads
    that source instead of the one the test configured. Only these are removed, as the
    binary's own journeys remove them, because clearing the whole environment would take
    `PATH` and the coverage variables with it.
    """
    for name in list(os.environ):
        if name.startswith("ONETASKGRAPH_"):
            monkeypatch.delenv(name)
