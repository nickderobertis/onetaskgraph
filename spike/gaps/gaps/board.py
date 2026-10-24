"""The test board this spike may write to, read from the one place it is declared.

`crates/onetaskgraph-github-live/src/lib.rs` declares the scratch repository and the board
the live lane and the janitor use; this module reads those constants out of that source on
every run, so the spike cannot drift onto another destination.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path
from typing import NewType

from gaps.github import Answer, require
from gaps.json_shape import Json, ShapeError, arr, obj, text

REPO = Path(__file__).resolve().parents[3]
LIVE_CRATE = REPO / "crates/onetaskgraph-github-live/src/lib.rs"

RepositoryId = NewType("RepositoryId", str)
ProjectId = NewType("ProjectId", str)
FieldId = NewType("FieldId", str)
OptionId = NewType("OptionId", str)


@dataclass(frozen=True)
class ScratchBoard:
    """Where the spike's writes may land."""

    owner: str
    number: int
    repository: str

    @property
    def repository_name(self) -> str:
        """The repository's name without its owner."""
        return self.repository.partition("/")[2]


def scratch_board(source: Path = LIVE_CRATE) -> ScratchBoard:
    """The board and scratch repository the live crate declares."""
    code = source.read_text(encoding="utf-8")

    def constant(name: str, pattern: str) -> str:
        match = re.search(rf"pub const {name}: [^=]+= {pattern};", code)
        if match is None:
            raise ShapeError(f"{source}: no `{name}` constant")
        return match.group(1)

    return ScratchBoard(
        owner=constant("BOARD_OWNER", r'"([^"]+)"'),
        number=int(constant("BOARD_NUMBER", r"(\d+)")),
        repository=constant("SCRATCH_REPOSITORY", r'"([^"]+)"'),
    )


@dataclass(frozen=True)
class BoardIds:
    """The node ids a probe writes with."""

    repository: RepositoryId
    project: ProjectId
    status_field: FieldId
    first_option: OptionId


IDS_QUERY = (
    "query ProbeIds($owner: String!, $name: String!, $number: Int!) {"
    " repository(owner: $owner, name: $name) { id }"
    " user(login: $owner) { projectV2(number: $number) { id"
    ' field(name: "Status") { ... on ProjectV2SingleSelectField { id options { id name } } } } } }'
)


def ids_variables(board: ScratchBoard) -> dict[str, Json]:
    """The variables `IDS_QUERY` is sent with."""
    return {"owner": board.owner, "name": board.repository_name, "number": board.number}


def board_ids(answer: Answer) -> BoardIds:
    """The ids `IDS_QUERY` answered, refusing an answer missing any of them."""
    data = require(answer, "ProbeIds")
    repository = obj(data.get("repository"), "ProbeIds.repository")
    project = obj(obj(data.get("user"), "ProbeIds.user").get("projectV2"), "ProbeIds.projectV2")
    status = obj(project.get("field"), "ProbeIds.projectV2.field")
    options = arr(status.get("options"), "ProbeIds.field.options")
    if not options:
        raise ShapeError("ProbeIds: the Status field has no option to write")
    return BoardIds(
        repository=RepositoryId(text(repository.get("id"), "repository.id")),
        project=ProjectId(text(project.get("id"), "projectV2.id")),
        status_field=FieldId(text(status.get("id"), "field.id")),
        first_option=OptionId(text(obj(options[0], "options[0]").get("id"), "options[0].id")),
    )
