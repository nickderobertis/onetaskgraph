#!/usr/bin/env python3
"""Close one throwaway issue as a duplicate of another, read back what GitHub kept, close both.

    python3 duplicate_close.py --repository nickderobertis/onetaskgraph-live-scratch \
        [--env-file PATH] [--endpoint URL] [--stop-after-points 30] [--out FILE] \
        [--readback-only SURVIVOR DUPLICATE]

It refuses any repository but the scratch one. Steps:

1. introspect ``CloseIssueInput``, ``ClosedEvent``, ``MarkedAsDuplicateEvent`` and ``Issue``'s
   duplicate fields, so nothing below names a field the schema lacks;
2. create the survivor and the duplicate (REST);
3. ``closeIssue(input:{issueId, stateReason: DUPLICATE, duplicateIssueId})`` on the duplicate;
4. read both back: ``state``, ``stateReason``, ``duplicateOf``, each timeline and the REST
   ``state_reason``;
5. close the survivor by REST ``PATCH state=closed, state_reason=duplicate``, which is the
   comparison, and read both back again, so nothing is left open.

``--readback-only`` writes nothing and runs step 4 for two existing issues.

Exit status: ``0`` when the duplicate reads back closed as ``DUPLICATE`` of the survivor and
both issues read back closed (or, with ``--readback-only``, when the read answered); ``1``
when GitHub refused a step, an answer did not have the shape it reads, a limit stopped it, or
what GitHub kept differs; ``2`` when the invocation is wrong, its
``--out`` cannot be written, or no token is configured. Once the schema is read, the record is printed as JSON on stdout
whatever the outcome; diagnostics go to stderr.
"""

from __future__ import annotations

import argparse
import enum
import json
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import NewType

from ghlive import (
    GRAPHQL_ENDPOINT,
    BoundaryError,
    RequestRecord,
    SpikeError,
    SpikeLimits,
    SpikeStop,
    Token,
    check_out,
    decode,
    expect_int,
    expect_list,
    expect_object,
    expect_str,
    graphql,
    load_token,
    positive,
    rate_headers,
    rest_base,
    send,
    write_out,
)

SCRATCH = "nickderobertis/onetaskgraph-live-scratch"
INTROSPECT = """query{
  closeInput:__type(name:"CloseIssueInput"){inputFields{name}}
  closedEvent:__type(name:"ClosedEvent"){fields{name}}
  marked:__type(name:"MarkedAsDuplicateEvent"){fields{name}}
  reasons:__type(name:"IssueClosedStateReason"){enumValues{name}}
  items:__type(name:"IssueTimelineItemsItemType"){enumValues{name}}
}"""
# GitHub allows `__Type.fields` twice per document, so `Issue`'s are asked for on their own.
INTROSPECT_ISSUE = 'query{issue:__type(name:"Issue"){fields{name}}}'
CLOSE = """mutation($id:ID!,$dup:ID!){closeIssue(input:{issueId:$id,stateReason:DUPLICATE,
  duplicateIssueId:$dup}){issue{number state stateReason(enableDuplicate:true)}}}"""


IssueNumber = NewType("IssueNumber", int)
NodeId = NewType("NodeId", str)


class Role(enum.StrEnum):
    SURVIVOR = "survivor"
    DUPLICATE = "duplicate"


@dataclass(frozen=True)
class CreatedIssue:
    role: Role
    number: IssueNumber
    node_id: NodeId


@dataclass(frozen=True)
class Schema:
    close_input: list[str]
    closed_event: list[str]
    marked_as_duplicate_event: list[str]
    issue_duplicate_fields: list[str]
    close_reasons: list[str]
    timeline_duplicate_types: list[str]


@dataclass(frozen=True)
class RestState:
    number: IssueNumber
    state: str
    state_reason: str | None
    html_url: str


@dataclass(frozen=True)
class RestClose:
    """What REST answered to ``PATCH state=closed, state_reason=duplicate``."""

    status: int
    state: str | None
    state_reason: str | None
    message: str | None


def optional_str(value: object, what: str) -> str | None:
    return None if value is None else expect_str(value, what)


@dataclass
class DuplicateCloseRun:
    repository: str
    schema: Schema
    created: list[CreatedIssue] = field(default_factory=list)
    close_answer: object = None
    rest_close: RestClose | None = None
    readback_after_close: object = None
    readback_final: object = None
    final_rest: list[RestState] = field(default_factory=list)
    points: int = 0
    # GitHub reports no cost for a mutation; each one sent is charged one point to the cap.
    close_mutations_sent: int = 0
    failure: str | None = None
    records: list[RequestRecord] = field(default_factory=list)


class Client:
    """The scratch repository, through REST and through metered GraphQL."""

    def __init__(
        self, endpoint: str, owner: str, name: str, token: Token, stop_after_points: int
    ):
        self.endpoint = endpoint
        self.owner = owner
        self.name = name
        self.token = token
        self.limits = SpikeLimits(stop_after_points=stop_after_points)
        self.records: list[RequestRecord] = []
        self.mutations_sent = 0

    def rest(
        self, method: str, path: str, payload: dict[str, object] | None = None
    ) -> tuple[int, dict[str, object]]:
        body = json.dumps(payload).encode() if payload is not None else None
        url = f"{rest_base(self.endpoint)}/repos/{self.owner}/{self.name}{path}"
        response = send(method, url, self.token, body)
        return response.status, decode(response.body, f"{method} {path}")

    def query(
        self, document: str, variables: dict[str, object], phase: str
    ) -> dict[str, object]:
        answer, _ = graphql(
            self.endpoint,
            document,
            variables,
            self.token,
            self.limits,
            self.records,
            phase,
        )
        if answer.get("errors"):
            raise BoundaryError(
                f"{phase} answered errors: {json.dumps(answer['errors'])[:600]}"
            )
        return expect_object(answer.get("data"), f"{phase} data")

    def rest_state(self, number: IssueNumber) -> RestState:
        status, issue = self.rest("GET", f"/issues/{number}")
        if status != 200:
            raise BoundaryError(f"reading issue {number} answered {status}: {issue}")
        reason = issue.get("state_reason")
        return RestState(
            number=IssueNumber(expect_int(issue.get("number"), "number")),
            state=expect_str(issue.get("state"), "state"),
            state_reason=None if reason is None else expect_str(reason, "state_reason"),
            html_url=expect_str(issue.get("html_url"), "html_url"),
        )


def names(value: object, key: str, what: str) -> list[str]:
    entries = expect_list(expect_object(value, what).get(key), f"{what}.{key}")
    return [
        expect_str(expect_object(e, what).get("name"), f"{what} name") for e in entries
    ]


def introspect(client: Client) -> Schema:
    data = client.query(INTROSPECT, {}, "introspect")
    issue = client.query(INTROSPECT_ISSUE, {}, "introspect")
    return Schema(
        close_input=names(data.get("closeInput"), "inputFields", "CloseIssueInput"),
        closed_event=sorted(names(data.get("closedEvent"), "fields", "ClosedEvent")),
        marked_as_duplicate_event=sorted(
            names(data.get("marked"), "fields", "MarkedAsDuplicate")
        ),
        issue_duplicate_fields=sorted(
            n for n in names(issue.get("issue"), "fields", "Issue") if "uplicate" in n
        ),
        close_reasons=names(
            data.get("reasons"), "enumValues", "IssueClosedStateReason"
        ),
        timeline_duplicate_types=[
            n
            for n in names(
                data.get("items"), "enumValues", "IssueTimelineItemsItemType"
            )
            if "DUPLICATE" in n
        ],
    )


def readback_document(schema: Schema) -> str:
    marked = (
        " ".join(
            f"{name}{{... on Issue{{number}} ... on PullRequest{{number}}}}"
            if name in {"canonical", "duplicate"}
            else name
            for name in ("canonical", "duplicate", "isCrossRepository", "createdAt")
            if name in schema.marked_as_duplicate_event
        )
        or "__typename"
    )
    closed = (
        " ".join(
            f"{name}{{... on Issue{{number}} ... on PullRequest{{number}}}}"
            if name == "duplicateOf"
            else name
            for name in ("stateReason", "duplicateOf", "createdAt")
            if name in schema.closed_event
        )
        or "__typename"
    )
    return (
        "query($owner:String!,$name:String!,$a:Int!,$b:Int!){repository(owner:$owner,name:$name){"
        + "".join(
            f"{alias}:issue(number:${alias}){{number state stateReason(enableDuplicate:true) "
            f"closed duplicateOf{{number}} projectItems(first:10){{nodes{{project{{number title}}}}}} "
            f"timelineItems(first:30){{nodes{{__typename "
            f"... on MarkedAsDuplicateEvent{{{marked}}} "
            f"... on ClosedEvent{{{closed}}} "
            f"... on CrossReferencedEvent{{source{{... on Issue{{number}}}}}}}}}}}}"
            for alias in ("a", "b")
        )
        + "}}"
    )


def read_back(
    client: Client, schema: Schema, survivor: IssueNumber, duplicate: IssueNumber
) -> dict[str, object]:
    variables: dict[str, object] = {
        "owner": client.owner,
        "name": client.name,
        "a": survivor,
        "b": duplicate,
    }
    data = client.query(readback_document(schema), variables, "readback")
    repository = expect_object(data.get("repository"), "the read-back's repository")
    for alias, number in (("a", survivor), ("b", duplicate)):
        issue = expect_object(
            repository.get(alias), f"issue #{number} (null: not found)"
        )
        if expect_int(issue.get("number"), f"issue #{number}'s number") != number:
            raise BoundaryError(f"the read-back answered another issue for #{number}")
        expect_str(issue.get("state"), f"issue #{number}'s state")
        expect_object(issue.get("timelineItems"), f"issue #{number}'s timeline")
    return data


def create(client: Client, role: Role, stamp: str) -> CreatedIssue:
    status, issue = client.rest(
        "POST",
        "/issues",
        {
            "title": f"onetaskgraph spike-store-defects {role} {stamp}",
            "body": "Throwaway issue for a duplicate-close measurement. Closed by the run "
            "that created it.",
        },
    )
    if status != 201:
        raise BoundaryError(f"creating the {role} answered {status}: {issue}")
    return CreatedIssue(
        role=role,
        number=IssueNumber(expect_int(issue.get("number"), "number")),
        node_id=NodeId(expect_str(issue.get("node_id"), "node_id")),
    )


def close_as_duplicate(
    client: Client, duplicate: CreatedIssue, survivor: CreatedIssue
) -> object:
    # `graphql` meters queries alone, so this one write to the scratch repository is sent
    # directly and charged GitHub's one-point minimum.
    client.limits.admit()
    client.limits.spent += 1
    client.mutations_sent += 1
    body = json.dumps(
        {
            "query": CLOSE,
            "variables": {"id": duplicate.node_id, "dup": survivor.node_id},
        }
    ).encode()
    response = send("POST", client.endpoint, client.token, body)
    client.records.append(
        RequestRecord(
            phase="closeIssue",
            at=time.time(),
            status=response.status,
            seconds=round(response.seconds, 3),
            headers=rate_headers(response.headers),
        )
    )
    answer = decode(response.body, "the closeIssue answer")
    if response.status != 200 or answer.get("errors"):
        raise BoundaryError(f"closeIssue answered {response.status}: {answer}")
    return answer


def landed(
    run: DuplicateCloseRun, survivor: CreatedIssue, duplicate: CreatedIssue
) -> str | None:
    """Why the run did not leave what it set out to, or ``None`` when it did."""
    data = expect_object(run.readback_final, "the final read-back")
    repository = expect_object(data.get("repository"), "repository")
    dup = expect_object(repository.get("b"), "the duplicate")
    if dup.get("stateReason") != "DUPLICATE":
        return f"#{duplicate.number} reads stateReason {dup.get('stateReason')!r}, not DUPLICATE"
    match dup.get("duplicateOf"):
        case {"number": number} if number == survivor.number:
            pass
        case other:
            return f"#{duplicate.number} reads duplicateOf {other!r}, not #{survivor.number}"
    still_open = [s.number for s in run.final_rest if s.state != "closed"]
    if still_open:
        return f"issues {still_open} are still open; close them by hand"
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--repository", required=True)
    parser.add_argument("--env-file")
    parser.add_argument("--endpoint", default=GRAPHQL_ENDPOINT)
    parser.add_argument("--stop-after-points", type=positive, default=30)
    parser.add_argument("--out", type=Path)
    parser.add_argument(
        "--readback-only",
        nargs=2,
        type=positive,
        metavar=("SURVIVOR", "DUPLICATE"),
        help="write nothing: read two issues back as step 4 does",
    )
    args = parser.parse_args()
    if args.out:
        try:
            check_out(args.out)
        except SpikeError as error:
            print(f"duplicate_close: {error}", file=sys.stderr)
            return 2
    if args.repository.lower() != SCRATCH:
        print(
            f"duplicate_close: refusing {args.repository}; only {SCRATCH} is written",
            file=sys.stderr,
        )
        return 2
    try:
        token = load_token(args.env_file)
    except SpikeError as error:
        print(f"duplicate_close: {error}", file=sys.stderr)
        return 2
    owner, _, name = args.repository.partition("/")
    client = Client(args.endpoint, owner, name, token, args.stop_after_points)
    run: DuplicateCloseRun | None = None
    try:
        schema = introspect(client)
        run = DuplicateCloseRun(repository=args.repository, schema=schema)
        if args.readback_only:
            survivor_number, duplicate_number = map(IssueNumber, args.readback_only)
            run.readback_final = read_back(
                client, schema, survivor_number, duplicate_number
            )
            run.final_rest = [
                client.rest_state(n) for n in (survivor_number, duplicate_number)
            ]
        else:
            if "duplicateIssueId" not in schema.close_input:
                raise BoundaryError(
                    "CloseIssueInput has no duplicateIssueId; nothing written"
                )
            stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
            survivor = create(client, Role.SURVIVOR, stamp)
            run.created.append(survivor)
            duplicate = create(client, Role.DUPLICATE, stamp)
            run.created.append(duplicate)
            run.close_answer = close_as_duplicate(client, duplicate, survivor)
            time.sleep(3)
            run.readback_after_close = read_back(
                client, schema, survivor.number, duplicate.number
            )
            status, closed = client.rest(
                "PATCH",
                f"/issues/{survivor.number}",
                {"state": "closed", "state_reason": "duplicate"},
            )
            run.rest_close = RestClose(
                status=status,
                state=optional_str(closed.get("state"), "state"),
                state_reason=optional_str(closed.get("state_reason"), "state_reason"),
                message=optional_str(closed.get("message"), "message"),
            )
            if status >= 300:
                # REST refused the reason: close the survivor anyway, so nothing is left open.
                client.rest(
                    "PATCH",
                    f"/issues/{survivor.number}",
                    {"state": "closed", "state_reason": "not_planned"},
                )
            time.sleep(3)
            run.readback_final = read_back(
                client, schema, survivor.number, duplicate.number
            )
            run.final_rest = [
                client.rest_state(n) for n in (survivor.number, duplicate.number)
            ]
            run.failure = landed(run, survivor, duplicate)
            if run.rest_close.status >= 300:
                run.failure = (
                    f"REST refused the comparison close ({run.rest_close.status}: "
                    f"{run.rest_close.message}); the survivor was closed as not planned"
                    + (f"; also {run.failure}" if run.failure else "")
                )
    except (BoundaryError, SpikeStop) as error:
        if run is None:
            print(f"duplicate_close: {error}", file=sys.stderr)
            return 1
        run.failure = str(error)
    run.points = client.limits.spent
    run.close_mutations_sent = client.mutations_sent
    run.records = client.records
    text = json.dumps(asdict(run), indent=2)
    print(text)
    if args.out:
        try:
            write_out(args.out, (text + "\n").encode())
        except SpikeError as error:
            print(f"duplicate_close: {error}", file=sys.stderr)
            return 2
    if run.failure:
        created = ", ".join(f"#{issue.number}" for issue in run.created) or "none"
        print(
            f"duplicate_close: {run.failure} (issues created: {created})",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
