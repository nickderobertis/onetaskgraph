"""The generator emits the build's contract whatever configuration its caller carries."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

PACKAGE = Path(__file__).parents[1]

# Half a source — a setting with no plugin — is what a host exporting one source's settings
# into every process hands the generator, and every verb refuses it, `schema` included.
STRAY = "ONETASKGRAPH_SOURCES__STRAY__CONFIG__TEAM"


def test_a_callers_configuration_does_not_reach_the_binary(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The real binary refuses the stray setting, and the generator emits the bundle anyway."""
    sys.path.insert(0, str(PACKAGE))
    import generate

    monkeypatch.setenv(STRAY, "ENG")
    direct = subprocess.run(
        [str(generate.BINARY), "schema"],
        capture_output=True,
        encoding="utf-8",
        check=False,
    )
    assert direct.returncode != 0, "the stray setting no longer makes the binary refuse"
    assert "sources.stray" in direct.stderr

    bundle = generate.validate_schema_bundle(json.loads(generate.run_workspace_binary("schema")))
    assert "StatusMapping" in bundle["roots"]
