"""The one contract this package carries today: its version is stated once."""

import tomllib
from pathlib import Path

import onetaskgraph_sdk


def test_the_module_version_matches_the_published_package_version() -> None:
    """A generated surface pinned to a version the wheel disagrees with is a bug.

    Reading `pyproject.toml` from the tree rather than the installed metadata is
    deliberate: it catches a hand-edited module constant that was never released.
    """
    manifest = Path(__file__).resolve().parents[1] / "pyproject.toml"
    declared = tomllib.loads(manifest.read_text())["project"]["version"]

    assert onetaskgraph_sdk.__version__ == declared


def test_the_version_is_a_plain_semantic_version() -> None:
    """The release pipeline parses this, so a decorated version would break it."""
    major, minor, patch = onetaskgraph_sdk.__version__.split(".")

    assert all(part.isdigit() for part in (major, minor, patch))
