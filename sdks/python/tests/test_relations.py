"""The relation calls, against the real binary over two folders of Markdown.

`task_parent_set_many`, `task_link_add_many` and `task_link_remove_many` take their batch typed —
or as a mapping validated to it — write it to the binary's standard input, and answer typed per
item; `task_relations` reads each task's parent, children and links. Every answer is the store's:
a cycle and a parent in another source come back `refused` beside the edges written, a repeated
add `exists`, a second removal `absent`, and a link's `other_end` says whether the other item,
read in the same call, holds it too.
"""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path

import pytest

from onetaskgraph_sdk import (
    Client,
    LinkBatch,
    LinkOutcome,
    LinksWritten,
    OnetaskgraphError,
    OtherEnd,
    ParentBatch,
    ParentOutcome,
    ParentsSet,
    TaskRelations,
)


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """Folders `work` (three tasks) and `other` (one task)."""
    for folder, names in (("work", ("t1", "t2", "t3")), ("other", ("o1",))):
        tasks = tmp_path / folder / "tasks"
        tasks.mkdir(parents=True)
        for name in names:
            (tasks / f"{name}.md").write_text(
                f"---\ntitle: Task {name}\nstatus: todo\n---\nThe {name} body.\n",
                encoding="utf-8",
                newline="\n",
            )
    sources = {
        folder: {"plugin": "local-md", "config": {"root": folder}} for folder in ("work", "other")
    }
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    return tmp_path


def client_at(binary: Path, store: Path) -> Client:
    """A client over the binary in `store`, its configuration that folder's alone."""
    environment = {**os.environ, "HOME": str(store), "XDG_CONFIG_HOME": str(store)}
    return Client(binary, cwd=store, environment=environment)


def test_parents_are_written_per_edge_and_read_back_from_either_end(
    binary: Path, store: Path
) -> None:
    """A chain is written beside a refused cycle and a refused cross-source parent."""
    client = client_at(binary, store)
    batch = ParentBatch.model_validate(
        {
            "schema_version": 1,
            "edges": [
                {"child": "work:t2", "parent": "work:t1"},
                {"child": "work:t3", "parent": "work:t2"},
            ],
        }
    )
    written = asyncio.run(client.task_parent_set_many(batch))
    assert isinstance(written, ParentsSet)
    assert [result.outcome for result in written.results] == [
        ParentOutcome.ParentOutcomeWritten,
        ParentOutcome.ParentOutcomeWritten,
    ]
    assert all(result.version for result in written.results)

    mixed = asyncio.run(
        client.task_parent_set_many(
            {
                "schema_version": 1,
                "edges": [
                    {"child": "work:t1", "parent": "work:t3"},
                    {"child": "work:t2", "parent": "work:t1"},
                    {"child": "work:t3", "parent": "other:o1"},
                ],
            }
        )
    )
    assert [result.outcome for result in mixed.results] == [
        ParentOutcome.ParentOutcomeRefused,
        ParentOutcome.ParentOutcomeUnchanged,
        ParentOutcome.ParentOutcomeRefused,
    ]
    assert "cycle" in (mixed.results[0].reason or "")
    assert "other:o1" in (mixed.results[2].reason or "")

    with pytest.raises(OnetaskgraphError) as unqualified:
        asyncio.run(client.task_relations(["t1"]))
    assert unqualified.value.exit_code != 0

    relations = asyncio.run(client.task_relations(["work:t1", "work:t2", "work:t3"]))
    assert isinstance(relations, TaskRelations)
    by_item = {item.item.root: item for item in relations.items}
    assert by_item["work:t1"].parent is None
    assert [child.root for child in by_item["work:t1"].children] == ["work:t2"]
    assert by_item["work:t2"].parent is not None and by_item["work:t2"].parent.root == "work:t1"
    assert [child.root for child in by_item["work:t2"].children] == ["work:t3"]
    assert by_item["work:t3"].parent is not None and by_item["work:t3"].parent.root == "work:t2"
    assert all(item.children_complete for item in relations.items)


def test_links_are_added_read_from_both_ends_and_removed(binary: Path, store: Path) -> None:
    """Each link is answered on its own: written, exists, absent; both ends read it typed."""
    client = client_at(binary, store)
    batch = LinkBatch.model_validate(
        {
            "schema_version": 1,
            "links": [
                {"from": "work:t2", "to": "work:t1", "type": "caused-by"},
                {"from": "work:t3", "to": "other:o1", "type": "relates"},
            ],
        }
    )
    added = asyncio.run(client.task_link_add_many(batch))
    assert isinstance(added, LinksWritten)
    assert [result.outcome for result in added.results] == [
        LinkOutcome.LinkOutcomeWritten,
        LinkOutcome.LinkOutcomeWritten,
    ]
    assert added.results[0].from_version and added.results[0].to_version

    again = asyncio.run(
        client.task_link_add_many(
            {
                "schema_version": 1,
                "links": [{"from": "work:t2", "to": "work:t1", "type": "caused-by"}],
            }
        )
    )
    assert [result.outcome for result in again.results] == [LinkOutcome.LinkOutcomeExists]

    relations = asyncio.run(client.task_relations(["work:t1", "work:t2", "other:o1"]))
    links = {
        item.item.root: [
            (link.type, link.direction.value, link.other.root, link.other_end)
            for link in item.links
        ]
        for item in relations.items
    }
    assert links == {
        "work:t1": [("caused-by", "in", "work:t2", OtherEnd.OtherEndPresent)],
        "work:t2": [("caused-by", "out", "work:t1", OtherEnd.OtherEndPresent)],
        "other:o1": [("relates", "in", "work:t3", OtherEnd.OtherEndUnread)],
    }
    shown = asyncio.run(client.task_show("work:t2")).items[0].item
    assert shown.content == "The t2 body."

    unlink = {
        "schema_version": 1,
        "links": [{"from": "work:t2", "to": "work:t1", "type": "caused-by"}],
    }
    removed = asyncio.run(client.task_link_remove_many(unlink))
    assert [result.outcome for result in removed.results] == [LinkOutcome.LinkOutcomeWritten]
    gone = asyncio.run(client.task_link_remove_many(unlink))
    assert [result.outcome for result in gone.results] == [LinkOutcome.LinkOutcomeAbsent]
    after = asyncio.run(client.task_relations(["work:t1", "work:t2"]))
    assert all(not item.links for item in after.items)

    with pytest.raises(TypeError, match="not a LinkBatch"):
        asyncio.run(client.task_link_add_many({"schema_version": 1, "links": [{"form": "x"}]}))
