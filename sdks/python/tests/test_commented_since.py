"""`task_list(commented_since=...)`, against the real binary over two folders of Markdown.

The SDK's answer is held to the command line's own over the same store: one engine behind
both, so the same question may not come back two ways.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError
from onetaskgraph_sdk._generated.models import QueryResponseOfQualifiedTask

SINCE = "2026-09-20T12:00:00Z"
"""The instant every query below asks about."""

OLD = "2026-09-01T09:00:00Z"
"""Long before `SINCE`."""

type Category = Literal["todo", "done"]


@dataclass(frozen=True)
class HeldComment:
    """One comment in a task file's comments section."""

    created_at: str
    updated_at: str


@dataclass(frozen=True)
class TaskFile:
    """One task file of one folder, and the comments its section holds."""

    folder: str
    name: str
    status: Literal["todo", "shipped"]
    comments: tuple[HeldComment, ...] = ()

    def text(self) -> str:
        """The file as a person would have written it."""
        text = f"---\ntitle: A task\nstatus: {self.status}\n---\nThe body.\n"
        if self.comments:
            text += "\n## Comments\n"
            for index, comment in enumerate(self.comments):
                text += (
                    f'\n<!-- onetaskgraph:comment id="c-{index}" author="ada" '
                    f'created_at="{comment.created_at}" updated_at="{comment.updated_at}" -->\n'
                    f"### ada — {comment.created_at}\n\nA word.\n\n"
                    "<!-- /onetaskgraph:comment -->\n"
                )
        return text


STORE = (
    TaskFile("home", "new", "todo", (HeldComment("2026-09-21T09:00:00Z", "2026-09-21T09:00:00Z"),)),
    TaskFile("home", "silent", "todo"),
    TaskFile("home", "old", "todo", (HeldComment(OLD, "2026-09-02T09:00:00Z"),)),
    TaskFile("home", "edited", "shipped", (HeldComment(OLD, "2026-09-25T09:00:00Z"),)),
    TaskFile(
        "away", "fresh", "shipped", (HeldComment("2026-09-22T09:00:00Z", "2026-09-22T09:00:00Z"),)
    ),
    TaskFile("away", "stale", "todo", (HeldComment(OLD, OLD),)),
)
"""Commented after the instant, never, only before it, and an old comment edited after it."""


@dataclass(frozen=True)
class Case:
    """One `task list` question, and the tasks it has to answer with."""

    expected: list[str]
    status: tuple[Category, ...] = ()
    source: tuple[str, ...] = ()
    arguments: list[str] = field(init=False)

    def __post_init__(self) -> None:
        """Spell the same question as the command line's flags."""
        arguments = ["--commented-since", SINCE]
        for category in self.status:
            arguments += ["--status", category]
        for name in self.source:
            arguments += ["--source", name]
        object.__setattr__(self, "arguments", arguments)


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """Two folders of Markdown, `home` and `away`, holding `STORE`."""
    for task in STORE:
        tasks = tmp_path / task.folder / "tasks"
        tasks.mkdir(parents=True, exist_ok=True)
        (tasks / f"{task.name}.md").write_text(task.text(), encoding="utf-8", newline="\n")
    mapping = {"todo": "todo", "shipped": "done"}
    sources = {
        folder: {"plugin": "local-md", "config": {"root": folder, "status_mapping": mapping}}
        for folder in ("home", "away")
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
    binary: Path,
    store: Path,
    commented_since: str,
    status: tuple[Category, ...] = (),
    source: tuple[str, ...] = (),
) -> list[str]:
    """The qualified ids `task_list` answers with, sorted."""
    client = Client(binary, cwd=store)
    answered = await client.task_list(
        limit=50, commented_since=commented_since, status=status, source=source
    )
    return sorted(item.id.root for item in answered.items)


@pytest.mark.parametrize(
    "case",
    [
        Case(expected=["away:fresh", "home:edited", "home:new"]),
        Case(expected=["away:fresh", "home:edited"], status=("done",)),
        Case(expected=["home:edited", "home:new"], source=("home",)),
    ],
    ids=["alone", "with-a-status", "with-a-source"],
)
def test_task_list_commented_since_answers_what_the_command_line_answers(
    binary: Path, store: Path, case: Case
) -> None:
    """The SDK and the command line select exactly the same tasks, and the right ones."""
    assert asyncio.run(cli(binary, store, case.arguments)) == case.expected
    assert (
        asyncio.run(sdk(binary, store, SINCE, status=case.status, source=case.source))
        == case.expected
    )


def test_an_instant_without_an_offset_is_refused_naming_the_flag(binary: Path, store: Path) -> None:
    """The binary's refusal reaches an SDK caller as an error that names the flag."""
    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(sdk(binary, store, "2026-09-20T12:00:00"))
    assert "--commented-since" in str(refused.value)
    assert refused.value.exit_code == 2
