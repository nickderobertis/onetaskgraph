"""The package version remains a plain semantic version."""

import onetaskgraph_sdk


def test_the_version_is_a_plain_semantic_version() -> None:
    """The release pipeline parses this, so a decorated version would break it."""
    major, minor, patch = onetaskgraph_sdk.__version__.split(".")
    assert all(part.isdigit() for part in (major, minor, patch))
