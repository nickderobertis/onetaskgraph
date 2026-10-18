"""`task_copy(status=...)`, against the real binary over two folders of Markdown.

The categories reach the binary as one `--status` flag each, so a copy by query keeps exactly
the tasks at them and a copy by id answers a task at another category `skipped`, writing only
what it kept into the destination folder.
"""

from __future__ import annotations

import asyncio
import json
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError
from onetaskgraph_sdk._generated.models import CopyReport

PLAN = {"proposal": "backlog", "ready": "todo", "shipped": "done"}
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


def copied(store: Path) -> list[str]:
    """The task files the destination folder holds, by name."""
    tasks = store / "snap" / "tasks"
    return sorted(path.stem for path in tasks.glob("*.md")) if tasks.is_dir() else []


def actions(report: CopyReport) -> list[tuple[str, str]]:
    """Each item of `report` as its source id and its action."""
    answered = report.model_dump(mode="json")["items"]
    return [(item["source"], item["action"]) for item in answered]


def test_a_copy_by_query_keeps_exactly_the_tasks_at_every_category_named(
    binary: Path, store: Path
) -> None:
    """Two categories are two flags: both are kept, and the third is neither named nor copied."""
    client = Client(binary, cwd=store)
    report = asyncio.run(
        client.task_copy([], source=["plan"], status=["backlog", "todo"], to="snap")
    )
    assert actions(report) == [("plan:proposal", "created"), ("plan:ready", "created")]
    assert copied(store) == ["proposal", "ready"]


def test_a_copy_by_id_answers_a_task_at_another_category_skipped_and_writes_only_the_kept(
    binary: Path, store: Path
) -> None:
    """An id is read only as far as its status, and the reason names both categories."""
    client = Client(binary, cwd=store)
    report = asyncio.run(
        client.task_copy(["plan:proposal", "plan:shipped"], status=("backlog",), to="snap")
    )
    assert actions(report) == [("plan:proposal", "created"), ("plan:shipped", "skipped")]
    skipped = report.model_dump(mode="json")["items"][1]
    assert skipped["reason"] == "status done is not one of backlog"
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
