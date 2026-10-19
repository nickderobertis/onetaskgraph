"""`task_copy(status=...)`, against the real binary over two folders of Markdown.

The categories reach the binary as one `--status` flag each, so a copy by query keeps exactly
the tasks at them and a copy by id answers a task at another category `skipped`, writing only
what it kept into the destination folder.
"""

from __future__ import annotations

import asyncio
import json
from pathlib import Path
from typing import Literal

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError
from onetaskgraph_sdk._generated.copy_report import (
    CopyOutcomeCreatedOrWouldCreate,
    CopyOutcomeSkipped,
    CopyReport,
    GlobalId,
)

type TaskName = str
"""A task file's stem in a folder of Markdown, which is also that task's native id there."""

type Category = Literal["backlog", "todo", "done"]
"""The status categories this store's tasks sit at, spelled as `task_copy` takes them."""

PLAN: dict[TaskName, Category] = {
    "proposal": "backlog",
    "ready": "todo",
    "shipped": "done",
}
"""Each task of the `plan` folder and the category its status reads as."""


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """A folder `plan` holding `PLAN`, and an empty folder `snap` to copy into."""
    tasks = tmp_path / "plan" / "tasks"
    tasks.mkdir(parents=True)
    for name, category in PLAN.items():
        (tasks / f"{name}.md").write_text(
            f"---\ntitle: {name}\nstatus: {category}\n---\nThe body.\n",
            encoding="utf-8",
            newline="\n",
        )
    (tmp_path / "snap").mkdir()
    sources = {
        folder: {"plugin": "local-md", "config": {"root": folder}} for folder in ("plan", "snap")
    }
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    return tmp_path


def copied(store: Path) -> list[TaskName]:
    """The task files the destination folder holds, by name."""
    tasks = store / "snap" / "tasks"
    return sorted(path.stem for path in tasks.glob("*.md")) if tasks.is_dir() else []


def created(report: CopyReport) -> list[GlobalId]:
    """The source ids of every item `report` answers created, in the order answered."""
    return [
        item.root.source
        for item in report.items
        if isinstance(item.root, CopyOutcomeCreatedOrWouldCreate)
    ]


def skipped(report: CopyReport) -> list[CopyOutcomeSkipped]:
    """Every item `report` answers skipped, in the order answered."""
    return [item.root for item in report.items if isinstance(item.root, CopyOutcomeSkipped)]


def plan(name: TaskName) -> GlobalId:
    """The qualified id of the `plan` folder's task `name`, as a copy's outcome carries it."""
    return GlobalId(f"plan:{name}")


def test_a_copy_by_query_keeps_exactly_the_tasks_at_every_category_named(
    binary: Path, store: Path
) -> None:
    """Two categories are two flags: both are kept, and the third is neither named nor copied."""
    client = Client(binary, cwd=store)
    report = asyncio.run(
        client.task_copy(
            [],
            source=["plan"],
            status=["backlog", "todo"],
            to="snap",
        )
    )
    assert created(report) == [plan("proposal"), plan("ready")]
    assert len(report.items) == 2, report
    assert copied(store) == ["proposal", "ready"]


def test_a_copy_by_id_answers_a_task_at_another_category_skipped_and_writes_only_the_kept(
    binary: Path, store: Path
) -> None:
    """An id is read only as far as its status, and the reason names both categories."""
    client = Client(binary, cwd=store)
    report = asyncio.run(
        client.task_copy(
            [plan("proposal").root, plan("shipped").root],
            status=("backlog",),
            to="snap",
        )
    )
    assert created(report) == [plan("proposal")]
    [answered] = skipped(report)
    assert answered.source == plan("shipped")
    assert answered.reason == "status done is not one of backlog"
    assert copied(store) == ["proposal"]


def test_a_category_the_binary_does_not_know_is_refused_naming_the_flag(
    binary: Path, store: Path
) -> None:
    """A value outside the categories reaches the binary and comes back a usage error."""
    client = Client(binary, cwd=store)
    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(
            client.task_copy(
                [],
                source=["plan"],
                # A board's option name where a category belongs is the mistake under test, so
                # the type checker's refusal of it is the point rather than a defect here.
                status=["shipped"],  # ty: ignore[invalid-argument-type]
                to="snap",
            )
        )
    assert "--status" in str(refused.value)
    assert refused.value.exit_code == 2
    assert copied(store) == []
