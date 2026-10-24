"""`proxy-refusal`, offline: the guarded proxy refusing writes to any other destination.

The proxy is started exactly as a live probe starts it.

Its upstream is a loopback stand-in recording what reaches it, so a write that leaves would
show there. It is allowed two test-board ids and sent each `Case`; a case passes when its status
is the expected one and exactly the expected number of requests reached the stand-in.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from pathlib import Path

from gaps.github import Client, Token
from gaps.json_shape import Json, encode, obj
from gaps.relay import relay

REPO_ID, PROJECT_ID = "R_test_board_repository", "PVT_test_board_project"
ELSEWHERE_REPO, ELSEWHERE_PROJECT = "R_another_repository", "PVT_another_project"
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
    'mutation { createIssue(input: {repositoryId: "R_test_board_repository", title: "x",'
    ' projectV2Ids: ["PVT_another_project"]}) { issue { id } } }'
)


@dataclass(frozen=True)
class Case:
    """One write put to the proxy."""

    label: str
    document: str
    variables: dict[str, Json]
    expected_status: int


CASES = (
    Case(
        "createIssue in another repository",
        CREATE,
        {"input": {"repositoryId": ELSEWHERE_REPO, "title": "x"}},
        403,
    ),
    Case(
        "addProjectV2ItemById to another project", ADD, {"p": ELSEWHERE_PROJECT, "c": "I_any"}, 403
    ),
    Case(
        "createIssue on the test repository placed on another project",
        CREATE,
        {"input": {"repositoryId": REPO_ID, "title": "x", "projectV2Ids": [ELSEWHERE_PROJECT]}},
        403,
    ),
    Case("projectV2Ids written inline, naming another project", INLINE, {}, 403),
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
        "createIssue naming only the test board",
        CREATE,
        {"input": {"repositoryId": REPO_ID, "title": "x", "projectV2Ids": [PROJECT_ID]}},
        200,
    ),
)


@dataclass(frozen=True)
class Outcome:
    """What one case did."""

    case: str
    status: int
    expected: int
    error_types: tuple[str, ...]
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
        """Whether every case behaved as expected."""
        return all(o.as_expected for o in self.cases)

    def to_json(self) -> Json:
        """The result as `result.json` holds it, its verdict stated beside it."""
        return obj(encode(self), "refusal") | {"all_as_expected": self.all_as_expected}


@dataclass
class StandIn:
    """The upstream: answers every request with an empty GraphQL answer, counting them."""

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
            await reader.readexactly(length)
            self.seen.append(lines[0])
            payload = b'{"data": {}}'
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


async def run(run_dir: Path) -> RefusalResult:
    """Put every case to the guarded proxy and record what it did."""
    stand_in = StandIn()
    server = await asyncio.start_server(stand_in.handle, "127.0.0.1", 0)
    upstream = f"http://127.0.0.1:{server.sockets[0].getsockname()[1]}"
    outcomes = []
    try:
        async with relay(run_dir, (REPO_ID, PROJECT_ID), upstream=upstream) as proxy:
            client = Client(proxy.endpoint, Token("placeholder"))
            for case in CASES:
                before = len(stand_in.seen)
                answer = await client.graphql(case.label, case.document, case.variables)
                outcomes.append(
                    Outcome(
                        case=case.label,
                        status=answer.record.status,
                        expected=case.expected_status,
                        error_types=answer.record.errors,
                        reached_upstream=len(stand_in.seen) - before,
                    )
                )
    finally:
        server.close()
        await server.wait_closed()
    result = RefusalResult(cases=tuple(outcomes))
    (run_dir / "result.json").write_text(json.dumps(result.to_json(), indent=1))
    return result
