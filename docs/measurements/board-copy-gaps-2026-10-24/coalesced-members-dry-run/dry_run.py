# llmlint: ignore-file[new_code_lands_in_a_project] This is the measurement record beside the costs it produced, not code any project runs: the manager ran it once, unchanged, to produce `costs.json`, and it is kept verbatim as that file's provenance, like every other run under docs/measurements; no target selects or runs it.
# llmlint: ignore-file[async_typed_clients_at_boundaries] Three sequential `gh` dry-run requests made once by hand to produce `costs.json`, kept verbatim as its provenance; rewriting the client now would make this file no longer the script that measured.
# llmlint: ignore-file[modern_domain_modeling] The variables are GraphQL variable maps sent as JSON exactly as the plugin binds them, and this is the one-off measurement script kept verbatim as `costs.json`'s provenance; typing them would change the record of what was run.
"""Price three read shapes with GitHub's own dry run, spending nothing and evaluating nothing.

The shapes, beside each other: the batched origin lookup a document copy sends today, the
listing of its destination project's members it sends today while the boundary screens, and the
two coalesced into one request with the members trimmed to what classification reads (id,
parent, body, repository; no nested connection). Each document is the plugin's own text, read
from `documents.json` (dumped from `onetaskgraph_github_projects::graphql` at this commit), with
`rateLimit(dryRun: true) { cost }` added to its selection; every variable is bound as the plugin
binds it, against the public test board (owner `nickderobertis`, project 1) and its public
scratch repository only.

    python3 docs/measurements/board-copy-gaps-2026-10-24/coalesced-members-dry-run/dry_run.py

writes `costs.json` beside this file. It needs `gh` logged in with a credential that reads the
board; it reads nothing private and writes nothing on GitHub.
"""

import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
DOCUMENTS = json.loads((HERE / "documents.json").read_text(encoding="utf-8"))
OWNER, NUMBER, REPOSITORY = "nickderobertis", 1, "onetaskgraph-live-scratch"
# The plugin's page sizes: `NESTED_PAGE_SIZE`, `BOARD_ITEMS_PAGE_SIZE`,
# `ORIGIN_LOOKUPS_PAGE_SIZE` and `MEMBER_PAGE_SIZE`.
NESTED, BOARD_ITEMS, LOOKUP_FIRST, MEMBERS = 50, 3, 1, 23
# What the trimmed members page selects of each member: what classifying it reads.
TRIMMED = (
    "members:node(id:$project){... on Issue{subIssues(first:$memberFirst)"
    "{pageInfo{hasNextPage endCursor} nodes{id body parent{id} repository{nameWithOwner}}}}}"
)


def priced(document: str) -> str:
    """`document` with GitHub's dry-run price selected beside its own top-level fields."""
    opened = document.index("{")
    return document[: opened + 1] + "rateLimit(dryRun:true){cost} " + document[opened + 1 :]


def coalesced(document: str) -> str:
    """The batched origin lookup with the trimmed members page of `$project` beside it."""
    head, body = document.split("){", 1)
    return head + ",$project:ID!,$memberFirst:Int!){" + TRIMMED + " " + body


def lookup_variables() -> dict:
    """The batched lookup's variables as a document copy binds them: its own origin and its
    project's, the other eight slots unsent."""
    variables = {
        "owner": OWNER, "number": NUMBER, "type": "ISSUE", "lookupFirst": LOOKUP_FIRST,
        "nestedFirst": NESTED, "boardItems": BOARD_ITEMS, "duplicates": True,
        "repositoryOwner": OWNER, "repositoryName": REPOSITORY,
    }
    for slot, origin in enumerate(["notes:D", "notes:P"] + ["notes:D"] * 8):
        variables[f"filter{slot}"] = f'onetaskgraph.origin:"{origin}"'
        variables[f"search{slot}"] = f'project:{OWNER}/{NUMBER} in:body "{origin}"'
        variables[f"ask{slot}"] = slot < 2
    return variables


def cost(document: str, variables: dict) -> int:
    request = json.dumps({"query": priced(document), "variables": variables})
    answered = subprocess.run(
        ["gh", "api", "graphql", "--input", "-"], input=request, capture_output=True,
        text=True, encoding="utf-8", check=False,
    )
    if answered.returncode != 0:
        sys.exit(f"GitHub refused the dry run: {answered.stdout}{answered.stderr}")
    try:
        price = json.loads(answered.stdout)["data"]["rateLimit"]["cost"]
    except (ValueError, KeyError, TypeError):
        sys.exit(f"GitHub's dry run carried no rateLimit.cost: {answered.stdout}")
    # A bool is an int to Python, and is no price.
    if type(price) is not int or price < 0:
        sys.exit(f"GitHub's dry run priced the document {price!r}, not a nonnegative integer")
    return price


def main() -> None:
    answered = subprocess.run(
        ["gh", "api", "graphql", "-f",
         f'query=query{{repository(owner:"{OWNER}",name:"{REPOSITORY}"){{issues(first:1){{nodes{{id}}}}}}}}'],
        capture_output=True, text=True, encoding="utf-8", check=True,
    ).stdout
    try:
        issue = json.loads(answered)["data"]["repository"]["issues"]["nodes"][0]["id"]
    except (ValueError, KeyError, IndexError, TypeError):
        sys.exit(f"{OWNER}/{REPOSITORY} answered no issue to bind the members page to: {answered}")
    if not isinstance(issue, str) or not issue.strip():
        sys.exit(f"{OWNER}/{REPOSITORY}'s first issue carried the id {issue!r}, not a node id")
    members = {"id": issue, "memberFirst": MEMBERS, "after": None, "nestedFirst": NESTED,
               "boardItems": BOARD_ITEMS, "duplicates": True}
    shapes = {
        "separate origin lookup (ORIGIN_LOOKUPS)": (DOCUMENTS["ORIGIN_LOOKUPS"], lookup_variables()),
        "separate members listing (SUB_ISSUES)": (DOCUMENTS["SUB_ISSUES"], members),
        "coalesced lookup with trimmed members page": (
            coalesced(DOCUMENTS["ORIGIN_LOOKUPS"]),
            {**lookup_variables(), "project": issue, "memberFirst": MEMBERS},
        ),
    }
    costs = {name: cost(document, variables) for name, (document, variables) in shapes.items()}
    record = {"board": f"{OWNER}/projects/{NUMBER}", "repository": f"{OWNER}/{REPOSITORY}",
              "member page": MEMBERS, "lookup page": LOOKUP_FIRST, "costs": costs,
              "coalesced document": priced(coalesced(DOCUMENTS["ORIGIN_LOOKUPS"]))}
    (HERE / "costs.json").write_text(json.dumps(record, indent=1) + "\n", encoding="utf-8")
    print(json.dumps(costs, indent=1))


if __name__ == "__main__":
    main()
