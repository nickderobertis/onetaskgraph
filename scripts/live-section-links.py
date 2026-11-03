#!/usr/bin/env python3
"""Observe copied section links on the real trackers: run once by hand, never by a gate.

The credentialed check of README "Section links", by the manager's ruling on node
`onetaskgraph-section-links`: it drives the real binary (`target/debug/onetaskgraph`), within
the live lane's write rules, and reports what it observed as evidence. Nothing selects it;
`just` and Nx never run it.

What it does, in order:

1. Copies board #1 (`GH_PROJECTS_OWNER` / `GH_PROJECTS_NUMBER`) to a per-run board titled
   `onetaskgraph live board <stamp>`, as the live lane's sessions do, never writing to board
   #1 itself; its issues are filed in `GH_PROJECTS_REPOSITORY`, which must be
   `nickderobertis/onetaskgraph-live-scratch`.
2. Writes the authored project of the section-link journeys — a target task holding the
   spike's five headings, a task, a project description and a design document linking them —
   into a scratch folder, every title carrying `onetaskgraph live cleanup <stamp>`.
3. Copies it to that board, to Linear team `LINEAR_WRITE_TEAM` (which must be `TES2`), and
   then from the board onward to a second TES2 project.
4. Reads every stored body back through each tracker's own API: on GitHub the `<a id>`
   anchors and the `https://www.github.com/…#user-content-<slug>` links; on Linear each
   fragment's 8 hex characters against the editor state's stored id of the heading it names,
   and that the board → Linear hop names no GitHub issue and holds no anchor markup.
5. In a real Chromium, opens each GitHub section link directly and clicks it inside its issue,
   recording whether the named heading's anchor is what the page scrolled to.
6. Deletes everything it created — the issues, the board copy, the Linear issues, documents
   and projects — however it ended.

Run it from the repository root, after building the binary:

    uv run --no-project --with playwright --with pycrdt python -I scripts/live-section-links.py \\
        --out <directory> --linear-env <path to a .env holding LINEAR_API_KEY and LINEAR_WRITE_TEAM>

It prints one line per check and per observation, writes `observations.json` into `--out`,
and exits non-zero when any check failed.
"""

from __future__ import annotations

import argparse
import base64
import glob
import json
import os
import pathlib
import re
import socket
import subprocess
import sys
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
BINARY = ROOT / "target" / "debug" / "onetaskgraph"
PREFIX = "onetaskgraph live cleanup "
SCRATCH = "nickderobertis/onetaskgraph-live-scratch"
HEADINGS = [
    ("What", "what", "what"),
    ("Budgets", "budgets", "budgets"),
    ("Budget demo-quota-seconds", "budget-demo-quota-seconds", "budget-demo-quota-seconds"),
    ("Step 2.b: retry (once)", "step-2b-retry-once", "step-2b-retry-once"),
    ("Budgets", "budgets-1", "budgets"),
]
# Each link the authored bodies hold: its text, the body holding it, the record it names
# (None for its own) and the index of the heading it names there (OVERVIEW for `D`'s own).
OVERVIEW = ("Overview", "overview", "overview")
LINKS = [
    ("T", "the second budgets", None, 4),
    ("U", "the demo", "T", 2),
    ("U", "the first", "T", 1),
    ("P", "what it is", "T", 0),
    ("P", "the retry", "T", 3),
    ("D", "first", "T", 1),
    ("D", "second", "T", 4),
    ("D", "retry", "T", 3),
    ("D", "overview", None, "overview"),
]


def target_body() -> str:
    return (
        "## What\n\nThe target. Jump to [the second budgets](#budgets-1).\n\n"
        + "".join(f"Filler line {n} of what.\n\n" for n in range(30))
        + "## Budgets\n\nFirst budgets.\n\n"
        + "".join(f"Filler line {n} of the first budgets.\n\n" for n in range(30))
        + "### Budget demo-quota-seconds\n\nThe demo quota.\n\n"
        + "".join(f"Filler line {n} of the demo quota.\n\n" for n in range(30))
        + "### Step 2.b: retry (once)\n\nRetry once.\n\n"
        + "".join(f"Filler line {n} of the retry.\n\n" for n in range(30))
        + "## Budgets\n\nSecond budgets.\n\n"
        + "".join(f"Filler line {n} of the second budgets.\n\n" for n in range(30))
    )


class Checks:
    def __init__(self) -> None:
        self.results: list[dict] = []

    def check(self, what: str, held: bool, detail: str = "") -> None:
        self.results.append({"check": what, "held": held, "detail": detail})
        print(f"{'ok  ' if held else 'FAIL'} {what}{(': ' + detail) if detail and not held else ''}")

    def failed(self) -> bool:
        return any(not result["held"] for result in self.results)


def request(url: str, token: str, body: dict | None = None, method: str | None = None, auth: str = "bearer") -> dict:
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Content-Type": "application/json", "User-Agent": "onetaskgraph-section-links-live"}
    headers["Authorization"] = f"bearer {token}" if auth == "bearer" else token
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=60) as response:
        answer = json.load(response)
    if isinstance(answer, dict) and answer.get("errors"):
        raise RuntimeError(json.dumps(answer["errors"])[:500])
    return answer


def github(token: str, query: str, variables: dict) -> dict:
    return request("https://api.github.com/graphql", token, {"query": query, "variables": variables})["data"]


def linear(key: str, query: str, variables: dict) -> dict:
    return request("https://api.linear.app/graphql", key, {"query": query, "variables": variables}, auth="raw")["data"]


def headings_of(state: str | None) -> list[tuple[str, str]]:
    import pycrdt

    if not state:
        return []
    doc = pycrdt.Doc()
    doc.apply_update(base64.b64decode(state))
    found: list[tuple[str, str]] = []

    def walk(node):
        if isinstance(node, pycrdt.XmlElement):
            if node.tag == "heading":
                text = "".join(piece[0] for child in node.children if isinstance(child, pycrdt.XmlText) for piece in child.diff())
                found.append((text.strip(), node.attributes["id"]))
            for child in node.children:
                walk(child)

    for child in doc.get("prosemirror", type=pycrdt.XmlFragment).children:
        walk(child)
    return found


def env_file(path: str) -> dict[str, str]:
    values = {}
    for line in pathlib.Path(path).read_text().splitlines():
        match = re.match(r"^\s*(?:export\s+)?([A-Z_][A-Z0-9_]*)=(.*)$", line)
        if match:
            values[match.group(1)] = match.group(2).strip().strip("'\"")
    return values


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--linear-env", required=True)
    args = parser.parse_args()
    out = pathlib.Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    token = os.environ["GH_PROJECTS_TOKEN"]
    owner = os.environ["GH_PROJECTS_OWNER"]
    number = int(os.environ["GH_PROJECTS_NUMBER"])
    repository = os.environ["GH_PROJECTS_REPOSITORY"]
    if repository.lower() != SCRATCH:
        sys.exit(f"GH_PROJECTS_REPOSITORY must be {SCRATCH}, not {repository}")
    linear_env = env_file(args.linear_env)
    key, team = linear_env["LINEAR_API_KEY"], linear_env["LINEAR_WRITE_TEAM"]
    if team != "TES2":
        sys.exit(f"LINEAR_WRITE_TEAM must be TES2, not {team}")
    stamp = f"{socket.gethostname()}-{os.getpid()}-{int(time.time() * 1e6)}"
    checks = Checks()
    created_issues: list[str] = []
    linear_created: list[tuple[str, str]] = []
    board_copy: str | None = None
    observations: dict = {"stamp": stamp, "checks": checks.results}

    try:
        # 1. This run's own copy of board #1.
        held = github(token, "query($o:String!,$n:Int!){ repositoryOwner(login:$o){ id ... on User{ projectV2(number:$n){ id } } ... on Organization{ projectV2(number:$n){ id } } } }", {"o": owner, "n": number})["repositoryOwner"]
        copied = github(token, "mutation($p:ID!,$o:ID!,$t:String!){ copyProjectV2(input:{projectId:$p,ownerId:$o,title:$t,includeDraftIssues:false}){ projectV2{ id number url } } }", {"p": held["projectV2"]["id"], "o": held["id"], "t": f"onetaskgraph live board {stamp}"})["copyProjectV2"]["projectV2"]
        board_copy = copied["id"]
        options = github(token, "query($id:ID!){ node(id:$id){ ... on ProjectV2{ field(name:\"Status\"){ ... on ProjectV2SingleSelectField{ options{ name } } } } } }", {"id": board_copy})["node"]["field"]["options"]
        observations["board"] = copied["url"]
        # Linear's names for the two categories this copy writes.
        vocabulary = linear(key, "query($k:String!){ teams(filter:{key:{eq:$k}}){ nodes{ states{ nodes{ name type } } } } projectStatuses{ nodes{ name type } } }", {"k": team})
        states = vocabulary["teams"]["nodes"][0]["states"]["nodes"]
        open_state = next(state["name"] for state in states if state["type"] == "unstarted")
        done_state = next(state["name"] for state in states if state["type"] == "completed")
        project_status = next(status["name"] for status in vocabulary["projectStatuses"]["nodes"] if status["type"] in ("planned", "backlog"))

        # 2. The authored project.
        sandbox = out / "sandbox"
        authored = sandbox / "authored"
        for folder in ("projects", "tasks", "documents"):
            (authored / folder).mkdir(parents=True, exist_ok=True)
        target = (authored / "tasks" / "T.md").resolve()
        title = lambda what: f"{PREFIX}{stamp} {what}"  # noqa: E731
        files = {
            "projects/P.md": (f"title: {title('Section plan')}\nstatus: todo", "The plan: [what it is](../tasks/T.md#what) and [the retry](../tasks/T.md#step-2b-retry-once).\n"),
            "tasks/T.md": (f"title: {title('Target')}\nstatus: todo\nproject: P", target_body()),
            "tasks/U.md": (f"title: {title('Referrer')}\nstatus: todo\nproject: P", f"See [the demo](T.md#budget-demo-quota-seconds) and [the first]({target}#budgets).\n"),
            "documents/D.md": (f"title: {title('Design')}\nproject: P", f"## Overview\n\n| Budget | Detail |\n| --- | --- |\n| first | [first](../tasks/T.md#budgets) |\n| second | [second](../tasks/T.md#budgets-1) |\n| retry | [retry]({target}#step-2b-retry-once) |\n\nBack to the [overview](#overview).\n"),
        }
        for path, (front, body) in files.items():
            (authored / path).write_text(f"---\n{front}\n---\n{body}")
        (sandbox / "config").mkdir(exist_ok=True)
        (sandbox / "onetaskgraph.yaml").write_text(json.dumps({"sources": {
            "authored": {"plugin": "local-md", "config": {"root": str(authored)}},
            "board": {"plugin": "github-projects", "config": {"owner": owner, "project_number": copied["number"], "repository": repository, "status_mapping": {"todo": options[0]["name"], "backlog": None, "queued": None, "in-progress": None}}},
            "linear": {"plugin": "linear", "config": {"team": team, "status_mapping": {"todo": {"task": open_state, "project": project_status}, "done": {"task": done_state}}}},
        }}))
        environment = {name: value for name, value in os.environ.items() if not name.startswith("ONETASKGRAPH_")}
        environment.update({"XDG_CONFIG_HOME": str(sandbox / "config"), "LINEAR_API_KEY": key, "GH_PROJECTS_TOKEN": token})

        def run(*arguments: str) -> dict:
            done = subprocess.run([str(BINARY), "--json", *arguments], cwd=sandbox, env=environment, capture_output=True, text=True)
            if done.returncode != 0:
                raise RuntimeError(f"onetaskgraph {' '.join(arguments)} exited {done.returncode}: {done.stderr}")
            return json.loads(done.stdout)

        def shown(verb: str, id: str) -> dict:
            return run(verb, "show", id)["items"][0]["item"]

        def copy(source: dict[str, tuple[str, str]], to: str) -> dict[str, tuple[str, str]]:
            project = run("project", "copy", source["P"][1], "--to", to)
            document = run("document", "copy", source["D"][1], "--to", to)
            landed = {}
            for key_, (verb, id) in source.items():
                report = document if verb == "document" else project
                outcome = next(item for item in report["items"] if item["source"] == id)
                landed[key_] = (verb, outcome["destination"])
                if to == "board":
                    created_issues.append(shown(verb, outcome["destination"])["location"]["url"])
                else:
                    linear_created.append((verb, outcome["destination"].split(":", 1)[1]))
            return landed

        source = {"P": ("project", "authored:P"), "T": ("task", "authored:T"), "U": ("task", "authored:U"), "D": ("document", "authored:D")}
        # 3. The three hops.
        on_board = copy(source, "board")
        in_linear = copy(source, "linear")
        onward = copy(on_board, "linear")

        # 4a. GitHub: the bodies as GitHub stores them.
        urls = {key_: shown(verb, id)["location"]["url"] for key_, (verb, id) in on_board.items()}
        raw = {key_: request(url.replace("https://github.com/", "https://api.github.com/repos/"), token)["body"] for key_, url in urls.items()}
        observations["github"] = {"issues": urls}
        slugs = [github_slug for _, github_slug, _ in HEADINGS]
        for slug in slugs:
            checks.check(f"GitHub: the target issue carries <a id=\"{slug}\"></a> before its heading", f'<a id="{slug}"></a>\n#' in raw["T"])
        checks.check("GitHub: the design issue carries <a id=\"overview\"></a>", '<a id="overview"></a>\n## Overview' in raw["D"])
        hrefs = {}
        for holder, text, named, index in LINKS:
            slug = "overview" if index == "overview" else slugs[index]
            expected = f"#user-content-{slug}" if named is None else urls[named].replace("https://github.com/", "https://www.github.com/") + f"#user-content-{slug}"
            checks.check(f"GitHub: {holder}'s [{text}] is {expected}", f"[{text}]({expected})" in raw[holder], raw[holder][:300])
            hrefs[(holder, text)] = (urls[holder] + expected) if named is None else expected

        # 4b. Linear: each fragment's id against the stored editor state.
        def linear_record(verb: str, id: str) -> dict:
            if verb == "task":
                record = linear(key, "query($id:String!){ issue(id:$id){ url description descriptionState } }", {"id": id})["issue"]
                return {"url": record["url"], "body": record["description"] or "", "state": record["descriptionState"]}
            field = "project" if verb == "project" else "document"
            record = linear(key, f"query($id:String!){{ {field}(id:$id){{ url content contentState }} }}", {"id": id})[field]
            return {"url": record["url"], "body": record["content"] or "", "state": record["contentState"]}

        observations["linear"] = {}
        for hop, landed in (("local-md -> TES2", in_linear), ("board -> TES2", onward)):
            records = {key_: linear_record(verb, id.split(":", 1)[1]) for key_, (verb, id) in landed.items()}
            observations["linear"][hop] = {key_: record["url"] for key_, record in records.items()}
            for key_, record in records.items():
                checks.check(f"Linear {hop}: {key_} holds no anchor markup", "<a " not in record["body"])
                if hop.startswith("board"):
                    checks.check(f"Linear {hop}: {key_} names no GitHub issue", "github.com" not in record["body"], record["body"][:300])
            for holder, text, named, index in LINKS:
                at = records[named or holder]
                heading = OVERVIEW if index == "overview" else HEADINGS[index]
                ordinal = 1 if index == 4 else 0
                stored = [hid for htext, hid in headings_of(at["state"]) if htext == heading[0]]
                match = re.search(r"\[" + re.escape(text) + r"\]\(([^)]*)\)", records[holder]["body"])
                # Linear stores every link destination in angle brackets.
                written = match.group(1).removeprefix("<").removesuffix(">") if match else ""
                id8 = stored[ordinal].replace("-", "")[:8] if len(stored) > ordinal else None
                expected = f"{at['url']}#heading-{heading[2]}-{id8}"
                checks.check(f"Linear {hop}: {holder}'s [{text}] names the stored id of {heading[0]} #{ordinal}", id8 is not None and written == expected, f"wrote {written!r}, stored id8 {id8}")

        # 5. Chromium: each GitHub section link opened directly and clicked inside its issue.
        observations["browser"] = browse(checks, urls, hrefs)
    finally:
        # 6. Remove what this run created, whatever happened.
        for url in created_issues:
            try:
                node = request(url.replace("https://github.com/", "https://api.github.com/repos/"), token)["node_id"]
                github(token, "mutation($id:ID!){ deleteIssue(input:{issueId:$id}){ clientMutationId } }", {"id": node})
            except Exception as error:  # noqa: BLE001 — reported, never swallowed
                print(f"CLEANUP could not delete {url}: {error}")
        if board_copy:
            try:
                github(token, "mutation($id:ID!){ deleteProjectV2(input:{projectId:$id}){ clientMutationId } }", {"id": board_copy})
            except Exception as error:  # noqa: BLE001
                print(f"CLEANUP could not delete the board copy {board_copy}: {error}")
        for verb, id in linear_created:
            mutation = {"task": "issueDelete", "document": "documentDelete", "project": "projectDelete"}[verb]
            try:
                linear(key, f"mutation($id:String!){{ {mutation}(id:$id){{ success }} }}", {"id": id})
            except Exception as error:  # noqa: BLE001
                print(f"CLEANUP could not delete Linear {verb} {id}: {error}")
        (out / "observations.json").write_text(json.dumps(observations, indent=2) + "\n")
        print(f"observations: {out / 'observations.json'}")
    return 1 if checks.failed() else 0


MEASURE = """(slug) => {
  const anchor = document.getElementById('user-content-' + slug);
  return {url: location.href, scrollY: Math.round(scrollY),
          anchorTop: anchor ? Math.round(anchor.getBoundingClientRect().top) : null};
}"""


def landed(seen: dict) -> bool:
    return seen["anchorTop"] is not None and seen["scrollY"] > 40 and -80 <= seen["anchorTop"] <= 250


def browse(checks: Checks, urls: dict, hrefs: dict) -> list[dict]:
    from playwright.sync_api import sync_playwright

    executable = sorted(glob.glob(str(pathlib.Path.home() / ".cache/ms-playwright/chromium-*/chrome-linux*/chrome")))[-1]
    results = []
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(executable_path=executable)
        context = browser.new_context(viewport={"width": 1280, "height": 800})
        for (holder, text), href in hrefs.items():
            slug = href.rsplit("#user-content-", 1)[1]
            entry = {"link": f"{holder}: [{text}]", "href": href, "heading": slug}
            page = context.new_page()
            try:
                page.goto(href, wait_until="load", timeout=60000)
                time.sleep(4)
                entry["direct"] = page.evaluate(MEASURE, slug)
            finally:
                page.close()
            page = context.new_page()
            try:
                page.goto(urls[holder], wait_until="load", timeout=60000)
                time.sleep(4)
                link = page.locator(".markdown-body a", has_text=re.compile("^" + re.escape(text) + "$")).first
                link.scroll_into_view_if_needed(timeout=15000)
                before = set(context.pages)
                link.click()
                time.sleep(5)
                opened = [candidate for candidate in context.pages if candidate not in before]
                target = opened[0] if opened else page
                entry["click"] = target.evaluate(MEASURE, slug)
                if target is not page:
                    target.close()
            finally:
                page.close()
            for mode in ("direct", "click"):
                checks.check(f"Chromium {mode}: {entry['link']} lands on #user-content-{slug}", landed(entry[mode]), json.dumps(entry[mode]))
            results.append(entry)
        browser.close()
    return results


if __name__ == "__main__":
    sys.exit(main())
