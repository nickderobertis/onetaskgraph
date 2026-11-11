"""The batched calls, against the real binary over two folders of Markdown.

`task_find`, `task_update_many` and `task_comment_add_many` take their request document typed —
or as a mapping validated to it — write it to the binary's standard input, and answer typed per
item; `task_copy` holds each counterpart to its own version; and `task_update` writes only while
its priority guard holds. Every answer is the store's: an item another writer moved comes back
`version-moved` with the version it is at, a guard that declined `held`, a missing task `error`
naming it — each beside the items that were written, never instead of them.
"""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path

import pytest

from onetaskgraph_sdk import (
    BatchOutcome,
    Client,
    CommentBatch,
    CommentsAdded,
    FindRequest,
    FoundTasks,
    OnetaskgraphError,
    TasksUpdated,
    UpdateBatch,
)
from onetaskgraph_sdk._generated.copy_report import (
    CopyOutcomeCreatedOrWouldCreate,
    CopyOutcomeFailed,
    CopyOutcomeUpdated,
)

TASKS = {
    "a": ("Alpha batch", "todo", "high", "red"),
    "b": ("Bravo batch", "todo", "high", "red"),
    "c": ("Charlie batch", "in-progress", "low", "blue"),
    "d": ("Delta", "todo", "none", "red"),
    "e": ("Echo batch", "done", "none", "green"),
}
"""Each task of `plan`: its title, status, priority and `myapp.colour`."""


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """A folder `plan` holding five tasks and an empty folder `snap` to copy into."""
    tasks = tmp_path / "plan" / "tasks"
    tasks.mkdir(parents=True)
    for name, (title, status, priority, colour) in TASKS.items():
        (tasks / f"{name}.md").write_text(
            f"---\ntitle: {title}\nstatus: {status}\npriority: {priority}\n"
            f"metadata:\n  myapp.colour: {colour}\n---\nThe {name} body.\n",
            encoding="utf-8",
            newline="\n",
        )
    (tmp_path / "snap").mkdir()
    sources = {
        folder: {"plugin": "local-md", "config": {"root": folder}} for folder in ("plan", "snap")
    }
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    return tmp_path


def client_at(binary: Path, store: Path) -> Client:
    """A client over the binary in `store`, its configuration that folder's alone."""
    environment = {**os.environ, "HOME": str(store), "XDG_CONFIG_HOME": str(store)}
    return Client(binary, cwd=store, environment=environment)


def version_of(client: Client, task: str) -> str:
    """The version a read of `task` reports."""
    item = asyncio.run(client.task_show(task)).items[0].item
    assert item.version is not None, f"{task} reports no version"
    return item.version


def test_many_searches_answer_each_key_with_every_match_across_pages(
    binary: Path, store: Path
) -> None:
    """A typed request and a mapping both reach the binary; each key holds exactly its matches."""
    client = client_at(binary, store)
    request = FindRequest.model_validate(
        {
            "schema_version": 1,
            "queries": [
                {"key": "text", "text": "batch", "in": "title"},
                {"key": "colour", "metadata": {"myapp.colour": "red"}},
                {"key": "open", "text": "batch", "status": ["todo", "in-progress"]},
                {"key": "nothing", "text": "no task holds this"},
            ],
        }
    )
    found = asyncio.run(client.task_find("plan", request, page_size=1))
    assert isinstance(found, FoundTasks)
    answers = {answer.key: answer for answer in found.answers}
    assert [answer.key for answer in found.answers] == ["text", "colour", "open", "nothing"]
    ids = {key: sorted(item.id.root for item in answer.items) for key, answer in answers.items()}
    assert ids == {
        "text": ["plan:a", "plan:b", "plan:c", "plan:e"],
        "colour": ["plan:a", "plan:b", "plan:d"],
        "open": ["plan:a", "plan:b", "plan:c"],
        "nothing": [],
    }
    assert all(answer.complete and answer.reason is None for answer in found.answers)

    as_mapping = asyncio.run(
        client.task_find(
            "plan", {"schema_version": 1, "queries": [{"key": "one", "text": "Delta"}]}
        )
    )
    assert [item.id.root for item in as_mapping.answers[0].items] == ["plan:d"]

    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(
            client.task_find("plan", {"schema_version": 1, "queries": [{"key": "everything"}]})
        )
    assert refused.value.exit_code == 1
    assert "everything" in str(refused.value)
    assert refused.value.document is not None
    assert refused.value.document.failure.kind == "unnarrowed-query"

    with pytest.raises(TypeError, match="not a FindRequest"):
        asyncio.run(client.task_find("plan", {"schema_version": 1, "queries": [{"kye": "x"}]}))


def test_a_copy_holds_each_counterpart_to_its_own_version(binary: Path, store: Path) -> None:
    """A counterpart another writer moved is `version-moved` with its version; others copy."""
    client = client_at(binary, store)
    ids = tuple(f"plan:{name}" for name in "abcd")
    first = asyncio.run(client.task_copy(ids, to="snap"))
    counterparts: dict[str, str] = {}
    for outcome in (item.root for item in first.items):
        assert isinstance(outcome, CopyOutcomeCreatedOrWouldCreate)
        assert outcome.destination is not None
        counterparts[outcome.source.root] = outcome.destination.root
    read = {source: version_of(client, copy) for source, copy in counterparts.items()}
    # Another writer moves `b`'s counterpart after it was read.
    asyncio.run(client.task_update(counterparts["plan:b"], title="Bravo, edited at snap"))
    moved_to = version_of(client, counterparts["plan:b"])
    assert moved_to != read["plan:b"]
    for name in "abcd":
        asyncio.run(client.task_update(f"plan:{name}", title=f"{TASKS[name][0]}, revised"))

    second = asyncio.run(
        client.task_copy(
            ids,
            to="snap",
            if_version=[f"plan:a={read['plan:a']}", f"plan:b={read['plan:b']}"],
        )
    )
    outcomes = {item.root.source.root: item.root for item in second.items}
    for written in ("plan:a", "plan:c", "plan:d"):
        assert isinstance(outcomes[written], CopyOutcomeUpdated), outcomes[written]
        title = asyncio.run(client.task_show(counterparts[written])).items[0].item.title
        assert title == f"{TASKS[written.removeprefix('plan:')][0]}, revised"
    refused = outcomes["plan:b"]
    assert isinstance(refused, CopyOutcomeFailed)
    assert refused.error.kind == "version-moved"
    assert refused.held is not None and refused.held.version == moved_to
    left = asyncio.run(client.task_show(counterparts["plan:b"])).items[0].item
    assert left.title == "Bravo, edited at snap"


def test_many_comments_are_each_answered_on_their_own(binary: Path, store: Path) -> None:
    """Written comments carry their id and version; a moved and a missing task do not stop them."""
    client = client_at(binary, store)
    stale = version_of(client, "plan:b")
    asyncio.run(client.task_update("plan:b", title="Bravo, moved"))
    batch = CommentBatch.model_validate(
        {
            "schema_version": 1,
            "comments": [
                {"item": "plan:a", "body": "first"},
                {"item": "plan:b", "body": "held", "if_version": stale},
                {"item": "plan:c", "body": "third", "if_version": version_of(client, "plan:c")},
                {"item": "plan:nowhere", "body": "lost"},
            ],
        }
    )
    added = asyncio.run(client.task_comment_add_many("plan", batch))
    assert isinstance(added, CommentsAdded)
    outcomes = [result.outcome for result in added.results]
    assert outcomes == [
        BatchOutcome.BatchOutcomeWritten,
        BatchOutcome.BatchOutcomeVersionMoved,
        BatchOutcome.BatchOutcomeWritten,
        BatchOutcome.BatchOutcomeError,
    ]
    first, moved, third, missing = added.results
    assert moved.current_version == version_of(client, "plan:b")
    assert missing.reason is not None and "plan:nowhere" in missing.reason
    for result, task, body in ((first, "plan:a", "first"), (third, "plan:c", "third")):
        assert result.comment is not None
        listed = asyncio.run(client.task_comment_list(task)).comments
        assert [(comment.id.root, comment.body) for comment in listed] == [
            (result.comment.id.root, body)
        ]
    assert asyncio.run(client.task_comment_list("plan:b")).comments == []


def test_many_updates_write_every_field_or_none(binary: Path, store: Path) -> None:
    """Each update's content, metadata, status and priority land together, or none of them do."""
    client = client_at(binary, store)
    stale = version_of(client, "plan:b")
    asyncio.run(client.task_update("plan:b", title="Bravo, moved"))
    moved_to = version_of(client, "plan:b")

    def every_field(item: str, **extra: object) -> dict[str, object]:
        return {
            "item": item,
            "body": "Rewritten.\n",
            "metadata": {"myapp.colour": "black"},
            "status": "done",
            "priority": "low",
            "when_priority": ["high", "none"],
            **extra,
        }

    batch = UpdateBatch.model_validate(
        {
            "schema_version": 1,
            "updates": [
                every_field("plan:a", if_version=version_of(client, "plan:a")),
                every_field("plan:b", if_version=stale),
                every_field("plan:c"),
                every_field("plan:nowhere"),
            ],
        }
    )
    updated = asyncio.run(client.task_update_many("plan", batch))
    assert isinstance(updated, TasksUpdated)
    assert [result.outcome.value for result in updated.results] == [
        "written",
        "version-moved",
        "held",
        "error",
    ]
    written, moved, held, missing = updated.results
    assert written.version == version_of(client, "plan:a")
    assert moved.current_version == moved_to
    assert missing.reason is not None and "plan:nowhere" in missing.reason

    def fields(task: str) -> tuple[str | None, object, str, str]:
        item = asyncio.run(client.task_show(task)).items[0].item
        colour = item.metadata.get("myapp.colour") if item.metadata else None
        return item.content, colour, item.status.category.value, item.priority.value

    assert fields("plan:a") == ("Rewritten.\n", "black", "done", "low")
    assert fields("plan:b") == ("The b body.", "red", "todo", "high")
    assert fields("plan:c") == ("The c body.", "blue", "in-progress", "low")


def test_a_guarded_update_writes_every_field_only_while_its_guard_holds(
    binary: Path, store: Path
) -> None:
    """`when_priority` decides the whole update, and the answer's `guard` says which way."""
    client = client_at(binary, store)
    body = store / "body.md"
    body.write_text("Guarded.\n", encoding="utf-8", newline="\n")

    held = asyncio.run(
        client.task_update(
            "plan:c",
            body_file=str(body),
            metadata=['myapp.colour="black"'],
            priority="high",
            when_priority=["high"],
        )
    )
    assert held.guard is not None and held.guard.outcome is not None
    assert held.guard.outcome.value == "held"
    assert held.guard.priority_condition is not None
    assert held.guard.priority_condition.held.value == "low"
    assert held.written == []
    unchanged = asyncio.run(client.task_show("plan:c")).items[0].item
    assert (unchanged.content, unchanged.priority.value) == ("The c body.", "low")

    written = asyncio.run(
        client.task_update(
            "plan:a",
            body_file=str(body),
            metadata=['myapp.colour="black"'],
            priority="low",
            when_priority=["high"],
        )
    )
    assert written.guard is not None and written.guard.outcome is not None
    assert written.guard.outcome.value == "written"
    assert sorted(field.value for field in written.written) == ["content", "metadata", "priority"]
    item = asyncio.run(client.task_show("plan:a")).items[0].item
    assert item.content == "Guarded.\n"
    assert item.priority.value == "low"
    assert item.metadata is not None and item.metadata["myapp.colour"] == "black"
