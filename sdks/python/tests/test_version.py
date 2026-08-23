"""The one contract this package carries today: its version is stated once.

That `__version__` agrees with `pyproject.toml` — and with the Cargo workspace and the
TypeScript package, which it must, since one product version spans all four — is
reconciled by `scripts/check-workspace-config.sh` in the deterministic gate, where the
other three copies are already checked against each other. Asserting it here as well
would be a second place that knows about only two of the four.
"""

import onetaskgraph_sdk


def test_the_version_is_a_plain_semantic_version() -> None:
    """The release pipeline parses this, so a decorated version would break it."""
    major, minor, patch = onetaskgraph_sdk.__version__.split(".")

    assert all(part.isdigit() for part in (major, minor, patch))
