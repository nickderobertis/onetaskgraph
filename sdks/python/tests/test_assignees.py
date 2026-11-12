"""Opt-in assignees through the SDK, against the real binary over two folders that opted in.

A task created with an assignee, `task_assignee_set` replacing and clearing it, a held
`task_update_many` — an `assignee` of `None` set explicitly unassigns, and a guard on the whole
set answers `held` — and a copy guarded by `when_destination_assignees` each come back as their
typed results, and every write lands in the folder as the binary wrote it.
"""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client
from onetaskgraph_sdk._generated.batch_outcome import BatchOutcome
from onetaskgraph_sdk._generated.copy_report import (
    CopyOutcomeCreatedOrWouldCreate,
    CopyOutcomeUnchanged,
    CopyOutcomeUpdated,
)
from onetaskgraph_sdk._generated.task_assignee_set import TaskAssigneeSet
from onetaskgraph_sdk._generated.update_batch import UpdateBatch, UpdateBatchItem


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """Folders `plan` and `snap`, both opted in to assignees, `plan` holding one task."""
    tasks = tmp_path / "plan" / "tasks"
    tasks.mkdir(parents=True)
    (tasks / "pair.md").write_text(
        "---\ntitle: Pair\nstatus: todo\nassignees: [ana, ben]\n---\nThe body.\n",
        encoding="utf-8",
        newline="\n",
    )
    (tmp_path / "snap").mkdir()
    sources = {
        folder: {"plugin": "local-md", "config": {"root": folder, "assignees": True}}
        for folder in ("plan", "snap")
    }
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    (tmp_path / "body.md").write_text("Route me.\n", encoding="utf-8")
    return tmp_path


def client_at(binary: Path, store: Path) -> Client:
    """A client over the binary in `store`."""
    environment = {**os.environ, "HOME": str(store), "XDG_CONFIG_HOME": str(store)}
    return Client(binary, cwd=store, environment=environment)


def test_create_and_assignee_set_answer_typed_results(binary: Path, store: Path) -> None:
    """A create carries its one assignee; a set replaces it and, naming none, clears it."""
    client = client_at(binary, store)
    asyncio.run(
        client.task_create("plan", project="p", title="Routed", body_file="body.md", assignee="ana")
    )
    shown = asyncio.run(client.task_show("plan:routed"))
    assert shown.items[0].item.assignees == ["ana"]

    replaced = asyncio.run(client.task_assignee_set("plan:routed", "ben"))
    assert isinstance(replaced, TaskAssigneeSet)
    assert replaced.assignees == ["ben"]
    assert replaced.result is not None and replaced.result.version is not None
    cleared = asyncio.run(client.task_assignee_set("plan:routed"))
    assert isinstance(cleared, TaskAssigneeSet)
    assert cleared.assignees == []
    assert "assignees" not in (store / "plan" / "tasks" / "routed.md").read_text()


def test_update_many_guards_on_the_whole_set_and_an_explicit_none_unassigns(
    binary: Path, store: Path
) -> None:
    """A guard naming one of two people is `held`; naming both writes; `None` unassigns."""
    client = client_at(binary, store)
    answered = asyncio.run(
        client.task_update_many(
            "plan",
            UpdateBatch(
                schema_version=1,
                updates=[
                    UpdateBatchItem(item="plan:pair", assignee="cara", when_assignees=["ana"]),
                ],
            ),
        )
    )
    assert [result.outcome for result in answered.results] == [BatchOutcome.BatchOutcomeHeld]
    assert asyncio.run(client.task_show("plan:pair")).items[0].item.assignees == ["ana", "ben"]

    answered = asyncio.run(
        client.task_update_many(
            "plan",
            {
                "schema_version": 1,
                "updates": [
                    {"item": "plan:pair", "assignee": "cara", "when_assignees": ["ben", "ana"]}
                ],
            },
        )
    )
    assert [result.outcome for result in answered.results] == [BatchOutcome.BatchOutcomeWritten]
    assert asyncio.run(client.task_show("plan:pair")).items[0].item.assignees == ["cara"]

    answered = asyncio.run(
        client.task_update_many(
            "plan",
            UpdateBatch(
                schema_version=1,
                updates=[UpdateBatchItem(item="plan:pair", assignee=None)],
            ),
        )
    )
    assert [result.outcome for result in answered.results] == [BatchOutcome.BatchOutcomeWritten]
    assert asyncio.run(client.task_show("plan:pair")).items[0].item.assignees == []


def test_a_guarded_copy_reports_its_assignee_condition(binary: Path, store: Path) -> None:
    """Over a counterpart holding what the guard names, the source's assignee is written."""
    client = client_at(binary, store)
    asyncio.run(client.task_assignee_set("plan:pair", "ana"))
    first = asyncio.run(client.task_copy(["plan:pair"], to="snap"))
    copied = first.items[0].root
    assert isinstance(copied, CopyOutcomeCreatedOrWouldCreate)
    assert copied.destination is not None
    destination = str(copied.destination.root)
    assert asyncio.run(client.task_show(destination)).items[0].item.assignees == ["ana"]

    asyncio.run(client.task_assignee_set(destination, "dan"))
    asyncio.run(client.task_assignee_set("plan:pair", "eve"))
    kept = asyncio.run(
        client.task_copy(["plan:pair"], to="snap", when_destination_assignees=["fay"])
    )
    # Nothing else of it moved, so the counterpart keeping its own assignee is unchanged.
    outcome = kept.items[0].root
    assert isinstance(outcome, CopyOutcomeUnchanged)
    assert outcome.assignee_condition is not None
    assert (outcome.assignee_condition.held, outcome.assignee_condition.met) == (["dan"], False)
    assert asyncio.run(client.task_show(destination)).items[0].item.assignees == ["dan"]

    written = asyncio.run(
        client.task_copy(["plan:pair"], to="snap", when_destination_assignees=["dan"])
    )
    outcome = written.items[0].root
    assert isinstance(outcome, CopyOutcomeUpdated)
    assert outcome.assignee_condition is not None
    assert (outcome.assignee_condition.held, outcome.assignee_condition.met) == (["dan"], True)
    assert asyncio.run(client.task_show(destination)).items[0].item.assignees == ["eve"]
