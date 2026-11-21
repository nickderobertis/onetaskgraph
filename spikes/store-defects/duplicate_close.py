#!/usr/bin/env python3
"""Close one throwaway issue as a duplicate of another, read back what GitHub kept, and close both.

    python3 spikes/store-defects/duplicate_close.py --repository OWNER/NAME \
        [--env-file PATH] [--max-points 30] [--out FILE]

Refuses any repository but the scratch one. Steps:

1. introspect ``CloseIssueInput``, ``ClosedEvent``, ``MarkedAsDuplicateEvent`` and ``Issue``'s
   duplicate fields, so nothing below names a field the schema lacks;
2. create the survivor and the duplicate (REST);
3. ``closeIssue(input:{issueId, stateReason: DUPLICATE, duplicateIssueId})`` on the duplicate;
4. read both back: ``state``, ``stateReason``, each timeline, the REST ``state_reason``;
5. close the survivor by REST ``PATCH state=closed, state_reason=duplicate`` — the comparison —
   and read it back the same way, so nothing is left open.

Prints one JSON document; ``--out`` writes it.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ghlive import API, Recorder, SpikeLimits, graphql, load_token, rate_headers, send  # noqa: E402

SCRATCH = "nickderobertis/onetaskgraph-live-scratch"
INTROSPECT = """query{
  closeInput:__type(name:"CloseIssueInput"){inputFields{name type{name kind ofType{name}}}}
  closedEvent:__type(name:"ClosedEvent"){fields{name}}
  marked:__type(name:"MarkedAsDuplicateEvent"){fields{name}}
  reasons:__type(name:"IssueClosedStateReason"){enumValues{name}}
  items:__type(name:"IssueTimelineItemsItemType"){enumValues{name}}
}"""
# GitHub allows `__Type.fields` twice per document, so `Issue`'s are asked for on their own.
INTROSPECT_ISSUE = 'query{issue:__type(name:"Issue"){fields{name}}}'


def rest(method: str, path: str, token: str, payload: dict | None = None) -> tuple[int, dict]:
    body = json.dumps(payload).encode() if payload is not None else None
    status, headers, data, _seconds = send(method, API + path, token, body)
    answer = json.loads(data or b"{}")
    return status, {"body": answer, "headers": rate_headers(headers)}


def readback_document(marked_fields: set[str], closed_fields: set[str]) -> str:
    marked = " ".join(
        f"{name}{{... on Issue{{number}} ... on PullRequest{{number}}}}"
        if name in {"canonical", "duplicate"}
        else name
        for name in ("canonical", "duplicate", "isCrossRepository", "createdAt")
        if name in marked_fields
    )
    closed = " ".join(
        f"{name}{{... on Issue{{number}} ... on PullRequest{{number}}}}"
        if name == "duplicateOf"
        else name
        for name in ("stateReason", "duplicateOf", "createdAt")
        if name in closed_fields
    )
    marked = marked or "__typename"
    closed = closed or "__typename"
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", required=True)
    parser.add_argument("--env-file")
    parser.add_argument("--max-points", type=int, default=30)
    parser.add_argument("--out")
    parser.add_argument(
        "--readback-only",
        nargs=2,
        type=int,
        metavar=("SURVIVOR", "DUPLICATE"),
        help="write nothing: read two issues back as step 4 does",
    )
    args = parser.parse_args()
    if args.repository.lower() != SCRATCH:
        raise SystemExit(f"refusing {args.repository}: this spike writes only to {SCRATCH}")
    owner, _, name = args.repository.partition("/")
    token = load_token(args.env_file)
    limits = SpikeLimits(max_points=args.max_points)
    recorder = Recorder()
    out: dict = {"repository": args.repository, "steps": {}}

    schema, _ = graphql(INTROSPECT, {}, token, limits, recorder, "introspect")
    if "data" not in schema:
        raise SystemExit(f"introspection failed, nothing written: {schema.get('errors')}")
    data = schema["data"]
    issue_schema, _ = graphql(INTROSPECT_ISSUE, {}, token, limits, recorder, "introspect")
    data["issue"] = issue_schema["data"]["issue"]
    out["schema"] = {
        "CloseIssueInput": [f["name"] for f in data["closeInput"]["inputFields"]],
        "ClosedEvent": sorted(f["name"] for f in data["closedEvent"]["fields"]),
        "MarkedAsDuplicateEvent": sorted(f["name"] for f in (data["marked"] or {}).get("fields", [])),
        "Issue_duplicate_fields": sorted(
            f["name"] for f in data["issue"]["fields"] if "uplicate" in f["name"]
        ),
        "IssueClosedStateReason": [v["name"] for v in data["reasons"]["enumValues"]],
        "timeline_duplicate_types": [
            v["name"] for v in data["items"]["enumValues"] if "DUPLICATE" in v["name"]
        ],
    }
    if args.readback_only:
        survivor, duplicate = args.readback_only
        document = readback_document(
            set(out["schema"]["MarkedAsDuplicateEvent"]), set(out["schema"]["ClosedEvent"])
        )
        variables = {"owner": owner, "name": name, "a": survivor, "b": duplicate}
        answer, _ = graphql(document, variables, token, limits, recorder, "readback")
        out["readback"] = answer
        out["rest"] = {
            number: {
                k: rest("GET", f"/repos/{owner}/{name}/issues/{number}", token)[1]["body"].get(k)
                for k in ("number", "state", "state_reason", "html_url")
            }
            for number in (survivor, duplicate)
        }
        out["points"] = limits.spent
        out["records"] = recorder.records
        text = json.dumps(out, indent=2)
        print(text)
        if args.out:
            Path(args.out).write_text(text + "\n", encoding="utf-8")
        return 0
    if "duplicateIssueId" not in out["schema"]["CloseIssueInput"]:
        out["stopped"] = "CloseIssueInput has no duplicateIssueId; nothing written"
        print(json.dumps(out, indent=2))
        return 1

    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    created = {}
    for role in ("survivor", "duplicate"):
        status, answer = rest(
            "POST",
            f"/repos/{owner}/{name}/issues",
            token,
            {
                "title": f"onetaskgraph spike-store-defects {role} {stamp}",
                "body": "Throwaway issue for a duplicate-close measurement. Closed by the run "
                "that created it.",
            },
        )
        if status != 201:
            out["stopped"] = f"creating the {role} answered {status}: {answer['body']}"
            print(json.dumps(out, indent=2))
            return 1
        created[role] = {"number": answer["body"]["number"], "node_id": answer["body"]["node_id"]}
    out["created"] = created

    close = """mutation($id:ID!,$dup:ID!){closeIssue(input:{issueId:$id,stateReason:DUPLICATE,
      duplicateIssueId:$dup}){issue{number state stateReason(enableDuplicate:true)}}}"""
    # `graphql` refuses mutations by design (the board measurement must never write); this
    # one write is to the scratch repository, so it is sent here directly.
    limits.admit()
    status, headers, payload, seconds = send(
        "POST",
        f"{API}/graphql",
        token,
        json.dumps(
            {
                "query": close,
                "variables": {
                    "id": created["duplicate"]["node_id"],
                    "dup": created["survivor"]["node_id"],
                },
            }
        ).encode(),
    )
    out["steps"]["graphql_close_duplicate"] = {
        "status": status,
        "answer": json.loads(payload),
        "headers": rate_headers(headers),
    }

    document = readback_document(
        set(out["schema"]["MarkedAsDuplicateEvent"]), set(out["schema"]["ClosedEvent"])
    )
    variables = {
        "owner": owner,
        "name": name,
        "a": created["survivor"]["number"],
        "b": created["duplicate"]["number"],
    }
    time.sleep(3)
    answer, _ = graphql(document, variables, token, limits, recorder, "readback-1")
    out["steps"]["readback_after_graphql_close"] = answer
    out["steps"]["rest_duplicate_after_graphql_close"] = {
        k: rest("GET", f"/repos/{owner}/{name}/issues/{created['duplicate']['number']}", token)[
            1
        ]["body"].get(k)
        for k in ("state", "state_reason")
    }

    status, answer_rest = rest(
        "PATCH",
        f"/repos/{owner}/{name}/issues/{created['survivor']['number']}",
        token,
        {"state": "closed", "state_reason": "duplicate"},
    )
    out["steps"]["rest_close_survivor_duplicate"] = {
        "status": status,
        "state": answer_rest["body"].get("state"),
        "state_reason": answer_rest["body"].get("state_reason"),
        "message": answer_rest["body"].get("message"),
        "errors": answer_rest["body"].get("errors"),
    }
    if status >= 300:
        # REST refused the reason: close the survivor anyway, so nothing is left open.
        status, answer_rest = rest(
            "PATCH",
            f"/repos/{owner}/{name}/issues/{created['survivor']['number']}",
            token,
            {"state": "closed", "state_reason": "not_planned"},
        )
        out["steps"]["rest_close_survivor_fallback"] = {
            "status": status,
            "state": answer_rest["body"].get("state"),
            "state_reason": answer_rest["body"].get("state_reason"),
        }
    time.sleep(3)
    answer, _ = graphql(document, variables, token, limits, recorder, "readback-2")
    out["steps"]["readback_final"] = answer
    out["final_rest"] = {
        role: {
            k: rest("GET", f"/repos/{owner}/{name}/issues/{info['number']}", token)[1]["body"].get(
                k
            )
            for k in ("number", "state", "state_reason", "html_url")
        }
        for role, info in created.items()
    }
    out["points"] = limits.spent + 1  # the close mutation's cost is not reported; GitHub's floor is 1
    out["records"] = recorder.records
    text = json.dumps(out, indent=2)
    print(text)
    if args.out:
        Path(args.out).write_text(text + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
