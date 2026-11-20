"""`sources migrate` through the SDK: its report comes back on exit 0, and on exit 1 too.

A folder of Markdown has only ever written one stored form, so against the real binary it answers
that it has nothing to migrate and its files do not move. A migration that left an item
unrewritten answers its report on exit 1, and only a Linear source can produce one; the Linear
suite drives that against the loopback Linear, so here a stand-in executable answers exactly the
document the binary writes then, to hold the SDK's own half: the report is returned, typed, and
not raised as a failure.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError
from onetaskgraph_sdk._generated.stored_forms_migrated import StoredFormsMigrated


def tree(root: Path) -> dict[str, bytes]:
    """Every file under `root`, by its path, with its bytes."""
    return {str(path): path.read_bytes() for path in sorted(root.rglob("*")) if path.is_file()}


@pytest.fixture
def folder(tmp_path: Path) -> Path:
    """A folder of Markdown holding one task, configured as the source `plan`."""
    plan = tmp_path / "plan"
    (plan / "tasks").mkdir(parents=True)
    (plan / "tasks" / "T-1.md").write_text(
        '---\ntitle: Build it\nstatus: todo\nmetadata: {"caller.key": 1}\n---\nBody.\n',
        encoding="utf-8",
        newline="\n",
    )
    sources = {"plan": {"plugin": "local-md", "config": {"root": str(plan)}}}
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    return plan


def client_over(binary: Path | str, tmp_path: Path) -> Client:
    """A client reading no configuration but the one in `tmp_path`."""
    environment = {**os.environ, "HOME": str(tmp_path), "XDG_CONFIG_HOME": str(tmp_path)}
    return Client(binary, cwd=tmp_path, environment=environment)


def test_a_folder_of_markdown_answers_nothing_to_migrate_and_moves_nothing(
    binary: Path, tmp_path: Path, folder: Path
) -> None:
    """Both a dry run and a real one are typed reports naming nothing, and no file moves."""
    client = client_over(binary, tmp_path)
    before = tree(folder)
    for dry_run in (True, None):
        report = asyncio.run(client.sources_migrate("plan", dry_run=dry_run))
        assert isinstance(report, StoredFormsMigrated)
        assert report.model_dump(mode="json") == {
            "source": "plan",
            "dry_run": dry_run is True,
            "rewritten": [],
            "current": 0,
            "failed": [],
        }
    assert tree(folder) == before


def test_an_unknown_source_is_refused_as_a_failure(
    binary: Path, tmp_path: Path, folder: Path
) -> None:
    """A source nothing configures never runs, and is raised rather than reported."""
    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(client_over(binary, tmp_path).sources_migrate("elsewhere"))
    assert refused.value.exit_code == 2


# What `sources migrate work --json` writes when one item was rewritten and Linear refused
# another, exit 1 — the shape `collapsed_slot.rs` in the Linear suite drives the binary to.
FAILED = {
    "source": "work",
    "dry_run": False,
    "rewritten": [{"kind": "task", "id": "work:I-1", "from": "html-comment-slot"}],
    "current": 3,
    "failed": [
        {
            "kind": "document",
            "id": "work:D-1",
            "from": "multi-line-comment-slot",
            "reason": "Linear reported documentUpdate was unsuccessful",
        }
    ],
}


@pytest.mark.skipif(
    sys.platform == "win32", reason="the stand-in is a script run by its shebang line"
)
def test_a_migration_that_left_an_item_unrewritten_returns_its_report_on_exit_1(
    tmp_path: Path,
) -> None:
    """Exit 1 with a report is the report, typed, whatever `raise_on_partial` says."""
    stand_in = tmp_path / "onetaskgraph"
    stand_in.write_text(
        f"#!{sys.executable}\n"
        "import json, sys\n"
        "assert sys.argv[1:4] == ['sources', 'migrate', 'work'], sys.argv\n"
        f"print(json.dumps({FAILED!r}))\n"
        "sys.exit(1)\n",
        encoding="utf-8",
    )
    stand_in.chmod(0o755)
    report = asyncio.run(client_over(stand_in, tmp_path).sources_migrate("work"))
    # `from` is a Python keyword: the model holds it as `from_`, and writes it back as `from`.
    assert report.rewritten[0].from_ == "html-comment-slot"
    assert report.model_dump(mode="json", by_alias=True) == FAILED
