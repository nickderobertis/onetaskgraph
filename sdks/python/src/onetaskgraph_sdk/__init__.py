"""The Python SDK for onetaskgraph.

The client surface is generated from the schema bundle `onetaskgraph schema` emits and
lands with a later change. What is here now is the version this package publishes, kept
in one place so the generated surface and the distribution metadata cannot disagree.
"""

from importlib.metadata import PackageNotFoundError, version

__all__ = ["__version__", "distribution_version"]

#: The version this package publishes. `pyproject.toml` must agree; see the tests.
__version__ = "0.1.0"


def distribution_version() -> str:
    """Return the version recorded in the installed distribution's metadata.

    Falls back to :data:`__version__` when the package is imported from a source tree
    that was never installed, so a contributor running the tests from a clean clone sees
    the same answer as a user who installed the wheel.
    """
    try:
        return version("onetaskgraph-sdk")
    except PackageNotFoundError:
        return __version__
