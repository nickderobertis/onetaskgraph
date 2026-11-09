"""The refusal every write of the relations spike passes before anything is sent.

Pure and offline: `harness.py` builds each mutation as a list of `Mut` and hands it here, so
`test_sandbox.py` can prove every refusal without a credential or a socket.
"""

# llmlint: ignore-file[new_code_lands_in_a_project] Spike branch kept for the record, not merged; see harness.py.
# llmlint: ignore-file[modern_domain_modeling] One script's refusal over the two trackers' JSON inputs, kept for one measurement; see harness.py.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] The whitelist below names the inputs this spike sends, and any root it did not list is refused; it is deliberately not derived from the trackers, whose schema is what the spike reads.

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

BOARD_TITLE_PREFIX = "onetaskgraph live board spike-relations-"
PROTECTED_BOARD_NUMBERS = {1, 2, 3}

GITHUB_ID_KEYS = {
    "issueId",
    "subIssueId",
    "blockingIssueId",
    "relatedIssueId",
    "duplicateIssueId",
    "duplicateId",
    "canonicalId",
    "projectId",
    "contentId",
    "itemId",
    "id",
    "subjectId",
    "afterId",
    "beforeId",
    "viewId",
}
LINEAR_ID_KEYS = {"parentId", "projectId", "issueId", "relatedIssueId", "id", "relatedProjectId"}

GITHUB_ROOTS = {
    "createIssue",
    "updateIssue",
    "deleteIssue",
    "closeIssue",
    "unmarkIssueAsDuplicate",
    "addSubIssue",
    "removeSubIssue",
    "addBlockedBy",
    "removeBlockedBy",
    "addRelatesTo",
    "removeRelatesTo",
    "addProjectV2ItemById",
    "createProjectV2",
    "updateProjectV2",
    "createProjectV2Field",
    "deleteProjectV2",
}
LINEAR_ROOTS = {
    "issueCreate",
    "issueUpdate",
    "issueDelete",
    "issueRelationCreate",
    "issueRelationUpdate",
    "projectCreate",
    "projectDelete",
}


class Refused(Exception):
    """A write outside the stated sandbox; raised before any byte is sent."""


@dataclass(frozen=True)
class Enum:
    """A GraphQL enum literal inside a `Mut` input."""

    name: str


@dataclass(frozen=True)
class Mut:
    """One aliased root field of a mutation document."""

    alias: str
    root: str
    args: dict[str, Any]
    selection: str = "clientMutationId"


def literal(value: Any) -> str:
    """A value as an inline GraphQL literal."""
    match value:
        case Enum(name=name):
            return name
        case dict():
            return "{" + ",".join(f"{k}:{literal(v)}" for k, v in value.items()) + "}"
        case list() | tuple():
            return "[" + ",".join(literal(v) for v in value) + "]"
        case _:
            return json.dumps(value)


def document(muts: list[Mut]) -> str:
    """The mutation document the guarded `muts` are sent as."""
    fields = [
        f"{m.alias}:{m.root}({','.join(f'{k}:{literal(v)}' for k, v in m.args.items())})"
        f"{{{m.selection}}}"
        for m in muts
    ]
    return "mutation{" + " ".join(fields) + "}"


@dataclass
class Sandbox:
    """What this run may write: its stamp, allowed containers, and what it created itself."""

    stamp: str
    owner_id: str
    repositories: set[str]
    teams: set[str]
    created: set[str]

    def _own(self, root: str, key: str, value: Any) -> None:
        if value not in self.created:
            raise Refused(f"{root}: {key}={value!r} was not created by run {self.stamp}")

    def _stamped(self, root: str, title: Any) -> None:
        if not isinstance(title, str) or self.stamp not in title:
            raise Refused(f"{root}: title {title!r} does not carry stamp {self.stamp}")

    def check_github(self, muts: list[Mut]) -> None:
        """Refuse any GitHub mutation outside the sandbox."""
        for mut in muts:
            if mut.root not in GITHUB_ROOTS:
                raise Refused(f"{mut.root}: not a mutation this spike may send to GitHub")
            payload = mut.args.get("input", mut.args)
            if mut.root == "createIssue":
                if payload.get("repositoryId") not in self.repositories:
                    raise Refused(f"createIssue: repository {payload.get('repositoryId')!r}")
                self._stamped(mut.root, payload.get("title"))
                continue
            if mut.root == "createProjectV2":
                title = payload.get("title", "")
                if payload.get("ownerId") != self.owner_id:
                    raise Refused(f"createProjectV2: owner {payload.get('ownerId')!r}")
                if not title.startswith(BOARD_TITLE_PREFIX + self.stamp):
                    raise Refused(f"createProjectV2: title {title!r}")
                continue
            if "title" in payload:
                self._stamped(mut.root, payload["title"])
            for key, value in payload.items():
                if key in GITHUB_ID_KEYS:
                    self._own(mut.root, key, value)

    def check_linear(self, muts: list[Mut]) -> None:
        """Refuse any Linear mutation outside the sandbox."""
        for mut in muts:
            if mut.root not in LINEAR_ROOTS:
                raise Refused(f"{mut.root}: not a mutation this spike may send to Linear")
            payload = mut.args.get("input", {})
            if mut.root in {"issueCreate", "projectCreate"}:
                teams = [payload["teamId"]] if "teamId" in payload else payload.get("teamIds", [])
                if not teams or any(team not in self.teams for team in teams):
                    raise Refused(f"{mut.root}: team {teams!r}")
                title = payload.get("title", payload.get("name"))
                self._stamped(mut.root, title)
                if payload.get("parentId") is not None:
                    self._own(mut.root, "parentId", payload["parentId"])
                if payload.get("projectId") is not None:
                    self._own(mut.root, "projectId", payload["projectId"])
                continue
            if mut.root in {"issueUpdate", "issueDelete", "issueRelationUpdate", "projectDelete"}:
                self._own(mut.root, "id", mut.args.get("id"))
            for key, value in payload.items():
                if key in LINEAR_ID_KEYS and value is not None:
                    self._own(mut.root, key, value)
            if "title" in payload:
                self._stamped(mut.root, payload["title"])
            if "teamId" in payload and payload["teamId"] not in self.teams:
                raise Refused(f"{mut.root}: moves to team {payload['teamId']!r}")


def refuse_protected_board(number: int) -> None:
    """Refuse a real-plugin source configured on a board this spike may only read."""
    if number in PROTECTED_BOARD_NUMBERS:
        raise Refused(f"board {number} is outside the sandbox")
