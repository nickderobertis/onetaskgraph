"""The package version remains a plain semantic version, and its interpreter floor is one."""

import sys
import tomllib
from pathlib import Path

import onetaskgraph_sdk

PACKAGE = Path(__file__).parents[1]
WORKSPACE = Path(__file__).parents[3]


def test_the_version_is_a_plain_semantic_version() -> None:
    """The release pipeline parses this, so a decorated version would break it."""
    major, minor, patch = onetaskgraph_sdk.__version__.split(".")
    assert all(part.isdigit() for part in (major, minor, patch))


def test_the_supported_floor_is_the_interpreter_every_lane_runs() -> None:
    """The oldest Python this package declares is the one its checks run and type-check on.

    `requires-python` is what an installer reads, `tool.ty.environment.python-version` is what
    the type check assumes, and the workspace's `.python-version` is the interpreter `uv run`
    gives every lane — so a package claiming a floor its tests never ran on, or type-checked
    against a newer standard library than it supports, fails here rather than on a user's
    machine.
    """
    project = tomllib.loads((PACKAGE / "pyproject.toml").read_text(encoding="utf-8"))
    requires = project["project"]["requires-python"]
    assert requires.startswith(">="), requires
    floor = requires.removeprefix(">=")
    assert project["tool"]["ty"]["environment"]["python-version"] == floor
    assert (WORKSPACE / ".python-version").read_text(encoding="utf-8").strip() == floor
    major, minor = (int(part) for part in floor.split("."))
    running = sys.version_info[:2]
    assert running == (major, minor), (
        f"these tests run on Python {running[0]}.{running[1]}, not the declared floor {floor}"
    )
