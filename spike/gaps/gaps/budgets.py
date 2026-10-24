"""The 16 candidate budgets, read from the plan project's own task records on every analysis.

The plan's tasks carry them under `orchestrator.budgets`; this module asks the real CLI for the
project's tasks and keeps every record of a known unit, so the spike states no budget, bar or
unit of its own. Which baseline scenario a budget is held against follows from its id: a
`github-` prefix is the bare store and none the host recipe, and the operation is the id's
`first`, `content-creations`, `changed`, `unchanged` or `approval`.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path

from gaps import proc
from gaps.json_shape import Json, ShapeError, arr, decode, obj, parse, text

PROJECT = "authoring:board-copy-gaps-2026-10-24"


class Unit(StrEnum):
    """What a budget counts."""

    POINTS = "points"
    SECONDS = "seconds"
    WRITES = "writes"


class Operation(StrEnum):
    """The measured operation a budget is held against."""

    BARE_FIRST = "bare first"
    BARE_CHANGED = "bare changed"
    BARE_UNCHANGED = "bare unchanged"
    HOST_FIRST = "host first"
    HOST_CHANGED = "host changed"
    HOST_UNCHANGED = "host unchanged"
    HOST_APPROVAL = "host approval"


@dataclass(frozen=True)
class Budget:
    """One budget record, as the plan states it."""

    id: str
    name: str
    threshold: float
    unit: Unit
    repository: str
    workload: str

    @property
    def operation(self) -> Operation:
        """The operation the id names."""
        bare = self.id.startswith("github-")
        if "approval" in self.id and not bare:
            return Operation.HOST_APPROVAL
        for marker, (on_bare, on_host) in {
            "-unchanged": (Operation.BARE_UNCHANGED, Operation.HOST_UNCHANGED),
            "-changed": (Operation.BARE_CHANGED, Operation.HOST_CHANGED),
            "-first": (Operation.BARE_FIRST, Operation.HOST_FIRST),
            "-content-creations": (Operation.BARE_FIRST, Operation.HOST_FIRST),
        }.items():
            if self.id.endswith(marker):
                return on_bare if bare else on_host
        raise ShapeError(f"budget {self.id} names no operation this spike measured")


def from_tasks(listing: Json) -> tuple[Budget, ...]:
    """Every budget record of every task in one `task list --json` page."""
    out: list[Budget] = []
    for index, entry in enumerate(arr(obj(listing, "listing").get("items"), "items")):
        item = obj(obj(entry, f"items[{index}]").get("item"), f"items[{index}].item")
        metadata = obj(item.get("metadata", {}), f"items[{index}].metadata")
        records = metadata.get("orchestrator.budgets", [])
        for number, record in enumerate(arr(records, "orchestrator.budgets")):
            out.append(decode(Budget, record, f"items[{index}].budgets[{number}]"))
    return tuple(sorted(out, key=lambda b: b.id))


async def read(onetaskgraph: Path, cwd: Path, limit: int | None = None) -> tuple[Budget, ...]:
    """The plan project's budgets, every page of its listing through the real CLI from `cwd`.

    A page reporting a source error, or a continuation token already followed, is refused rather
    than taken as the whole project.
    """
    command = [onetaskgraph, "task", "list", "--project", PROJECT, "--json"]
    if limit is not None:
        command += ["--limit", str(limit)]
    out: list[Budget] = []
    followed: set[str] = set()
    token: str | None = None
    while True:
        done = await proc.check(command + ([] if token is None else ["--page", token]), cwd=cwd)
        page = obj(parse(done.out, "task list"), "task list")
        if arr(page.get("errors", []), "task list errors"):
            raise ShapeError(f"task list --project {PROJECT} reported errors: {page['errors']}")
        out.extend(from_tasks(page))
        following = page.get("next")
        if following is None:
            return tuple(sorted(out, key=lambda b: b.id))
        token = text(following, "task list next")
        if token in followed:
            raise ShapeError(f"task list --project {PROJECT} repeated its page token {token}")
        followed.add(token)
