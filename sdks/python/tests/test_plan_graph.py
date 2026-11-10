"""A plan graph created from a project template's `members`, through the SDK.

Every call drives the real binary over a real folder of Markdown, and every answer validates
against the model generated from the root the binary emits for it: `PlannedProject` for a dry
run, `QueryResponseOfQualifiedProject` for a create, and `MemberDeclaration` for a member as a
project template declares it.
"""

from __future__ import annotations

import asyncio
import json
from collections.abc import Coroutine
from pathlib import Path

from onetaskgraph_sdk import (
    Client,
    MemberDeclaration,
    PlannedProject,
    QueryResponseOfQualifiedProject,
)

PLAN = """---
onetaskgraph_template: 1
variables:
  goal:
    description: What the plan is for
members:
  - id: design
    title: "Design {{ goal }}"
    template: {file: task.md}
    answers: {topic: "{{ goal }}"}
  - id: ship
    title: "Ship {{ goal }}"
    template: {file: task.md}
    answers: {topic: "shipping {{ goal }}"}
    depends_on: [design]
---
The plan for {{ goal }}.
"""

TASK = """---
onetaskgraph_template: 1
variables:
  topic:
    description: What the task covers
---
Topic: {{ topic }}
"""


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one public async SDK call to completion."""
    return asyncio.run(call)


def test_a_plan_graph_is_planned_then_created_as_its_models_describe(
    binary: Path, tmp_path: Path
) -> None:
    """A dry run answers `PlannedProject` and writes nothing; the create lands every member."""
    (tmp_path / "notes").mkdir()
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps({"sources": {"notes": {"plugin": "local-md", "config": {"root": "notes"}}}}),
        encoding="utf-8",
    )
    (tmp_path / "plan.md").write_text(PLAN, encoding="utf-8", newline="\n")
    (tmp_path / "task.md").write_text(TASK, encoding="utf-8", newline="\n")
    client = Client(binary, cwd=tmp_path)

    planned = run(
        client.project_create(
            "notes", "plan-1", "Plan", template="plan.md", answers={"goal": "w"}, dry_run=True
        )
    )
    assert isinstance(planned, PlannedProject)
    assert [(member.id.root, member.content) for member in planned.members] == [
        ("design", "Topic: w\n"),
        ("ship", "Topic: shipping w\n"),
    ]
    assert [entry.root for entry in planned.members[1].depends_on] == ["design"]
    assert list((tmp_path / "notes").iterdir()) == [], "a dry run writes nothing"

    created = run(
        client.project_create("notes", "plan-1", "Plan", template="plan.md", answers={"goal": "w"})
    )
    assert isinstance(created, QueryResponseOfQualifiedProject)
    assert created.items[0].item.content == "The plan for w.\n"
    tasks = run(client.task_list(project="notes:plan-1"))
    assert sorted(task.id.root for task in tasks.items) == ["notes:design", "notes:ship"]
    edges = run(client.task_deps("notes:ship"))
    assert [edge.to.id.root for edge in edges.items] == ["notes:design"]

    declared = MemberDeclaration.model_validate(
        {"id": "a", "title": "A", "template": {"loader": "a.json"}, "depends_on": ["b"]}
    )
    assert declared.depends_on == ["b"]
