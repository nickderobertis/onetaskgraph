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
from gaps.json_shape import Json, ShapeError, arr, decode, obj, parse

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
    """Every budget record of every task in a `task list --json` answer."""
    out: list[Budget] = []
    for index, entry in enumerate(arr(obj(listing, "listing").get("items"), "items")):
        item = obj(obj(entry, f"items[{index}]").get("item"), f"items[{index}].item")
        metadata = obj(item.get("metadata", {}), f"items[{index}].metadata")
        records = metadata.get("orchestrator.budgets", [])
        for number, record in enumerate(arr(records, "orchestrator.budgets")):
            out.append(decode(Budget, record, f"items[{index}].budgets[{number}]"))
    return tuple(sorted(out, key=lambda b: b.id))


async def read(onetaskgraph: Path, cwd: Path) -> tuple[Budget, ...]:
    """The plan project's budgets, through the real CLI configured from `cwd`."""
    done = await proc.check([onetaskgraph, "task", "list", "--project", PROJECT, "--json"], cwd=cwd)
    return from_tasks(parse(done.out, "task list"))
