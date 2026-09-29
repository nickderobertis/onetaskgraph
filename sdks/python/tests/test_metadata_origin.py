"""`task_list(metadata=..., origin=...)`, against the real binary over two folders of Markdown.

The SDK's answer is held to the command line's own over the same store: one engine behind
both, so the same question may not come back two ways.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError
from onetaskgraph_sdk._generated.models import QueryResponseOfQualifiedTask


@dataclass(frozen=True)
class TaskFile:
    """One task file of one folder, and the `metadata:` block its front matter holds."""

    folder: str
    name: str
    metadata: str

    def text(self) -> str:
        """The file as a person would have written it."""
        return f"---\ntitle: A task\nstatus: todo\nmetadata: {self.metadata}\n---\nThe body.\n"


STORE = (
    TaskFile("home", "tagged", "{orchestrator.follow-up: {root_cause: stale-cache}, team: ada}"),
    TaskFile("home", "shallow", "{orchestrator.follow-up: stale-cache}"),
    TaskFile("home", "copied", '{onetaskgraph.origin: "work:ENG-1"}'),
    TaskFile("away", "tagged", "{orchestrator.follow-up: {root_cause: stale-cache}}"),
    TaskFile("away", "near", '{onetaskgraph.origin: "work:ENG-10"}'),
)
"""A nested root cause in both folders, the same value one level up, and two copy origins, one
of which only begins with the other."""


@dataclass(frozen=True)
class Case:
    """One `task list` question, and the tasks it has to answer with."""

    expected: list[str]
    metadata: tuple[str, ...] = ()
    origin: str | None = None
    arguments: list[str] = field(init=False)

    def __post_init__(self) -> None:
        """Spell the same question as the command line's flags."""
        arguments: list[str] = []
        for match in self.metadata:
            arguments += ["--metadata", match]
        if self.origin is not None:
            arguments += ["--origin", self.origin]
        object.__setattr__(self, "arguments", arguments)


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """Two folders of Markdown, `home` and `away`, holding `STORE`."""
    for task in STORE:
        tasks = tmp_path / task.folder / "tasks"
        tasks.mkdir(parents=True, exist_ok=True)
        (tasks / f"{task.name}.md").write_text(task.text(), encoding="utf-8", newline="\n")
    sources = {
        folder: {"plugin": "local-md", "config": {"root": folder}} for folder in ("home", "away")
    }
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    return tmp_path


async def cli(binary: Path, store: Path, arguments: list[str]) -> list[str]:
    """The qualified ids `onetaskgraph task list` answers with, sorted."""
    process = await asyncio.create_subprocess_exec(
        str(binary),
        "--json",
        "task",
        "list",
        "--limit",
        "50",
        *arguments,
        cwd=store,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, stderr = await process.communicate()
    assert process.returncode == 0, stderr.decode("utf-8")
    answered = QueryResponseOfQualifiedTask.model_validate_json(stdout)
    return sorted(item.id.root for item in answered.items)


async def sdk(
    binary: Path, store: Path, metadata: tuple[str, ...] = (), origin: str | None = None
) -> list[str]:
    """The qualified ids `task_list` answers with, sorted."""
    client = Client(binary, cwd=store)
    answered = await client.task_list(limit=50, metadata=metadata, origin=origin)
    return sorted(item.id.root for item in answered.items)


@pytest.mark.parametrize(
    "case",
    [
        Case(
            expected=["away:tagged", "home:tagged"],
            metadata=("orchestrator.follow-up/root_cause=stale-cache",),
        ),
        Case(
            expected=["home:tagged"],
            metadata=("orchestrator.follow-up/root_cause=stale-cache", "team=ada"),
        ),
        Case(expected=["home:shallow"], metadata=("orchestrator.follow-up=stale-cache",)),
        Case(expected=["home:copied"], origin="work:ENG-1"),
        Case(expected=[], origin="work:ENG"),
    ],
    ids=["nested", "anded", "top-level", "origin", "origin-prefix"],
)
def test_task_list_metadata_and_origin_answer_what_the_command_line_answers(
    binary: Path, store: Path, case: Case
) -> None:
    """The SDK and the command line select exactly the same tasks, and the right ones."""
    assert asyncio.run(cli(binary, store, case.arguments)) == case.expected
    assert (
        asyncio.run(sdk(binary, store, metadata=case.metadata, origin=case.origin)) == case.expected
    )


def test_an_origin_that_is_not_a_qualified_id_is_refused_naming_the_flag(
    binary: Path, store: Path
) -> None:
    """The binary's refusal reaches an SDK caller as an error that names the flag."""
    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(sdk(binary, store, origin="ENG-1"))
    assert "--origin" in str(refused.value)
    assert refused.value.exit_code == 2


def test_a_metadata_match_with_no_value_is_refused_naming_the_flag(
    binary: Path, store: Path
) -> None:
    """A match missing its `=` never reaches a source."""
    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(sdk(binary, store, metadata=("orchestrator.follow-up",)))
    assert "--metadata" in str(refused.value)
    assert refused.value.exit_code == 2
