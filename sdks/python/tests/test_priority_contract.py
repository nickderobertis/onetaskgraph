"""The priority members the contract added, as the generated models decode them.

A task's `priority` and a source's `priority` and `filter_by_priority` capabilities are
defaulted when a document written before them omits them — that is how a pre-change engine or
plugin is read — and never nullable: the binary always writes them, and an explicit `null` is
a document no engine emits, so a model that accepted one would read a malformed answer as a
priority of its own choosing.
"""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

import pytest
from pydantic import BaseModel, ValidationError

from onetaskgraph_sdk import Client
from onetaskgraph_sdk._generated import (
    query_response_of_qualified_task,
    query_response_of_search_hit,
    source_listing,
    task_detail,
)

# Every generated model a task is decoded through, since each root carries its own copy.
TASK_MODELS: list[type[BaseModel]] = [
    query_response_of_qualified_task.Task,
    query_response_of_search_hit.Task,
    task_detail.Task,
]

# A task exactly as an engine or plugin wrote one before `priority` existed.
PRE_CHANGE_TASK: dict[str, Any] = {
    "id": "tasks/migrate.md",
    "key": None,
    "title": "Migrate the store",
    "content": "Move the rows.",
    "status": {"category": "todo", "name": "Todo"},
    "labels": [],
    "project": None,
    "url": None,
    "location": None,
    "created_at": None,
    "updated_at": None,
    "metadata": {"team.estimate": 3},
    "repositories": [],
}

# A capability declaration exactly as a plugin's handshake carried one before priorities.
PRE_CHANGE_CAPABILITIES: dict[str, Any] = {
    "projects": "native",
    "documents": "unsupported",
    "comments": "unsupported",
    "orphan_tasks": "native",
    "filter_by_label": "unsupported",
    "filter_by_status": "native",
    "search_title": "native",
    "search_content": "unsupported",
    "task_dependencies": "forward-only",
    "project_dependencies": "both-directions",
    "max_page_size": 25,
}

PRIORITY_MEMBERS = ("priority", "filter_by_priority")


@pytest.mark.parametrize("model", TASK_MODELS)
def test_a_pre_change_task_reads_its_omitted_priority_as_none(model: type[BaseModel]) -> None:
    """A task written before `priority` existed reads it as `none`, in every task model."""
    task = model.model_validate(PRE_CHANGE_TASK)
    assert task.model_dump()["priority"] == "none"


@pytest.mark.parametrize("model", TASK_MODELS)
def test_a_task_with_an_explicit_null_priority_is_refused(model: type[BaseModel]) -> None:
    """An explicit `null` is no priority the binary writes, so it is refused, naming the field."""
    with pytest.raises(ValidationError) as refused:
        model.model_validate({**PRE_CHANGE_TASK, "priority": None})
    assert refused.value.errors()[0]["loc"] == ("priority",)


@pytest.mark.parametrize("model", TASK_MODELS)
@pytest.mark.parametrize("priority", ["none", "urgent", "high", "medium", "low"])
def test_a_tasks_priority_round_trips(model: type[BaseModel], priority: str) -> None:
    """Every priority is written back out and read in again as itself."""
    task = model.model_validate({**PRE_CHANGE_TASK, "priority": priority})
    dumped = task.model_dump(mode="json")
    assert dumped["priority"] == priority
    assert model.model_validate(dumped) == task


def test_a_pre_change_handshake_reads_both_priority_capabilities_as_unsupported() -> None:
    """A handshake written before priorities declares neither, which reads as unsupported."""
    capabilities = source_listing.Capabilities.model_validate(PRE_CHANGE_CAPABILITIES)
    for member in PRIORITY_MEMBERS:
        assert getattr(capabilities, member) == "unsupported", member


@pytest.mark.parametrize("member", PRIORITY_MEMBERS)
def test_a_capability_with_an_explicit_null_priority_member_is_refused(member: str) -> None:
    """An explicit `null` for either member is refused, naming it."""
    with pytest.raises(ValidationError) as refused:
        source_listing.Capabilities.model_validate({**PRE_CHANGE_CAPABILITIES, member: None})
    assert refused.value.errors()[0]["loc"] == (member,)


@pytest.mark.parametrize("member", PRIORITY_MEMBERS)
@pytest.mark.parametrize("support", ["native", "unsupported"])
def test_a_priority_capability_round_trips(member: str, support: str) -> None:
    """Both declarations are written back out and read in again as themselves."""
    capabilities = source_listing.Capabilities.model_validate(
        {**PRE_CHANGE_CAPABILITIES, member: support}
    )
    dumped = capabilities.model_dump(mode="json")
    assert dumped[member] == support
    assert source_listing.Capabilities.model_validate(dumped) == capabilities


def test_the_real_binary_writes_both_members_and_they_decode(binary: Path, tmp_path: Path) -> None:
    """What the binary emits for a task and a source decodes through the same models."""
    tasks = tmp_path / "work" / "tasks"
    tasks.mkdir(parents=True)
    (tasks / "T-1.md").write_text("---\ntitle: Unranked\nstatus: todo\n---\nBody.\n")
    (tmp_path / "onetaskgraph.yaml").write_text(
        '{"sources": {"work": {"plugin": "local-md", "config": {"root": "work"}}}}'
    )
    client = Client(binary, cwd=tmp_path)

    shown = asyncio.run(client.task_show(id="work:T-1")).items[0].item
    assert shown.priority == "none"
    listed = asyncio.run(client.sources_list())
    available = listed[0].root
    assert isinstance(available, source_listing.SourceListingAvailable)
    capabilities = available.capabilities
    assert (capabilities.priority, capabilities.filter_by_priority) == ("native", "native")
