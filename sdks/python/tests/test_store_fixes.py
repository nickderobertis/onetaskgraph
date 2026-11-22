"""`sources_quota` and a duplicate close, against the real binary over a folder of Markdown.

A folder reports no reading, so its quota is a source with no resources; a duplicate close leaves
the task `cancelled` with its `duplicate` link at both ends, and beside another category the
binary refuses it writing nothing. The hosted backends' own answers are the Rust suites'.
"""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError
from onetaskgraph_sdk._generated.source_quota import SourceQuota


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """A folder `plan` holding T-1 and T-2 at `todo`."""
    tasks = tmp_path / "plan" / "tasks"
    tasks.mkdir(parents=True)
    for n in (1, 2):
        (tasks / f"T-{n}.md").write_text(
            f"---\ntitle: Task {n}\nstatus: todo\n---\nTask {n}.\n",
            encoding="utf-8",
            newline="\n",
        )
    sources = {"plan": {"plugin": "local-md", "config": {"root": "plan"}}}
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    return tmp_path


def client_at(binary: Path, store: Path) -> Client:
    """A client over the binary in `store`, its configuration that folder's alone."""
    environment = {**os.environ, "HOME": str(store), "XDG_CONFIG_HOME": str(store)}
    return Client(binary, cwd=store, environment=environment)


def task(store: Path, n: int) -> str:
    """The file of task `T-n`."""
    return (store / "plan" / "tasks" / f"T-{n}.md").read_text(encoding="utf-8")


def test_sources_quota_answers_a_folder_s_empty_reading_and_refuses_an_unknown_source(
    binary: Path, store: Path
) -> None:
    """A folder holds no allowance; a source nothing configures is the binary's refusal."""
    client = client_at(binary, store)
    quota = asyncio.run(client.sources_quota("plan"))
    assert isinstance(quota, SourceQuota)
    assert (str(quota.source.root), quota.resources) == ("plan", [])
    with pytest.raises(OnetaskgraphError):
        asyncio.run(client.sources_quota("elsewhere"))


def test_task_status_set_with_duplicate_of_closes_the_task_with_its_link(
    binary: Path, store: Path
) -> None:
    """The close lands with the link at both ends; beside `done` it is refused, writing nothing."""
    client = client_at(binary, store)
    closed = asyncio.run(client.task_status_set("plan:T-1", "cancelled", duplicate_of="plan:T-2"))
    assert closed.status.category.value == "cancelled"
    assert "status: cancelled" in task(store, 1)
    assert "## Links" in task(store, 2)
    before = task(store, 2)
    with pytest.raises(OnetaskgraphError, match="cancelled"):
        asyncio.run(client.task_status_set("plan:T-2", "done", duplicate_of="plan:T-1"))
    assert task(store, 2) == before


def test_task_update_with_duplicate_of_closes_the_task_as_a_duplicate(
    binary: Path, store: Path
) -> None:
    """An update naming only the survivor closes the task `cancelled` and says what it wrote."""
    client = client_at(binary, store)
    updated = asyncio.run(client.task_update("plan:T-1", duplicate_of="plan:T-2"))
    assert updated.task.status.category.value == "cancelled"
    assert "duplicate-of" in [field.value for field in updated.written]
