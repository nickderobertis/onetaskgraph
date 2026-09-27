"""Tasks and documents created from templates, regenerated and read back, through the SDK.

Every call drives the real binary over a real folder of Markdown, which keeps the answers an
item was rendered from in its own file. Prompting is turned on in the environment each client
is handed, so a call that could prompt would be refused for having no terminal: the SDK's own
`--no-interactive` is what these calls rest on.
"""

from __future__ import annotations

import asyncio
import hashlib
import json
import os
from collections.abc import Coroutine
from pathlib import Path

import pytest

from onetaskgraph_sdk import (
    Client,
    OnetaskgraphError,
    QueryResponseOfQualifiedDocument,
    Regenerated,
    TaskDetail,
    TemplateAnswers,
    TemplateProvenance,
    TemplateVariables,
)

TASK = """---
onetaskgraph_template: 1
variables:
  goal:
    description: What the task is for
  steps:
    description: How it is done
    type: list
    default: []
---
Goal: {{ goal }}
{% for step in steps %}
- {{ step }}
{% endfor %}
"""


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one public async SDK call to completion."""
    return asyncio.run(call)


def sha256(text: str) -> str:
    """The digest a provenance entry spells: `sha256:` and the lowercase hex SHA-256."""
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()


def plan(binary: Path, tmp_path: Path) -> tuple[Client, Path]:
    """A client over a folder of Markdown called `notes`, prompting turned on, and a template."""
    (tmp_path / "notes").mkdir()
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps({"sources": {"notes": {"plugin": "local-md", "config": {"root": "notes"}}}}),
        encoding="utf-8",
    )
    template = tmp_path / "task.md"
    template.write_text(TASK, encoding="utf-8", newline="\n")
    environment = dict(os.environ)
    environment["ONETASKGRAPH_INTERACTIVE"] = "true"
    return Client(binary, cwd=tmp_path, environment=environment), template


def provenance(detail: TaskDetail) -> TemplateProvenance:
    """The `onetaskgraph.template` entry of the one task a detail carries."""
    metadata = detail.items[0].item.metadata or {}
    return TemplateProvenance.model_validate(metadata["onetaskgraph.template"])


def test_a_task_is_created_regenerated_and_its_answers_read_without_a_prompt(
    binary: Path, tmp_path: Path
) -> None:
    """Answers go over standard input and come back typed; a render overlays the stored ones."""
    client, template = plan(binary, tmp_path)
    created = run(
        client.task_create(
            "notes",
            "P-1",
            "Ship it",
            template=str(template),
            answers={"goal": "Ship it", "steps": ["build"]},
            label=["release"],
            metadata=["myapp.estimate=3"],
        )
    )
    assert isinstance(created, TaskDetail)
    task = created.items[0]
    identifier = task.id.root
    assert task.item.content == "Goal: Ship it\n- build\n"
    assert (task.item.metadata or {})["myapp.estimate"] == 3
    recorded = provenance(created)
    assert recorded.template == str(template.resolve())
    assert recorded.body_digest.root == sha256(task.item.content)

    answers = run(client.task_answers(identifier))
    assert isinstance(answers, TemplateAnswers)
    assert answers.model_dump() == {"goal": "Ship it", "steps": ["build"]}
    assert recorded.answers_digest.root == sha256('{"goal":"Ship it","steps":["build"]}')

    regenerated = run(client.task_render(identifier, var=["goal=Ship it again"]))
    assert isinstance(regenerated, Regenerated)
    assert regenerated.changed
    assert regenerated.body == "Goal: Ship it again\n- build\n", "steps came from storage"
    assert regenerated.digest == recorded.digest.root
    again = run(client.task_render(identifier))
    assert not again.changed

    dry = run(client.task_render(identifier, answers={"goal": "Never"}, dry_run=True))
    assert dry.changed
    shown = run(client.task_show(identifier))
    assert shown.items[0].item.content == "Goal: Ship it again\n- build\n"


def test_a_plain_body_and_a_document_through_the_sdk(binary: Path, tmp_path: Path) -> None:
    """A body goes over standard input; a document is created, regenerated and answered."""
    client, template = plan(binary, tmp_path)
    plain = run(client.task_create("notes", "P-1", "By hand", body="Written by hand."))
    assert plain.items[0].item.content == "Written by hand."
    assert "onetaskgraph.template" not in (plain.items[0].item.metadata or {})
    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_answers(plain.items[0].id.root))
    assert refused.value.exit_code == 1
    assert "has no stored template answers" in str(refused.value)

    document = run(
        client.document_create(
            "notes", "P-1", "Design", id="design", template=str(template), var=["goal=Design"]
        )
    )
    assert isinstance(document, QueryResponseOfQualifiedDocument)
    assert document.items[0].id.root == "notes:design"
    assert run(client.document_answers("notes:design")).model_dump() == {
        "goal": "Design",
        "steps": [],
    }
    regenerated = run(client.document_render("notes:design", var=["goal=Design again"]))
    assert regenerated.body == "Goal: Design again\n"


def test_answers_out_of_step_refuse_a_partial_render_with_exit_two(
    binary: Path, tmp_path: Path
) -> None:
    """Answers edited by hand are not trusted, so every required one is asked of the caller."""
    client, template = plan(binary, tmp_path)
    created = run(
        client.task_create("notes", "P-1", "Edited", template=str(template), var=["goal=Ship"])
    )
    identifier = created.items[0].id.root
    file = tmp_path / "notes" / "tasks" / "edited.md"
    file.write_text(file.read_text(encoding="utf-8").replace("goal: Ship", "goal: Forged"))

    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_render(identifier, var=["steps=[x]"]))
    assert refused.value.exit_code == 2
    assert "supply every required answer" in str(refused.value)
    assert run(client.task_render(identifier, answers={"goal": "Ship"})).changed


def test_a_loader_document_names_the_template_and_body_with_answers_is_refused(
    binary: Path, tmp_path: Path
) -> None:
    """A loader document stands in for the file, and a body never rides beside answers."""
    client, template = plan(binary, tmp_path)
    loader = tmp_path / "loader.json"
    loader.write_text(
        json.dumps(
            {"reference": "caller:task", "entry": "task.md", "search_path": [str(tmp_path)]}
        ),
        encoding="utf-8",
    )
    described = run(client.template_variables(template_loader=str(loader)))
    assert isinstance(described, TemplateVariables)
    assert [variable.name for variable in described.variables] == ["goal", "steps"]
    created = run(
        client.task_create("notes", "P-1", "Loaded", template_loader=str(loader), var=["goal=x"])
    )
    assert provenance(created).template == "caller:task"

    with pytest.raises(TypeError, match="body is read only when no template"):
        run(client.task_create("notes", "P-1", "Both", template=str(template), body="x"))
    with pytest.raises(TypeError, match="body is read only when no template"):
        run(client.document_create("notes", "P-1", "Both", body_file="b.md", body="x"))
    with pytest.raises(TypeError, match="body is a int, not a string"):
        # Deliberately outside the declared type, as a caller whose values reached it untyped.
        run(client.task_create("notes", "P-1", "Typed", body=7))  # ty: ignore[invalid-argument-type]
    with pytest.raises(TypeError, match="body and answers both go to standard input"):
        run(
            client.task_create(
                "notes", "P-1", "Both", template=str(template), body="x", answers={"goal": "x"}
            )
        )
