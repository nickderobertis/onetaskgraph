"""Installed-wheel journey."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


def test_wheel_installs_and_queries_through_public_import(tmp_path: Path) -> None:
    """Install a wheel cleanly and drive a real configured query through it."""
    package = Path(__file__).parents[1]
    workspace = package.parents[1]
    subprocess.run(["uv", "build", "--wheel", "--out-dir", str(tmp_path)], cwd=package, check=True)
    environment = tmp_path / "venv"
    subprocess.run(["uv", "venv", "--python", sys.executable, str(environment)], check=True)
    python = environment / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    wheel = next(tmp_path.glob("onetaskgraph_sdk-*.whl"))
    subprocess.run(["uv", "pip", "install", "--python", str(python), str(wheel)], check=True)
    config = tmp_path / "query"
    config.mkdir()
    (config / "onetaskgraph.yaml").write_text(
        '{"sources":{"work":{"plugin":"in-memory","config":{"tasks":['
        '{"id":"T-1","title":"Installed","status":{"category":"todo","name":"Todo"},'
        '"labels":[]}]}}}}',
        encoding="utf-8",
    )
    suffix = ".exe" if os.name == "nt" else ""
    binary = (workspace / "target" / "debug" / f"onetaskgraph{suffix}").resolve()
    script = (
        "from onetaskgraph_sdk import Client; "
        f"r=Client({str(binary)!r}, cwd={str(config)!r}).task_list(); "
        "assert r.items[0].item.title == 'Installed'"
    )
    subprocess.run([str(python), "-c", script], check=True)
