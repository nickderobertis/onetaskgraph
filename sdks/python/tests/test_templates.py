"""Task templates through the SDK, driving the real binary."""

from __future__ import annotations

import asyncio
import os
from collections.abc import Coroutine
from pathlib import Path

import pytest

from onetaskgraph_sdk import (
    Client,
    ItemType,
    OnetaskgraphError,
    RenderedTemplate,
    TemplateVariables,
    VariableType,
)

TASK = """---
onetaskgraph_template: 1
variables:
  title:
    description: What the task is called
  steps:
    description: What to do
    type: list
  size:
    description: How big
    type: integer
    default: 1
---
{% extends "base.md" %}
{% block body %}
{{ title }} ({{ size }})
{% for step in steps %}
- {{ step }}
{% endfor %}
{% endblock %}
"""

BASE = """---
onetaskgraph_template: 1
variables:
  owner:
    description: Who owns it
    default: nobody
---
# Owned by {{ owner }}
{% block body %}{% endblock %}
"""


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one public async SDK call to completion."""
    return asyncio.run(call)


def template(tmp_path: Path) -> tuple[Path, Path]:
    """Write the task template and, in a directory of its own, the base it extends."""
    library = tmp_path / "library"
    library.mkdir()
    (library / "base.md").write_text(BASE, encoding="utf-8")
    task = tmp_path / "task.md"
    task.write_text(TASK, encoding="utf-8")
    return task, library


def interactive_client(binary: Path, tmp_path: Path) -> Client:
    """A client whose environment turns prompting on, which the SDK must override."""
    environment = dict(os.environ)
    environment["ONETASKGRAPH_INTERACTIVE"] = "true"
    return Client(binary, cwd=tmp_path, environment=environment)


def test_template_variables_reads_the_declared_set_down_the_chain(
    binary: Path, tmp_path: Path
) -> None:
    """Every declared variable comes back typed, with the chain's digest."""
    task, library = template(tmp_path)
    described = run(
        Client(binary, cwd=tmp_path).template_variables(str(task), search_path=[str(library)])
    )
    assert isinstance(described, TemplateVariables)
    assert described.template == "task.md"
    assert described.digest.startswith("sha256:")
    assert len(described.digest) == len("sha256:") + 64
    assert [
        (variable.name, variable.type, variable.declared_in) for variable in described.variables
    ] == [
        ("title", VariableType.VariableTypeString, "task.md"),
        ("steps", VariableType.VariableTypeList, "task.md"),
        ("size", VariableType.VariableTypeInteger, "task.md"),
        ("owner", VariableType.VariableTypeString, "base.md"),
    ]
    steps = described.variables[1]
    assert steps.items == ItemType.ItemTypeString
    assert steps.required
    assert described.variables[3].default == "nobody"


def test_template_render_hands_the_answers_over_on_stdin_and_never_prompts(
    binary: Path, tmp_path: Path
) -> None:
    """A mapping of answers renders, a --var outranks it, and a default fills the rest."""
    task, library = template(tmp_path)
    client = interactive_client(binary, tmp_path)
    rendered = run(
        client.template_render(
            str(task),
            search_path=[str(library)],
            answers={"title": "Ship it", "steps": ["build", "release"], "size": 3},
            var=["size=5"],
        )
    )
    assert isinstance(rendered, RenderedTemplate)
    assert rendered.body == "# Owned by nobody\nShip it (5)\n- build\n- release\n"
    assert rendered.answers == {
        "title": "Ship it",
        "steps": ["build", "release"],
        "size": 5,
        "owner": "nobody",
    }

    # Nothing answers `owner`, and prompting is on in the environment: a call that could
    # prompt would be refused for having no terminal. The SDK passes --no-interactive, so the
    # default is taken instead.
    described = run(client.template_variables(str(task), search_path=[str(library)]))
    assert rendered.digest == described.digest


def test_every_refused_answer_is_exit_two_naming_what_it_refuses(
    binary: Path, tmp_path: Path
) -> None:
    """Missing, mistyped and undeclared answers each raise with the binary's exit 2."""
    task, library = template(tmp_path)
    client = interactive_client(binary, tmp_path)

    with pytest.raises(OnetaskgraphError) as missing:
        run(client.template_render(str(task), search_path=[str(library)]))
    assert missing.value.exit_code == 2
    assert "required variables are unanswered: title, steps" in str(missing.value)

    with pytest.raises(OnetaskgraphError) as mistyped:
        run(
            client.template_render(
                str(task),
                search_path=[str(library)],
                answers={"title": "t", "steps": "not a list"},
            )
        )
    assert mistyped.value.exit_code == 2
    assert '"steps" is not a list' in str(mistyped.value)

    with pytest.raises(OnetaskgraphError) as undeclared:
        run(
            client.template_render(
                str(task),
                search_path=[str(library)],
                answers={"title": "t", "steps": [], "colour": "blue"},
            )
        )
    assert undeclared.value.exit_code == 2
    assert "colour" in str(undeclared.value)


def test_answers_json_cannot_carry_are_refused_before_the_binary_is_started(
    binary: Path, tmp_path: Path
) -> None:
    """A non-finite number, a non-string key and a value that is not JSON never reach it."""
    task, library = template(tmp_path)
    client = Client(binary, cwd=tmp_path)
    refused: list[object] = [
        {"title": "t", "steps": [], "size": float("nan")},
        {"title": "t", "steps": [], 1: "a key that is not a string"},
        {"title": "t", "steps": {"a", "set"}},
    ]
    for answers in refused:
        with pytest.raises(TypeError, match="answers are not a JSON mapping"):
            run(
                client.template_render(
                    str(task),
                    search_path=[str(library)],
                    # Deliberately outside the declared type: the runtime half of that check.
                    answers=answers,  # ty: ignore[invalid-argument-type]
                )
            )

    # An iterable of pairs is not a mapping, though `dict` would make one of it.
    with pytest.raises(TypeError, match="answers are a list, not a mapping"):
        run(
            client.template_render(
                str(task),
                search_path=[str(library)],
                answers=[("title", "t"), ("steps", [])],  # ty: ignore[invalid-argument-type]
            )
        )


def test_a_template_file_that_is_not_a_path_string_is_refused_before_the_binary_is_started(
    binary: Path, tmp_path: Path
) -> None:
    """A non-string, an empty path and one the binary would read as an option are refused."""
    _, library = template(tmp_path)
    client = Client(binary, cwd=tmp_path)
    # Deliberately outside the declared type, as a caller whose values reached it untyped.
    refused: list[object] = [7, None, tmp_path / "task.md", "", "--json", "-"]
    for file in refused:
        with pytest.raises(TypeError, match="template_variables: file is not a template path"):
            run(client.template_variables(file))  # ty: ignore[invalid-argument-type]
        with pytest.raises(TypeError, match="template_render: file is not a template path"):
            run(
                client.template_render(
                    file,  # ty: ignore[invalid-argument-type]
                    search_path=[str(library)],
                )
            )

    # The remedy the refusal names: the same file spelled from the directory it is in.
    (tmp_path / "-dashed.md").write_text("dashed\n", encoding="utf-8")
    assert run(client.template_render("./-dashed.md")).body == "dashed\n"


def test_a_search_path_or_var_that_is_not_strings_is_refused_before_the_binary_is_started(
    binary: Path, tmp_path: Path
) -> None:
    """Each entry becomes one process argument, so only a list or tuple of strings is sent."""
    task, library = template(tmp_path)
    client = Client(binary, cwd=tmp_path)
    # Deliberately outside the declared types, as a caller whose values reached it untyped.
    refusals: list[tuple[object, str]] = [
        (str(library), "search_path is a str, not a list"),
        ([str(library), 7], r"search_path\[1\] is a int, not a string"),
        ([library], r"search_path\[0\] is a \w*Path, not a string"),
    ]
    for search_path, message in refusals:
        with pytest.raises(TypeError, match=f"template_variables: ({message})"):
            run(
                client.template_variables(
                    str(task),
                    search_path=search_path,  # ty: ignore[invalid-argument-type]
                )
            )
        with pytest.raises(TypeError, match=f"template_render: ({message})"):
            run(
                client.template_render(
                    str(task),
                    search_path=search_path,  # ty: ignore[invalid-argument-type]
                )
            )
    for var, message in [
        ("title=t", "var is a str, not a list"),
        (["title=t", 5], r"var\[1\] is a int, not a string"),
    ]:
        with pytest.raises(TypeError, match=f"template_render: {message}"):
            run(
                client.template_render(
                    str(task),
                    search_path=[str(library)],
                    var=var,  # ty: ignore[invalid-argument-type]
                )
            )

    # A tuple of strings is a sequence the binary is handed as it is.
    rendered = run(
        client.template_render(
            str(task),
            search_path=(str(library),),
            var=("title=t", "size=1", "steps=[a]"),
        )
    )
    assert rendered.answers["title"] == "t"
