"""The one contract this package carries today: its version is stated once."""

import tomllib
from pathlib import Path

import onetaskgraph_sdk


def test_the_module_version_matches_the_distribution_metadata() -> None:
    """A generated surface pinned to a version that disagrees with the wheel is a bug.

    Reading `pyproject.toml` from the tree rather than trusting the installed metadata is
    deliberate: it catches a hand-edited module constant that was never released.
    """
    manifest = Path(__file__).resolve().parents[1] / "pyproject.toml"
    declared = tomllib.loads(manifest.read_text())["project"]["version"]

    assert onetaskgraph_sdk.__version__ == declared


def test_the_distribution_version_resolves_without_an_installed_wheel() -> None:
    """A clean clone has no installed distribution; the fallback must still answer."""
    assert onetaskgraph_sdk.distribution_version() == onetaskgraph_sdk.__version__
