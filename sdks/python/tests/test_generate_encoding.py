"""The generator writes and compares UTF-8 whatever the platform's default encoding is."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

PACKAGE = Path(__file__).parents[1]
MARKER = "Carried through generation — café, naïve, ✓."
ROOT = "CopyReport"

# Run in a child interpreter so the staged encoding is the one it starts under: the default
# `open` and `subprocess` read is fixed at interpreter start and cannot be changed afterwards.
# The marker is spelled in escapes because the command line itself is decoded in that
# encoding, before any of the generator runs.
#
# Generation is narrowed to the one root carrying the marker, and the client is left out:
# walking the binary's help for the client costs more than a minute a pass, every root adds
# seconds, and neither is what this proves. `generate-check` itself still regenerates them all.
CHILD = f"""
import copy, json, locale, sys
from pathlib import Path
sys.path.insert(0, {str(PACKAGE)!r})
import generate

print(locale.getencoding(), flush=True)
bundle = generate.validate_schema_bundle(json.loads(generate.run_workspace_binary("schema")))
bundle["roots"][{ROOT!r}]["properties"]["delivered"]["description"] = {ascii(MARKER)}
generate.RESPONSE_ROOTS = {{"task_copy": {ROOT!r}}}
generate.CONTRACT_ROOTS = set()
written, regenerated = Path(sys.argv[1]), Path(sys.argv[2])
for destination in (written, regenerated):
    generate.generate_models(copy.deepcopy(bundle), destination)
    generate.format_generated(destination)
generate.check_generated(regenerated, written)
"""


def test_generation_is_utf8_under_a_legacy_default_encoding(tmp_path: Path) -> None:
    """A description outside ASCII is written as UTF-8 and the check compares it unchanged.

    The Windows runner's default encoding is its ANSI code page. The closest a Linux host can
    stage is the C locale with coercion and UTF-8 mode both off, which makes the default
    ASCII: stricter than a code page, because text it cannot represent fails outright rather
    than being mangled. The real generator reads the real binary's schema, writes a package,
    then regenerates it and compares the two with the comparison `generate.py --check` makes.
    """
    env = {**os.environ, "PYTHONUTF8": "0", "PYTHONCOERCECLOCALE": "0", "LC_ALL": "C"}
    written = tmp_path / "written"
    result = subprocess.run(
        [sys.executable, "-c", CHILD, str(written), str(tmp_path / "regenerated")],
        cwd=PACKAGE,
        env=env,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    staged = result.stdout.splitlines()[0].lower() if result.stdout else ""
    if staged.replace("-", "") == "utf8":
        if sys.platform in {"linux", "win32"}:
            pytest.fail(f"the staged default encoding did not take: the child reads {staged}")
        pytest.skip(f"this platform keeps a UTF-8 default under the C locale ({staged})")
    assert result.returncode == 0, (
        f"generation under the {staged} default encoding failed:\n{result.stderr}"
    )

    module = written / "copy_report.py"
    assert MARKER.encode("utf-8") in module.read_bytes(), (
        f"{module.name} does not carry the description as UTF-8; pass an explicit UTF-8 "
        "encoding to whatever in generate.py wrote or read it"
    )
