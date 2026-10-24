"""`proxy-refusal`, offline: the guard refusing every write it owes a refusal.

The guard is started exactly as a live probe starts it, once allowed the two test-board ids
and once allowed nothing, in front of a loopback stand-in that records what reaches it and
answers a `createIssue` with an issue, so a request that leaves would show there. A case passes
when its status is the expected one and exactly the expected number of requests reached the
stand-in.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from enum import StrEnum
from pathlib import Path
from urllib.parse import urlsplit

from gaps.github import Client, Token, split_reply
from gaps.json_shape import Json, encode, obj
from gaps.relay import relay

REPO_ID, PROJECT_ID = "R_test_board_repository", "PVT_test_board_project"
ELSEWHERE_REPO, ELSEWHERE_PROJECT = "R_another_repository", "PVT_another_project"
MADE = "I_made_by_this_run"
CREATE = "mutation($input: CreateIssueInput!) { createIssue(input: $input) { issue { id } } }"
ADD = (
    "mutation($p: ID!, $c: ID!) { addProjectV2ItemById(input: {projectId: $p, contentId: $c})"
    " { item { id } } }"
)
ALIASED = (
    "mutation($a: CreateIssueInput!, $b: CreateIssueInput!) {"
    " a: createIssue(input: $a) { issue { id } } b: createIssue(input: $b) { issue { id } } }"
)
INLINE = (
    f'mutation {{ createIssue(input: {{repositoryId: "{REPO_ID}", title: "x",'
    f' projectV2Ids: ["{ELSEWHERE_PROJECT}"]}}) {{ issue {{ id }} }} }}'
)
INLINE_VARIABLE = (
    f'mutation($p: ID!) {{ createIssue(input: {{repositoryId: "{REPO_ID}", title: "x",'
    " projectV2Ids: [$p]}) { issue { id } } }"
)
UNSENT = (
    "mutation($p: ID!, $i: ID!) { deleteProjectV2Item(input: {projectId: $p, itemId: $i})"
    " { deletedItemId } }"
)
DELETE = "mutation($i: ID!) { deleteIssue(input: {issueId: $i}) { repository { id } } }"
READ = "query { viewer { login } }"
READ_AN_ISSUE = 'query { x: node(id: "I_returned_by_a_read") { id } }'
RETURNED = "I_returned_by_a_read"
COMMENTED = "# a comment first\n" + DELETE
FIELD = (
    "mutation($p: ID!, $i: ID!) { updateProjectV2ItemFieldValue(input: {projectId: $p,"
    ' itemId: $i, fieldId: "F", value: {singleSelectOptionId: "O"}}) { projectV2Item { id } } }'
)


class Allowed(StrEnum):
    """Which guard a case is put to."""

    TEST_BOARD = "the test board's two ids"
    NOTHING = "nothing"


@dataclass(frozen=True)
class Case:
    """One GraphQL request put to a guard."""

    label: str
    document: str
    variables: dict[str, Json]
    expected_status: int
    allowed: Allowed = Allowed.TEST_BOARD


@dataclass(frozen=True)
class RawCase:
    """One request a GraphQL client would never send, put to the guard byte for byte."""

    label: str
    request: bytes
    expected_status: int


CASES = (
    Case(
        "createIssue in another repository",
        CREATE,
        {"input": {"repositoryId": ELSEWHERE_REPO, "title": "x"}},
        403,
    ),
    Case(
        "addProjectV2ItemById to another project",
        ADD,
        {"p": ELSEWHERE_PROJECT, "c": "I_any"},
        403,
    ),
    Case(
        "createIssue on the test repository placed on another project",
        CREATE,
        {"input": {"repositoryId": REPO_ID, "title": "x", "projectV2Ids": [ELSEWHERE_PROJECT]}},
        403,
    ),
    Case("projectV2Ids written inline, naming another project", INLINE, {}, 403),
    Case(
        "projectV2Ids written inline around a variable naming another project",
        INLINE_VARIABLE,
        {"p": ELSEWHERE_PROJECT},
        403,
    ),
    Case(
        "aliased request, second mutation in another repository",
        ALIASED,
        {
            "a": {"repositoryId": REPO_ID, "title": "x"},
            "b": {"repositoryId": ELSEWHERE_REPO, "title": "y"},
        },
        403,
    ),
    Case(
        "a mutation the probes never send, on the test board",
        UNSENT,
        {"p": PROJECT_ID, "i": "PVTI_any"},
        403,
    ),
    Case("deleteIssue of an issue this run did not create", DELETE, {"i": "I_someone_else"}, 403),
    Case(
        "a mutation behind a comment, of an issue this run did not create",
        COMMENTED,
        {"i": "I_someone_else"},
        403,
    ),
    Case(
        "createIssue under a parent this run did not create",
        CREATE,
        {"input": {"repositoryId": REPO_ID, "title": "x", "parentIssueId": "I_someone_else"}},
        403,
    ),
    Case(
        "a field value on a board item of an issue this run did not create",
        FIELD,
        {"p": PROJECT_ID, "i": "PVTI_someone_else"},
        403,
    ),
    Case("a read answering an issue", READ_AN_ISSUE, {}, 200),
    Case("deleteIssue of an issue a read answered", DELETE, {"i": RETURNED}, 403),
    Case(
        "createIssue naming only the test board",
        CREATE,
        {"input": {"repositoryId": REPO_ID, "title": "x", "projectV2Ids": [PROJECT_ID]}},
        200,
    ),
    Case("deleteIssue of the issue this run just created", DELETE, {"i": MADE}, 200),
    Case(
        "createIssue naming only the test board, with no write allowed",
        CREATE,
        {"input": {"repositoryId": REPO_ID, "title": "x"}},
        403,
        Allowed.NOTHING,
    ),
    Case("a read, with no write allowed", READ, {}, 200, Allowed.NOTHING),
)
RAW = (
    RawCase(
        "an asset upload",
        b"POST /user-attachments/assets HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\n{}",
        403,
    ),
    RawCase(
        "a CONNECT tunnel", b"CONNECT github.com:443 HTTP/1.1\r\nHost: github.com\r\n\r\n", 403
    ),
    RawCase(
        "a GraphQL body that is not JSON",
        b"POST /graphql HTTP/1.1\r\nHost: x\r\nContent-Length: 8\r\n\r\nnot json",
        403,
    ),
    RawCase(
        "a negative Content-Length",
        b"POST /graphql HTTP/1.1\r\nHost: x\r\nContent-Length: -5\r\n\r\n",
        400,
    ),
)


@dataclass(frozen=True)
class Outcome:
    """What one case did."""

    case: str
    status: int
    expected: int
    reached_upstream: int

    @property
    def as_expected(self) -> bool:
        """Whether the status and what reached the upstream are the expected ones."""
        leaves = 1 if self.expected == 200 else 0
        return self.status == self.expected and self.reached_upstream == leaves


@dataclass(frozen=True)
class RefusalResult:
    """Every case's outcome."""

    cases: tuple[Outcome, ...]

    @property
    def all_as_expected(self) -> bool:
        """Whether every case behaved as expected; no case at all is not."""
        return bool(self.cases) and all(o.as_expected for o in self.cases)

    def to_json(self) -> Json:
        """The result as `result.json` holds it, its verdict stated beside it."""
        return obj(encode(self), "refusal") | {"all_as_expected": self.all_as_expected}


@dataclass
class StandIn:
    """The upstream: answers every request, a `createIssue` with an issue, counting them."""

    seen: list[str] = field(default_factory=list)

    async def handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        """Answer one request."""
        try:
            head = await reader.readuntil(b"\r\n\r\n")
            lines = head.decode("latin-1").split("\r\n")
            length = next(
                (int(x.split(":", 1)[1]) for x in lines if x.lower().startswith("content-length:")),
                0,
            )
            body = await reader.readexactly(length)
            self.seen.append(lines[0])
            made: dict[str, object] = {}
            if b"createIssue" in body:
                made = {"createIssue": {"issue": {"id": MADE}}}
            elif RETURNED.encode() in body:
                made = {"x": {"issue": {"id": RETURNED}}}
            payload = json.dumps({"data": made}).encode()
            writer.write(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n"
                + f"Content-Length: {len(payload)}\r\n\r\n".encode()
                + payload
            )
            await writer.drain()
        except (asyncio.IncompleteReadError, ValueError):
            pass
        finally:
            writer.close()


async def send_raw(endpoint: str, request: bytes) -> int:
    """Send bytes to the guard and answer the status it replied with."""
    target = urlsplit(endpoint)
    reader, writer = await asyncio.open_connection(target.hostname, target.port)
    try:
        writer.write(request)
        await writer.drain()
        return split_reply(await reader.read()).status
    finally:
        writer.close()


async def run(run_dir: Path) -> RefusalResult:
    """Put every case to the guard and record what it did."""
    stand_in = StandIn()
    server = await asyncio.start_server(stand_in.handle, "127.0.0.1", 0)
    upstream = f"http://127.0.0.1:{server.sockets[0].getsockname()[1]}"
    outcomes: list[Outcome] = []
    try:
        for allowed in Allowed:
            ids = (REPO_ID, PROJECT_ID) if allowed is Allowed.TEST_BOARD else ()
            async with relay(run_dir, ids, upstream=upstream) as proxy:
                client = Client(proxy.endpoint, Token("placeholder"))
                for case in (c for c in CASES if c.allowed is allowed):
                    before = len(stand_in.seen)
                    answer = await client.graphql(case.label, case.document, case.variables)
                    reached = len(stand_in.seen) - before
                    status = answer.record.status
                    outcomes.append(Outcome(case.label, status, case.expected_status, reached))
                if allowed is Allowed.TEST_BOARD:
                    for raw in RAW:
                        before = len(stand_in.seen)
                        status = await send_raw(proxy.endpoint, raw.request)
                        reached = len(stand_in.seen) - before
                        outcomes.append(Outcome(raw.label, status, raw.expected_status, reached))
    finally:
        server.close()
        await server.wait_closed()
    result = RefusalResult(cases=tuple(outcomes))
    (run_dir / "result.json").write_text(json.dumps(result.to_json(), indent=1))
    return result
