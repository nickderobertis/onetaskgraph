"""`task_copy`'s guards, results and retry-safe creates, against the real binary over two folders.

A copy guarded with `when_destination_status`, `otherwise` and `when_destination_priority`
reaches the binary as one flag each, a counterpart at another category comes back as a parsed
`destination-status` outcome carrying `held` under exit 4 rather than raising, and every write
answers what it left as `result`. A copy stopped after it recorded its intent to create, run
again, comes back found `via` `intent`.
"""

from __future__ import annotations

import asyncio
import json
import os
import time
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError
from onetaskgraph_sdk._generated.copy_report import (
    CopyOutcomeCreatedOrWouldCreate,
    CopyOutcomeFailed,
    CopyOutcomeUnchanged,
    CopyOutcomeUpdated,
)


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """A folder `plan` holding one task, copied once into the folder `snap`."""
    tasks = tmp_path / "plan" / "tasks"
    tasks.mkdir(parents=True)
    (tasks / "ready.md").write_text(
        "---\ntitle: Ready\nstatus: todo\npriority: high\n---\nThe body.\n",
        encoding="utf-8",
        newline="\n",
    )
    (tmp_path / "snap").mkdir()
    sources = {
        folder: {"plugin": "local-md", "config": {"root": folder}} for folder in ("plan", "snap")
    }
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    return tmp_path


def client_at(binary: Path, store: Path, **extra: str) -> Client:
    """A client over the binary in `store`, with `extra` in its environment."""
    environment = {**os.environ, "HOME": str(store), "XDG_CONFIG_HOME": str(store), **extra}
    return Client(binary, cwd=store, environment=environment)


def revise(store: Path) -> None:
    """Retitle the source task and move it to `in-progress`, keeping its recorded link."""
    path = store / "plan" / "tasks" / "ready.md"
    path.write_text(
        path.read_text()
        .replace("title: Ready", "title: Ready, revised")
        .replace("status: todo", "status: in-progress")
    )


def test_a_status_guard_refuses_a_counterpart_at_another_category_as_a_parsed_outcome(
    binary: Path, store: Path
) -> None:
    """A refused counterpart is a parsed outcome with `held`; `preserve-destination` keeps it."""
    client = client_at(binary, store)
    first = asyncio.run(client.task_copy(["plan:ready"], to="snap"))
    created = first.items[0].root
    assert isinstance(created, CopyOutcomeCreatedOrWouldCreate)
    assert created.result is not None
    assert created.result.status is not None and created.result.status.category == "todo"
    assert created.result.priority == "high"
    revise(store)
    refused = asyncio.run(
        client.task_copy(
            ["plan:ready"], to="snap", when_destination_status=["backlog"], otherwise="refuse"
        )
    )
    outcome = refused.items[0].root
    assert isinstance(outcome, CopyOutcomeFailed)
    assert outcome.error.kind == "destination-status"
    assert outcome.status_condition is not None
    assert (outcome.status_condition.held, outcome.status_condition.met) == ("todo", False)
    assert outcome.held is not None and outcome.held.priority == "high"
    assert "title: Ready\n" in (store / "snap" / "tasks" / "ready.md").read_text()

    kept = asyncio.run(
        client.task_copy(
            ["plan:ready"],
            to="snap",
            when_destination_status=["backlog"],
            otherwise="preserve-destination",
            when_destination_priority=["none"],
        )
    )
    outcome = kept.items[0].root
    assert isinstance(outcome, CopyOutcomeUpdated)
    assert outcome.priority_condition is not None
    assert (outcome.priority_condition.held, outcome.priority_condition.met) == ("high", False)
    assert outcome.result is not None and outcome.result.priority == "high"
    written = (store / "snap" / "tasks" / "ready.md").read_text()
    assert "Ready, revised" in written and "status: todo" in written


def test_otherwise_without_a_status_guard_is_refused_before_anything_is_read(
    binary: Path, store: Path
) -> None:
    """`otherwise` alone is refused as `otherwise-without-status`."""
    client = client_at(binary, store)
    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(client.task_copy(["plan:ready"], to="snap", otherwise="refuse"))
    assert refused.value.failure is not None
    assert refused.value.failure.kind == "otherwise-without-status"


def test_a_priority_set_answers_what_it_left_and_a_comment_names_its_task(
    binary: Path, store: Path
) -> None:
    """A priority set answers `result` at a read's version; a comment names its task."""
    client = client_at(binary, store)
    set_ = asyncio.run(client.task_priority_set("plan:ready", "low"))
    assert set_.result is not None and set_.result.priority == "low"
    assert set_.result.version is not None
    shown = asyncio.run(client.task_show("plan:ready"))
    assert set_.result.version == shown.items[0].item.version
    added = asyncio.run(client.task_comment_add("plan:ready", body="A word.\n"))
    assert added.item is not None and added.item.root == "plan:ready"


def test_a_copy_stopped_before_its_link_is_finished_by_its_retry_found_via_intent(
    binary: Path, store: Path
) -> None:
    """A copy stopped before its link is finished by its retry, found via its intent."""
    # A killed copy's lease outlives it: the source says for how long, a second here, and the
    # retry waits it out.
    sources = json.loads((store / "onetaskgraph.yaml").read_text())
    sources["sources"]["plan"]["create_lease_seconds"] = 1
    (store / "onetaskgraph.yaml").write_text(json.dumps(sources), encoding="utf-8")
    stopped = client_at(binary, store, OTG_TEST_STOP_AT="before-link")
    with pytest.raises(OnetaskgraphError):
        asyncio.run(stopped.task_copy(["plan:ready"], to="snap"))
    time.sleep(1.5)
    client = client_at(binary, store)
    retried = asyncio.run(client.task_copy(["plan:ready"], to="snap"))
    outcome = retried.items[0].root
    assert isinstance(outcome, (CopyOutcomeUpdated, CopyOutcomeUnchanged))
    assert outcome.via == "intent"
    assert len(list((store / "snap" / "tasks").glob("*.md"))) == 1
    assert "onetaskgraph.creating" not in (store / "plan" / "tasks" / "ready.md").read_text()
