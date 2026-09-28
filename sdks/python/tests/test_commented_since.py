"""`task_list(commented_since=...)`, against the real binary over two folders of Markdown.

The SDK's answer is held to the command line's own over the same store: one engine behind
both, so the same question may not come back two ways.
"""

from __future__ import annotations

import asyncio
import json
import subprocess
from pathlib import Path
from typing import Literal

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError

SINCE = "2026-09-20T12:00:00Z"
"""The instant every query below asks about."""

OLD = "2026-09-01T09:00:00Z"
"""Long before `SINCE`."""


def task(status: str, comments: list[tuple[str, str]]) -> str:
    """One task file at `status`, with a comment for each `(created_at, updated_at)` pair."""
    text = f"---\ntitle: A task\nstatus: {status}\n---\nThe body.\n"
    if comments:
        text += "\n## Comments\n"
        for index, (created, updated) in enumerate(comments):
            text += (
                f'\n<!-- onetaskgraph:comment id="c-{index}" author="ada" '
                f'created_at="{created}" updated_at="{updated}" -->\n'
                f"### ada — {created}\n\nA word.\n\n<!-- /onetaskgraph:comment -->\n"
            )
    return text


@pytest.fixture
def store(tmp_path: Path) -> Path:
    """Two folders — commented after the instant, never, only before it, and edited after it."""
    files = {
        ("home", "new"): task("todo", [("2026-09-21T09:00:00Z", "2026-09-21T09:00:00Z")]),
        ("home", "silent"): task("todo", []),
        ("home", "old"): task("todo", [(OLD, "2026-09-02T09:00:00Z")]),
        ("home", "edited"): task("shipped", [(OLD, "2026-09-25T09:00:00Z")]),
        ("away", "fresh"): task("shipped", [("2026-09-22T09:00:00Z", "2026-09-22T09:00:00Z")]),
        ("away", "stale"): task("todo", [(OLD, OLD)]),
    }
    for (folder, name), text in files.items():
        tasks = tmp_path / folder / "tasks"
        tasks.mkdir(parents=True, exist_ok=True)
        (tasks / f"{name}.md").write_text(text, encoding="utf-8")
    mapping = {"todo": "todo", "shipped": "done"}
    sources = {
        folder: {"plugin": "local-md", "config": {"root": folder, "status_mapping": mapping}}
        for folder in ("home", "away")
    }
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}))
    return tmp_path


def cli(binary: Path, store: Path, *arguments: str) -> list[str]:
    """The qualified ids `onetaskgraph task list` answers with, sorted."""
    ran = subprocess.run(
        [str(binary), "--json", "task", "list", "--limit", "50", *arguments],
        cwd=store,
        capture_output=True,
        encoding="utf-8",
        check=False,
    )
    assert ran.returncode == 0, ran.stderr
    return sorted(item["id"] for item in json.loads(ran.stdout)["items"])


def sdk(
    binary: Path,
    store: Path,
    commented_since: str,
    status: tuple[Literal["todo", "done"], ...] | None = None,
    source: list[str] | None = None,
) -> list[str]:
    """The qualified ids `task_list` answers with, sorted."""
    client = Client(binary, cwd=store)
    answered = asyncio.run(
        client.task_list(limit=50, commented_since=commented_since, status=status, source=source)
    )
    return sorted(item.id.root for item in answered.items)


@pytest.mark.parametrize(
    ("status", "source", "expected"),
    [
        (None, None, ["away:fresh", "home:edited", "home:new"]),
        (("done",), None, ["away:fresh", "home:edited"]),
        (None, ["home"], ["home:edited", "home:new"]),
    ],
)
def test_task_list_commented_since_answers_what_the_command_line_answers(
    binary: Path,
    store: Path,
    status: tuple[Literal["todo", "done"], ...] | None,
    source: list[str] | None,
    expected: list[str],
) -> None:
    """The SDK and the command line select exactly the same tasks, and the right ones."""
    arguments = ["--commented-since", SINCE]
    for category in status or []:
        arguments += ["--status", category]
    for name in source or []:
        arguments += ["--source", name]
    assert cli(binary, store, *arguments) == expected
    assert sdk(binary, store, SINCE, status=status, source=source) == expected


def test_an_instant_without_an_offset_is_refused_naming_the_flag(binary: Path, store: Path) -> None:
    """The binary's refusal reaches an SDK caller as an error that names the flag."""
    with pytest.raises(OnetaskgraphError) as refused:
        sdk(binary, store, "2026-09-20T12:00:00")
    assert "--commented-since" in str(refused.value)
    assert refused.value.exit_code == 2
