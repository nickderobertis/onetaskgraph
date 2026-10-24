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

from gaps import documents
from gaps.documents import Doc
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


def ids_query() -> str:
    """The plugin's own `CREATION_CONTEXT` document: the board, its fields and the repository."""
    return documents.snapshot()[Doc.CREATION_CONTEXT]


def ids_variables(board: ScratchBoard) -> dict[str, Json]:
    """The variables `ids_query` is sent with."""
    return {
        "owner": board.owner,
        "number": board.number,
        "nestedFirst": 50,
        "repositoryOwner": board.repository.partition("/")[0],
        "repositoryName": board.repository_name,
    }


def board_ids(answer: Answer) -> BoardIds:
    """The ids `ids_query` answered, refusing an answer missing any of them."""
    data = require(answer, "CREATION_CONTEXT")
    repository = obj(data.get("repository"), "CREATION_CONTEXT.repository")
    owner = obj(data.get("boardFields"), "CREATION_CONTEXT.boardFields")
    project = obj(owner.get("projectV2"), "CREATION_CONTEXT.projectV2")
    fields = arr(obj(project.get("fields"), "projectV2.fields").get("nodes"), "fields.nodes")
    status = next(
        (obj(f, "field") for f in fields if isinstance(f, dict) and f.get("name") == "Status"),
        None,
    )
    if status is None:
        raise ShapeError("CREATION_CONTEXT: the board has no Status field")
    options = arr(status.get("options"), "Status.options")
    if not options:
        raise ShapeError("CREATION_CONTEXT: the Status field has no option to write")
    return BoardIds(
        repository=RepositoryId(text(repository.get("id"), "repository.id")),
        project=ProjectId(text(project.get("id"), "projectV2.id")),
        status_field=FieldId(text(status.get("id"), "Status.id")),
        first_option=OptionId(text(obj(options[0], "options[0]").get("id"), "options[0].id")),
    )
