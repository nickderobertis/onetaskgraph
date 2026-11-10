"""The follow-up loop's store features through the SDK, against the real binary over folders.

Every write this SDK holds to a version lands at the task's version and raises
`VersionMovedError` — carrying the store's whole refusal, its `version-moved` kind and the
version the task is at — at a stale one, the task left as it was. Beside them: a partial copy's
typed result, a guarded priority set answering `held`, a repositories update and clear, a
comments-since listing and its watermark, an all-pages listing and a show with dependencies.
The item models are imported from the package root, where a caller is owed them.
"""

from __future__ import annotations

import asyncio
import json
import os
from collections.abc import Callable, Coroutine
from pathlib import Path

import pytest

from onetaskgraph_sdk import (
    Client,
    Comment,
    CommentActivity,
    CommentsSince,
    CopyAction,
    CopyLink,
    CopyOutcome,
    CopyReport,
    CopyVia,
    Located,
    OnetaskgraphError,
    QualifiedTask,
    Task,
    VersionMovedError,
)

MAPPING = {
    "backlog": "backlog",
    "todo": "todo",
    "queued": "queued",
    "in-progress": "in-progress",
    "done": "done",
    "cancelled": "cancelled",
}


def task_file(folder: Path, name: str, front: str = "", status: str = "todo") -> None:
    """Write the task `name` into `folder`, at `status`, with `front` beside its title."""
    tasks = folder / "tasks"
    tasks.mkdir(parents=True, exist_ok=True)
    (tasks / f"{name}.md").write_text(
        f"---\ntitle: {name}\nstatus: {status}\npriority: high\n{front}---\nBody of {name}.\n",
        encoding="utf-8",
        newline="\n",
    )


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """A `plan` folder of three tasks and a `work` folder copies land in."""
    task_file(tmp_path / "plan", "a", "metadata:\n  orchestrator.team: followup\n")
    task_file(tmp_path / "plan", "b", "depends_on: [a]\n")
    task_file(tmp_path / "plan", "c", status="queued")
    # `work` names no word for `queued`, so a task at it cannot be copied there.
    narrow = {key: value for key, value in MAPPING.items() if key != "queued"}
    (tmp_path / "work").mkdir()
    sources = {
        "plan": {
            "plugin": "local-md",
            "config": {"root": str(tmp_path / "plan"), "status_mapping": MAPPING},
        },
        "work": {
            "plugin": "local-md",
            "config": {"root": str(tmp_path / "work"), "status_mapping": narrow},
        },
    }
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps({"page_size": 1, "sources": sources}), encoding="utf-8"
    )
    return tmp_path


@pytest.fixture
def client(binary: Path, store: Path) -> Client:
    """A client reading no configuration but the store's."""
    environment = {**os.environ, "HOME": str(store), "XDG_CONFIG_HOME": str(store)}
    return Client(binary, cwd=store, environment=environment)


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one client call to its answer."""
    return asyncio.run(call)


def version_of(client: Client, task: str) -> str:
    """The task's version, as `task show` reports it."""
    [shown] = run(client.task_show(task)).items
    # Read as the exported models a caller holds a task by.
    qualified = QualifiedTask.model_validate(shown.model_dump(mode="json"))
    task_model = Task.model_validate(qualified.item.model_dump(mode="json"))
    assert task_model.model_dump(mode="json") == qualified.item.model_dump(mode="json")
    assert qualified.item.version is not None
    return qualified.item.version


def file_of(store: Path, task: str) -> bytes:
    """The task's file, byte for byte: what a refused write must leave as it was."""
    folder, name = task.split(":")
    return (store / folder / "tasks" / f"{name}.md").read_bytes()


def body(store: Path, text: str) -> str:
    """A file holding `text`, by its path."""
    path = store / f"body-{len(text)}.md"
    path.write_text(text, encoding="utf-8", newline="\n")
    return str(path)


def test_the_item_models_are_exported_from_the_package_root() -> None:
    """A caller names every item model from the root, never from a generated module."""
    for model in (
        Task,
        QualifiedTask,
        Comment,
        CommentActivity,
        CommentsSince,
        Located,
        CopyOutcome,
        CopyAction,
        CopyLink,
        CopyVia,
        CopyReport,
    ):
        assert model.__module__.startswith("onetaskgraph_sdk._generated"), model
    assert issubclass(VersionMovedError, OnetaskgraphError)


def test_every_held_write_lands_at_the_version_and_is_refused_typed_at_a_stale_one(
    client: Client, store: Path
) -> None:
    """Each of the eight held writes, at the current version and then at one the task left."""
    added = run(client.task_comment_add("plan:a", body="The comment an edit edits.\n"))
    comment: str = added.model_dump(mode="json")["id"]
    run(client.task_copy(["plan:a"], to="work"))
    [landed] = run(client.task_list(source=["work"], all=True)).items
    counterpart = landed.id.root

    writes: list[tuple[str, Callable[[str, int], Coroutine[object, object, object]]]] = [
        (
            "plan:a",
            lambda version, n: client.task_update("plan:a", title=f"Round {n}", if_version=version),
        ),
        (
            "plan:a",
            lambda version, n: client.task_priority_set(
                "plan:a", ["low", "urgent"][n % 2], if_version=version
            ),
        ),
        (
            "plan:a",
            lambda version, n: client.task_content_set(
                "plan:a", body(store, f"Content {n}.\n"), if_version=version
            ),
        ),
        (
            "plan:a",
            lambda version, n: client.task_metadata_set(
                "plan:a", "followup.round", str(n), if_version=version
            ),
        ),
        (
            "plan:a",
            lambda version, n: client.task_status_set(
                "plan:a", ["backlog", "in-progress"][n % 2], if_version=version
            ),
        ),
        (
            "plan:a",
            lambda version, n: client.task_comment_add(
                "plan:a", body=f"Follow-up {n}.\n", if_version=version
            ),
        ),
        (
            "plan:a",
            lambda version, n: client.task_comment_edit(
                "plan:a", comment, body=f"Edited {n}.\n", if_version=version
            ),
        ),
        # A copy is held to the version of the destination task it writes over: `plan:a`'s
        # counterpart, which every write above has given something new to carry.
        (
            counterpart,
            lambda version, n: client.task_copy(["plan:a"], to="work", if_version=version),
        ),
    ]
    for round_, (task, write) in enumerate(writes):
        read = version_of(client, task)
        run(write(read, 2 * round_))
        written = version_of(client, task)
        assert written != read, round_

        # Another writer moves the task after this caller's read.
        run(client.task_update(task, title=f"Moved before write {round_}"))
        moved = version_of(client, task)
        before = file_of(store, task)
        with pytest.raises(VersionMovedError) as refused:
            run(write(written, 2 * round_ + 1))
        error = refused.value
        assert error.kind == "version-moved", round_
        assert error.current_version == moved, round_
        assert error.document is not None
        assert error.document.failure.kind == "version-moved"
        assert isinstance(error.answer, dict)
        assert error.answer["held"]["version"] == moved
        assert file_of(store, task) == before, f"write {round_} left the task as it was"


def test_a_partial_copy_answers_its_typed_result_with_every_item(client: Client) -> None:
    """One task the destination cannot hold fails; the result keeps every item's answer."""
    report = run(client.task_copy(["plan:a", "plan:c"], to="work"))
    assert isinstance(report, CopyReport)
    by_source = {outcome.root.source.root: outcome.root for outcome in report.items}
    assert set(by_source) == {"plan:a", "plan:c"}
    assert by_source["plan:a"].action == "created"
    assert by_source["plan:c"].action == "failed"


def test_a_guarded_priority_set_answers_held_and_a_repositories_update_replaces_alone(
    client: Client,
) -> None:
    """The guard, the repositories list and its clear, through the SDK."""
    held = run(client.task_priority_set("plan:a", "low", when_priority=["none"]))
    assert held.outcome == "held"
    assert held.priority == "high"
    assert held.priority_condition is not None
    assert held.priority_condition.met is False
    written = run(client.task_priority_set("plan:a", "low", when_priority=["high"]))
    assert written.outcome == "written"
    assert written.priority == "low"

    updated = run(client.task_update("plan:a", repository=["github.com/acme/widgets"]))
    assert [repository.root for repository in updated.task.repositories] == [
        "github.com/acme/widgets"
    ]
    assert [field.value for field in updated.written] == ["repositories"]
    cleared = run(client.task_update("plan:a", no_repositories=True))
    assert cleared.task.repositories == []
    with pytest.raises(VersionMovedError):
        run(client.task_update("plan:a", no_repositories=True, if_version="md1|stale"))


def test_comments_since_all_pages_and_a_show_with_dependencies(client: Client) -> None:
    """The listing and its watermark, every page of a listing, and a task with its edges."""
    added = run(client.task_comment_add("plan:a", body="Since.\n", author="ci-helper[bot]"))
    listed = run(client.task_comments("plan", "2020-01-01T00:00:00Z"))
    assert isinstance(listed, CommentsSince)
    assert listed.schema_version == 1
    [listed_activity] = listed.comments
    activity = CommentActivity.model_validate(listed_activity.model_dump(mode="json"))
    assert activity.model_dump(mode="json")["id"] == added.model_dump(mode="json")["id"]
    assert activity.author_kind == "bot"
    assert listed.watermark is not None
    again = run(client.task_comments("plan", listed.watermark.isoformat()))
    for comment in again.comments:
        assert comment.version == activity.version

    every = run(client.task_list(source=["plan"], all=True))
    assert sorted(task.id.root for task in every.items) == ["plan:a", "plan:b", "plan:c"]
    assert every.next is None

    shown = run(client.task_show("plan:b", with_deps=True))
    deps = run(client.task_deps("plan:b"))
    assert shown.depends_on is not None
    assert [edge.model_dump(mode="json") for edge in shown.depends_on] == [
        edge.model_dump(mode="json") for edge in deps.items
    ]
