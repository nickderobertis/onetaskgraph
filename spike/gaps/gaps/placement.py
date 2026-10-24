"""`probe-placement`: how GitHub places the issues a coalesced first copy creates.

Disposable issues in the scratch repository, every one deleted before the probe returns.

1. A parent, then eight children in one aliased request, each with `projectV2Ids` (and the
   children `parentIssueId`): does a create answer carry its board item, and how long after
   it does each item appear (one aliased read every `POLL` seconds)?
2. `addProjectV2ItemById` sent at once after a create, inside GitHub's own placement, and for
   an issue already placed: is any issue ever on the board twice?
3. Every field value and blocked-by edge the plan writes, as one request; then three more
   requests of 19 aliased field values, whose `x-ratelimit-used` rises price such a request.
4. One `subIssues` read of the parent, charged: does it answer every child?

Every issue id a create answers is written to `state` before anything else is sent, so an
interrupted probe's issues are deleted by the next one.
"""

from __future__ import annotations

import asyncio
import json
import time
from dataclasses import dataclass, replace
from enum import StrEnum
from pathlib import Path
from typing import NewType

from gaps import documents
from gaps.board import BoardIds
from gaps.documents import Doc
from gaps.github import Answer, Client, GitHubError, require
from gaps.harness import workload
from gaps.json_shape import Json, ShapeError, arr, encode, integer, load, obj, text

IssueId = NewType("IssueId", str)
ItemId = NewType("ItemId", str)

POLL = 0.5
RACE_SETTLE = 10.0
PLACEMENT_TIMEOUT = 45.0
DELETE_BATCH = 5  # 11 aliased deleteIssue in one request met RESOURCE_LIMITS_EXCEEDED at the 8th
CHILDREN = 8
RACERS = 2
NINETEEN = 19
REPEATS = 3
TITLE = documents.TITLE
ITEMS = "projectItems(first: 5) { nodes { id project { id } } }"
# The plan's ten blocked-by edges among its eight tasks, (task, blocked by); the parent is 0.
EDGES: tuple[tuple[int, int], ...] = workload.TASK_EDGES


@dataclass(frozen=True)
class Created:
    """One issue a create answered."""

    issue: IssueId
    parent: IssueId | None
    items: tuple[ItemId, ...]


@dataclass(frozen=True)
class Placement:
    """When each created issue first read as on the board."""

    issues: int
    placed: int
    polls: int
    poll_costs: tuple[int, ...]
    children_seconds_after_their_answer: tuple[float, ...]
    items_per_issue: tuple[int, ...]


class AddOutcome(StrEnum):
    """What one `addProjectV2ItemById` answered."""

    RETURNED_THE_ITEM = "returned the issue's one item"
    REFUSED_ALREADY_EXISTS = "refused: content already exists in this project"
    OTHER = "anything else"


ALREADY_EXISTS = "Content already exists in this project"


@dataclass(frozen=True)
class Idempotence:
    """What `addProjectV2ItemById` did to issues already, or about to be, placed."""

    racing_outcomes: tuple[AddOutcome, ...]
    racing_items_after_10s: tuple[int, ...]
    placed_outcome: AddOutcome
    placed_items_after_add: int

    @property
    def one_item_each_after_settling(self) -> bool:
        """Every issue read one item once the adds settled; every add returned it or refused.

        A final reading: it cannot see a duplicate that existed only between two reads.
        """
        return (
            set(self.racing_items_after_10s) == {1}
            and self.placed_items_after_add == 1
            and AddOutcome.OTHER not in (*self.racing_outcomes, self.placed_outcome)
        )


@dataclass(frozen=True)
class Cleanup:
    """What the probe's deletion found and did."""

    deleted: int
    already_gone: int
    refused: int
    left: int
    errors: tuple[str, ...]


@dataclass(frozen=True)
class PlacementResult:
    """Everything `probe-placement` observed."""

    board_items_in_create_answers: int
    children_with_parent: int
    placement: Placement
    idempotence: Idempotence
    coalesced_write_answered: int
    coalesced_write_aliases: int
    coalesced_write_seconds: float
    reference_resolution_cost: int
    reference_resolution_children: int
    cleanup: Cleanup

    def to_json(self) -> Json:
        """The result as `feasibility.json` holds it."""
        return encode(self)


class LeftBehind(RuntimeError):
    """Probe issues the cleanup could not delete."""


class State:
    """The probe's own issue ids, persisted before any later request is sent."""

    def __init__(self, path: Path) -> None:
        """Load whatever an interrupted run left recorded."""
        self.path = path
        self.ids: list[IssueId] = []
        if path.exists():
            self.ids = [IssueId(text(v, f"{path}[]")) for v in arr(load(path), str(path))]

    def add(self, issues: list[IssueId]) -> None:
        """Record `issues` as this probe's."""
        self.ids += issues
        self.save()

    def save(self) -> None:
        """Write the recorded ids."""
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_text(json.dumps(self.ids))


def create_document(label: str, count: int) -> str:
    """`count` aliased `createIssue`, each answering its issue, parent and board items."""
    defs = ", ".join(f"$c{k}: CreateIssueInput!" for k in range(count))
    aliases = " ".join(
        f"c{k}: createIssue(input: $c{k}) {{ issue {{ id parent {{ id }} {ITEMS} }} }}"
        for k in range(count)
    )
    return f"mutation ProbeCreate{label}({defs}) {{ {aliases} }}"


def placed_document(count: int) -> str:
    """One aliased read of `count` issues' board items, with its stated cost."""
    aliases = " ".join(
        f"i{k}: node(id: $id{k}) {{ ... on Issue {{ id {ITEMS} }} }}" for k in range(count)
    )
    defs = ", ".join(f"$id{k}: ID!" for k in range(count))
    return f"query ProbePlaced({defs}) {{ rateLimit {{ cost }} {aliases} }}"


ADD_ALIASES = ("a", "b", "c")


def add_document() -> str:
    """Three aliased `addProjectV2ItemById`: two racing GitHub's placement, one after it."""
    adds = " ".join(
        f"{a}: addProjectV2ItemById(input: {{projectId: $p, contentId: ${a}}}) {{ item {{ id }} }}"
        for a in ADD_ALIASES
    )
    return f"mutation ProbeAdd($p: ID!, $a: ID!, $b: ID!, $c: ID!) {{ {adds} }}"


def coalesced_document(items: int, used: list[int]) -> str:
    """Every field value of `items` items and every plan edge, as one request."""
    defs = ["$p: ID!", "$f: ID!", "$o: String!"]
    defs += [f"$i{k}: ID!" for k in range(items)] + [f"$x{k}: ID!" for k in used]
    writes = [field_write(f"v{k}", f"i{k}") for k in range(items)]
    writes += [
        f"e{n}: addBlockedBy(input: {{issueId: $x{t}, blockingIssueId: $x{b}}})"
        " { issue { id } }"
        for n, (t, b) in enumerate(EDGES)
    ]
    return f"mutation ProbeCoalesced({', '.join(defs)}) {{ {' '.join(writes)} }}"


def nineteen_document() -> str:
    """`NINETEEN` aliased field values in one request."""
    defs = ["$p: ID!", "$f: ID!", "$o: String!"] + [f"$i{k}: ID!" for k in range(NINETEEN)]
    writes = " ".join(field_write(f"v{k}", f"i{k}") for k in range(NINETEEN))
    return f"mutation ProbeNineteen({', '.join(defs)}) {{ {writes} }}"


def own_document(count: int) -> str:
    """One aliased read of `count` issues' titles and repositories."""
    reads = " ".join(
        f"r{k}: node(id: $r{k}) {{ ... on Issue {{ id title repository {{ id }} }} }}"
        for k in range(count)
    )
    return f"query ProbeOwn({', '.join(f'$r{k}: ID!' for k in range(count))}) {{ {reads} }}"


def delete_document(count: int) -> str:
    """`count` aliased `deleteIssue`."""
    aliases = " ".join(
        f"d{k}: deleteIssue(input: {{issueId: $d{k}}}) {{ repository {{ id }} }}"
        for k in range(count)
    )
    defs = ", ".join(f"$d{k}: ID!" for k in range(count))
    return f"mutation ProbeDelete({defs}) {{ {aliases} }}"


def on_board(node: Json, project: str, where: str) -> tuple[ItemId, ...]:
    """The items of `project` an issue's `projectItems` lists."""
    if node is None:
        return ()
    connection = obj(obj(node, where).get("projectItems"), f"{where}.projectItems")
    nodes = arr(connection.get("nodes"), f"{where}.projectItems.nodes")
    out = []
    for item in nodes:
        entry = obj(item, f"{where}.item")
        if text(obj(entry.get("project"), f"{where}.project").get("id"), "project.id") == project:
            out.append(ItemId(text(entry.get("id"), f"{where}.item.id")))
    return tuple(out)


async def create(
    client: Client, state: State, inputs: list[dict[str, Json]], label: str, project: str
) -> tuple[list[Created], float]:
    """Create every input in one aliased request; answer each issue and when the answer came."""
    answer = await client.graphql(
        f"ProbeCreate{label}",
        create_document(label, len(inputs)),
        {f"c{k}": value for k, value in enumerate(inputs)},
        aliases=len(inputs),
    )
    answered = [answer.data.get(f"c{k}") for k in range(len(inputs))]
    issues = [i.get("issue") if isinstance(i, dict) else None for i in answered]
    # Every id an answer carries is recorded before anything about the answer is refused, so a
    # create answered in part, or malformed in part, still has each issue it made deleted.
    ids = [
        IssueId(found)
        for i in issues
        if isinstance(i, dict) and isinstance(found := i.get("id"), str)
    ]
    state.add(ids)
    require(answer, f"ProbeCreate{label}")
    if len(ids) != len(inputs):
        raise ShapeError(f"ProbeCreate{label}: {len(inputs)} sent, {len(ids)} answered")
    out = []
    for issue in issues:
        entry = obj(issue, "issue")
        parent = entry.get("parent")
        out.append(
            Created(
                issue=IssueId(text(entry.get("id"), "issue.id")),
                parent=None
                if parent is None
                else IssueId(text(obj(parent, "parent").get("id"), "parent.id")),
                items=on_board(entry, project, "issue"),
            )
        )
    return out, time.time()


async def read_items(
    client: Client, issues: list[IssueId], project: str
) -> tuple[dict[IssueId, tuple[ItemId, ...]], int]:
    """One aliased read of every issue's board items, and what GitHub charged for it."""
    answer = await client.graphql(
        "ProbePlaced",
        placed_document(len(issues)),
        {f"id{k}": issue for k, issue in enumerate(issues)},
        aliases=len(issues),
    )
    data = require(answer, "ProbePlaced")
    cost = integer(obj(data.get("rateLimit"), "rateLimit").get("cost"), "rateLimit.cost")
    return {
        issue: on_board(data.get(f"i{k}"), project, f"i{k}") for k, issue in enumerate(issues)
    }, cost


async def poll(
    client: Client, created: dict[IssueId, float], project: str
) -> tuple[Placement, dict[IssueId, tuple[ItemId, ...]]]:
    """Read every created issue's items until all are placed or `PLACEMENT_TIMEOUT` passes."""
    issues = list(created)
    seen: dict[IssueId, float] = {}
    items: dict[IssueId, tuple[ItemId, ...]] = {}
    costs: list[int] = []
    deadline = time.time() + PLACEMENT_TIMEOUT
    while len(seen) < len(issues) and time.time() < deadline:
        answer, cost = await read_items(client, issues, project)
        costs.append(cost)
        now = time.time()
        for issue, found in answer.items():
            if found and issue not in seen:
                seen[issue] = now - created[issue]
                items[issue] = found
        if len(seen) < len(issues):
            await asyncio.sleep(POLL)
    children = tuple(round(seen[i], 3) for i in issues[1:] if i in seen)
    return (
        Placement(
            issues=len(issues),
            placed=len(seen),
            polls=len(costs),
            poll_costs=tuple(sorted(set(costs))),
            children_seconds_after_their_answer=children,
            items_per_issue=tuple(sorted({len(v) for v in items.values()})),
        ),
        items,
    )


async def race(client: Client, state: State, ids: BoardIds, placed: IssueId) -> Idempotence:
    """`addProjectV2ItemById` inside GitHub's own placement, and after it."""
    inputs: list[dict[str, Json]] = [
        {
            "repositoryId": ids.repository,
            "title": f"{TITLE} race {k}",
            "projectV2Ids": [ids.project],
        }
        for k in range(1, RACERS + 1)
    ]
    racers, _ = await create(client, state, inputs, "Race", ids.project)
    variables: dict[str, Json] = {"p": ids.project, "a": racers[0].issue, "b": racers[1].issue}
    variables["c"] = placed
    answer = await client.graphql("ProbeAdd", add_document(), variables, aliases=len(ADD_ALIASES))
    await asyncio.sleep(RACE_SETTLE)
    targets = [racers[0].issue, racers[1].issue, placed]
    after, _ = await read_items(client, targets, ids.project)
    outcomes = [
        add_outcome(answer, alias, after[issue])
        for alias, issue in zip(ADD_ALIASES, targets, strict=True)
    ]
    return Idempotence(
        racing_outcomes=tuple(outcomes[:RACERS]),
        racing_items_after_10s=tuple(len(after[issue]) for issue in targets[:RACERS]),
        placed_outcome=outcomes[RACERS],
        placed_items_after_add=len(after[placed]),
    )


def add_outcome(answer: Answer, alias: str, items: tuple[ItemId, ...]) -> AddOutcome:
    """What one aliased add answered, against the items its issue holds afterwards."""
    refusals = [e for e in answer.errors if e.path[:1] == (alias,)]
    if refusals:
        refused = all(ALREADY_EXISTS in e.message for e in refusals)
        return AddOutcome.REFUSED_ALREADY_EXISTS if refused else AddOutcome.OTHER
    payload = answer.data.get(alias)
    item = payload.get("item") if isinstance(payload, dict) else None
    found = item.get("id") if isinstance(item, dict) else None
    # A malformed payload, or an item other than the one the issue reads, is an outcome to
    # record, not a reason to abandon the probe before its cleanup.
    return AddOutcome.RETURNED_THE_ITEM if (found,) == items else AddOutcome.OTHER


def field_write(alias: str, item: str) -> str:
    """One aliased `updateProjectV2ItemFieldValue` setting Status to the first option."""
    return (
        f"{alias}: updateProjectV2ItemFieldValue(input: {{projectId: $p, itemId: ${item},"
        " fieldId: $f, value: {singleSelectOptionId: $o}}) { projectV2Item { id } }"
    )


@dataclass(frozen=True)
class CoalescedWrite:
    """One request carrying every field value and every edge."""

    answered: int
    aliases: int
    seconds: float


async def coalesced_write(
    client: Client, ids: BoardIds, issues: list[IssueId], items: list[ItemId]
) -> CoalescedWrite:
    """Every field value and every edge as one request."""
    used = sorted({k for edge in EDGES for k in edge})
    variables: dict[str, Json] = {"p": ids.project, "f": ids.status_field, "o": ids.first_option}
    variables |= {f"i{k}": v for k, v in enumerate(items)} | {f"x{k}": issues[k] for k in used}
    answer = await client.graphql(
        "ProbeCoalesced",
        coalesced_document(len(items), used),
        variables,
        aliases=len(items) + len(EDGES),
    )
    data = require(answer, "ProbeCoalesced")
    answered = sum(1 for value in data.values() if value)
    aliases = len(items) + len(EDGES)
    return CoalescedWrite(answered=answered, aliases=aliases, seconds=answer.seconds)


async def nineteen(client: Client, ids: BoardIds, items: list[ItemId]) -> None:
    """`REPEATS` requests of 19 aliased field values, which the analysis prices by their rise."""
    document = nineteen_document()
    variables: dict[str, Json] = {"p": ids.project, "f": ids.status_field, "o": ids.first_option}
    variables |= {f"i{k}": items[k % len(items)] for k in range(NINETEEN)}
    for _ in range(REPEATS):
        require(await client.graphql("ProbeNineteen", document, variables, aliases=NINETEEN), "19")


async def references(client: Client, parent: IssueId) -> tuple[int, int]:
    """One charged `SUB_ISSUES` read of the parent: its cost and the children it answered."""
    document = documents.charged(documents.snapshot()[Doc.SUB_ISSUES])
    data = require(await client.graphql("SUB_ISSUES", document, documents.members(parent)), "SUB")
    cost = integer(obj(data.get("rateLimit"), "rateLimit").get("cost"), "rateLimit.cost")
    sub = obj(obj(data.get("node"), "node").get("subIssues"), "node.subIssues")
    return cost, len(arr(sub.get("nodes"), "subIssues.nodes"))


def confirmed(payload: Json) -> bool:
    """Whether a `deleteIssue` payload confirms the delete: it names the repository."""
    repository = payload.get("repository") if isinstance(payload, dict) else None
    return isinstance(repository, dict) and isinstance(repository.get("id"), str)


async def delete(client: Client, state: State, ids: BoardIds) -> Cleanup:
    """Delete every recorded issue that reads back as this probe's, `DELETE_BATCH` a request."""
    recorded = list(dict.fromkeys(state.ids))
    if not recorded:
        return Cleanup(0, 0, 0, 0, ())
    answer = await client.graphql(
        "ProbeOwn",
        own_document(len(recorded)),
        {f"r{k}": v for k, v in enumerate(recorded)},
    )
    not_found = {e.path[0] for e in answer.errors if e.type == "NOT_FOUND" and e.path}
    unreadable = [e for e in answer.errors if e.type != "NOT_FOUND"]
    if unreadable:
        raise GitHubError(f"ProbeOwn: {unreadable[0].type}: {unreadable[0].message}")
    gone = [recorded[k] for k in range(len(recorded)) if f"r{k}" in not_found]
    own: list[IssueId] = []
    for k, issue in enumerate(recorded):
        node = answer.data.get(f"r{k}")
        if node is None:
            continue
        entry = obj(node, f"r{k}")
        repository = text(obj(entry.get("repository"), "repository").get("id"), "repository.id")
        if text(entry.get("title"), "title").startswith(TITLE) and repository == ids.repository:
            own.append(issue)
    deleted: list[IssueId] = []
    errors: list[str] = []
    for start in range(0, len(own), DELETE_BATCH):
        batch = own[start : start + DELETE_BATCH]
        result = await client.graphql(
            "ProbeDelete",
            delete_document(len(batch)),
            {f"d{k}": v for k, v in enumerate(batch)},
            aliases=len(batch),
        )
        failed = {e.path[0] for e in result.errors if e.path}
        errors += [e.type for e in result.errors]
        # A delete counts only when its payload names the repository it left; anything else,
        # a null included, keeps the issue recorded for the next probe to delete.
        deleted += [
            v
            for k, v in enumerate(batch)
            if f"d{k}" not in failed and confirmed(result.data.get(f"d{k}"))
        ]
    state.ids = [i for i in recorded if i not in deleted and i not in gone]
    state.save()
    refused = len(recorded) - len(gone) - len(own)
    return Cleanup(len(deleted), len(gone), refused, len(state.ids), tuple(errors))


async def probe(client: Client, ids: BoardIds, state: State) -> PlacementResult:
    """Run every step, deleting every probe issue whatever happens; refuse leftovers."""
    try:
        result = await _steps(client, ids, state)
    except BaseException:
        await delete(client, state, ids)
        raise
    cleanup = await delete(client, state, ids)
    if cleanup.left or cleanup.errors:
        raise LeftBehind(f"{cleanup.left} probe issue(s) left; run the probe again to delete them")
    return replace(result, cleanup=cleanup)


async def _steps(client: Client, ids: BoardIds, state: State) -> PlacementResult:
    parent_input: dict[str, Json] = {
        "repositoryId": ids.repository,
        "title": f"{TITLE} parent",
        "projectV2Ids": [ids.project],
    }
    [parent], parent_end = await create(client, state, [parent_input], "Parent", ids.project)
    children_inputs: list[dict[str, Json]] = [
        {
            "repositoryId": ids.repository,
            "title": f"{TITLE} child {k}",
            "projectV2Ids": [ids.project],
            "parentIssueId": parent.issue,
        }
        for k in range(1, CHILDREN + 1)
    ]
    children, children_end = await create(client, state, children_inputs, "Children", ids.project)
    created = {parent.issue: parent_end} | {c.issue: children_end for c in children}
    placement, items = await poll(client, created, ids.project)
    if placement.placed < placement.issues:
        raise GitHubError(f"{placement.issues - placement.placed} issue(s) never placed")
    issues = [parent.issue] + [c.issue for c in children]
    item_ids = [items[i][0] for i in issues]
    idempotence = await race(client, state, ids, parent.issue)
    write = await coalesced_write(client, ids, issues, item_ids)
    await nineteen(client, ids, item_ids)
    cost, members = await references(client, parent.issue)
    return PlacementResult(
        board_items_in_create_answers=sum(1 for c in [parent, *children] if c.items),
        children_with_parent=sum(1 for c in children if c.parent == parent.issue),
        placement=placement,
        idempotence=idempotence,
        coalesced_write_answered=write.answered,
        coalesced_write_aliases=write.aliases,
        coalesced_write_seconds=round(write.seconds, 3),
        reference_resolution_cost=cost,
        reference_resolution_children=members,
        cleanup=Cleanup(0, 0, 0, 0, ()),
    )
