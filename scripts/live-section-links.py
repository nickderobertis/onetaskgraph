#!/usr/bin/env python3
"""Observe copied section links on the real trackers: run once by hand, never by a gate.

The credentialed check of README "Section links", by the manager's ruling on node
`onetaskgraph-section-links`: it drives the real binary (`target/debug/onetaskgraph`) within the
live lane's write rules and reports what it observed as evidence. Nothing selects it; `just` and
Nx never run it.

1. Copies board #1 (`GH_PROJECTS_OWNER` / `GH_PROJECTS_NUMBER`) to a per-run board titled
   `onetaskgraph live board <stamp>`, never writing to board #1 itself; its issues are filed in
   `GH_PROJECTS_REPOSITORY`, which must be `nickderobertis/onetaskgraph-live-scratch`.
2. Writes the authored project of the section-link journeys — a target task holding the spike's
   five headings, a task, a project description and a design document linking them — into a
   scratch folder, under a source name of this run's own, every title carrying
   `onetaskgraph live cleanup <stamp>`.
3. Copies it to that board, to Linear team `LINEAR_WRITE_TEAM` (which must be `TES2`), and from
   the board onward to a second TES2 project.
4. Reads every stored body back through each tracker's own API: on GitHub the `<a id>` anchors
   and the `https://www.github.com/…#user-content-<slug>` links; on Linear each fragment's 8 hex
   characters against the stored editor-state id of the heading it names, and that the
   board → Linear hop names no GitHub issue and holds no anchor markup.
5. In a real Chromium, opens each GitHub section link directly and clicks it inside its issue,
   recording whether the named heading's anchor is what the page scrolled to.
6. Deletes everything it created — the issues, the board copy, the Linear records — however it
   ended, and names on stderr anything it could not.

Run from the repository root, after building the binary:

    uv run --no-project --with playwright --with pycrdt python -I scripts/live-section-links.py \\
        --out <directory> --linear-env <a .env holding LINEAR_API_KEY and LINEAR_WRITE_TEAM>

Quiet on success but for one summary line; each failed check goes to stderr. Writes
`observations.json` into `--out`, and exits non-zero when a check failed or anything it created
could not be removed.
"""

# llmlint: ignore-file[async_typed_clients_at_boundaries] A hand-run, strictly sequential observation: every step consumes the previous step's answer — the board copy before the copies, the copies before the reads, the reads before the browser — so an async client would add an event loop and nothing could run concurrently; the payloads are mapped into the dataclasses below as soon as they arrive.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] The forms are restated here on purpose: this script is the independent observer the README's section forms are held to on the real trackers, and deriving them from `SectionForm` would make it compare the engine with itself. The locally gated drift check is the section-link journeys in `crates/onetaskgraph-e2e/tests/e2e/section_links.rs`, which restate the same forms literally.

from __future__ import annotations

import argparse
import base64
import dataclasses
import glob
import json
import os
import pathlib
import re
import socket
import subprocess
import sys
import time
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
BINARY = ROOT / "target" / "debug" / "onetaskgraph"
PREFIX = "onetaskgraph live cleanup "
SCRATCH = "nickderobertis/onetaskgraph-live-scratch"
GITHUB_ISSUES = f"https://github.com/{SCRATCH}/issues/"
LINEAR_HOST = "linear.app"


@dataclasses.dataclass(frozen=True)
class Heading:
    """One heading of the target: its text, GitHub's (the authored) slug, Linear's slug."""

    text: str
    github: str
    linear: str
    ordinal: int = 0


TARGET_HEADINGS = [
    Heading("What", "what", "what"),
    Heading("Budgets", "budgets", "budgets"),
    Heading("Budget demo-quota-seconds", "budget-demo-quota-seconds", "budget-demo-quota-seconds"),
    Heading("Step 2.b: retry (once)", "step-2b-retry-once", "step-2b-retry-once"),
    Heading("Budgets", "budgets-1", "budgets", ordinal=1),
]
OVERVIEW = Heading("Overview", "overview", "overview")


@dataclasses.dataclass(frozen=True)
class Link:
    """One section link the authored bodies hold: the body holding it, its text, the record it
    names — `None` for its own — and the heading it names there."""

    holder: str
    text: str
    names: str | None
    heading: Heading


LINKS = [
    Link("T", "the second budgets", None, TARGET_HEADINGS[4]),
    Link("U", "the demo", "T", TARGET_HEADINGS[2]),
    Link("U", "the first", "T", TARGET_HEADINGS[1]),
    Link("P", "what it is", "T", TARGET_HEADINGS[0]),
    Link("P", "the retry", "T", TARGET_HEADINGS[3]),
    Link("D", "first", "T", TARGET_HEADINGS[1]),
    Link("D", "second", "T", TARGET_HEADINGS[4]),
    Link("D", "retry", "T", TARGET_HEADINGS[3]),
    Link("D", "overview", None, OVERVIEW),
]


@dataclasses.dataclass(frozen=True)
class Record:
    """One record of the copied project, by its key, its verb and its qualified id."""

    key: str
    verb: str
    id: str

    @property
    def native(self) -> str:
        return self.id.split(":", 1)[1]


@dataclasses.dataclass(frozen=True)
class Stored:
    """One Linear record as Linear stores it."""

    url: str
    body: str
    state: str | None


@dataclasses.dataclass
class Check:
    what: str
    held: bool
    detail: str = ""


@dataclasses.dataclass
class Observation:
    """One GitHub section link, opened directly and clicked inside its issue."""

    link: str
    href: str
    slug: str
    direct: dict = dataclasses.field(default_factory=dict)
    click: dict = dataclasses.field(default_factory=dict)


@dataclasses.dataclass
class Run:
    """What one run did, created and saw — written out as `observations.json`."""

    stamp: str
    checks: list[Check] = dataclasses.field(default_factory=list)
    board: str | None = None
    github: dict[str, str] = dataclasses.field(default_factory=dict)
    linear: dict[str, dict[str, str]] = dataclasses.field(default_factory=dict)
    browser: list[Observation] = dataclasses.field(default_factory=list)
    cleanup_failures: list[str] = dataclasses.field(default_factory=list)

    def check(self, what: str, held: bool, detail: str = "") -> None:
        self.checks.append(Check(what, held, detail))
        if not held:
            print(
                f"FAIL {what}: {detail}\n  next: read observations.json, then compare the stored "
                "body with README \"Section links\"",
                file=sys.stderr,
            )


def filler(what: str) -> str:
    return "".join(f"Filler line {n} of {what}.\n\n" for n in range(30))


def target_body() -> str:
    return (
        "## What\n\nThe target. Jump to [the second budgets](#budgets-1).\n\n" + filler("what")
        + "## Budgets\n\nFirst budgets.\n\n" + filler("the first budgets")
        + "### Budget demo-quota-seconds\n\nThe demo quota.\n\n" + filler("the demo quota")
        + "### Step 2.b: retry (once)\n\nRetry once.\n\n" + filler("the retry")
        + "## Budgets\n\nSecond budgets.\n\n" + filler("the second budgets")
    )


def request(url: str, authorization: str, body: dict | None = None) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    headers = {
        "Content-Type": "application/json",
        "User-Agent": "onetaskgraph-section-links-live",
        "Authorization": authorization,
    }
    with urllib.request.urlopen(urllib.request.Request(url, data=data, headers=headers), timeout=60) as response:
        answer = json.load(response)
    if isinstance(answer, dict) and answer.get("errors"):
        raise RuntimeError(
            f"{url} refused: {json.dumps(answer['errors'])[:500]}; next: check the credential's "
            "scopes and the tracker's status, then run again"
        )
    return answer


def github(token: str, query: str, variables: dict) -> dict:
    return request("https://api.github.com/graphql", f"bearer {token}", {"query": query, "variables": variables})["data"]


def linear(key: str, query: str, variables: dict) -> dict:
    return request("https://api.linear.app/graphql", key, {"query": query, "variables": variables})["data"]


def scratch_issue(url: str) -> str:
    """The REST address of `url`, which has to be an issue of the scratch repository."""
    if not url.startswith(GITHUB_ISSUES) or not url[len(GITHUB_ISSUES):].isdigit():
        raise RuntimeError(f"{url} is no issue of {SCRATCH}; next: check GH_PROJECTS_REPOSITORY")
    return url.replace("https://github.com/", "https://api.github.com/repos/", 1)


def linear_url(url: str) -> str:
    if urllib.parse.urlparse(url).hostname != LINEAR_HOST:
        raise RuntimeError(f"{url} is not a Linear address; next: check the Linear source's endpoint")
    return url


def headings_of(state: str | None) -> list[tuple[str, str]]:
    import pycrdt

    if not state:
        return []
    doc = pycrdt.Doc()
    doc.apply_update(base64.b64decode(state))
    found: list[tuple[str, str]] = []

    def walk(node) -> None:
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
    if os.environ["GH_PROJECTS_REPOSITORY"].lower() != SCRATCH:
        sys.exit(f"GH_PROJECTS_REPOSITORY must be {SCRATCH}; next: nominate the scratch repository")
    linear_env = env_file(args.linear_env)
    key, team = linear_env["LINEAR_API_KEY"], linear_env["LINEAR_WRITE_TEAM"]
    if team != "TES2":
        sys.exit(f"LINEAR_WRITE_TEAM must be TES2, not {team}; next: use the scratch team's key")
    run = Run(stamp=f"{socket.gethostname()}-{os.getpid()}-{int(time.time() * 1e6)}")
    created_issues: list[str] = []
    created_linear: list[Record] = []
    board_copy: str | None = None
    try:
        held = github(token, "query($o:String!,$n:Int!){ repositoryOwner(login:$o){ id ... on User{ projectV2(number:$n){ id } } ... on Organization{ projectV2(number:$n){ id } } } }", {"o": owner, "n": number})["repositoryOwner"]
        copied = github(token, "mutation($p:ID!,$o:ID!,$t:String!){ copyProjectV2(input:{projectId:$p,ownerId:$o,title:$t,includeDraftIssues:false}){ projectV2{ id number url } } }", {"p": held["projectV2"]["id"], "o": held["id"], "t": f"onetaskgraph live board {run.stamp}"})["copyProjectV2"]["projectV2"]
        board_copy = copied["id"]
        run.board = copied["url"]
        options = github(token, "query($id:ID!){ node(id:$id){ ... on ProjectV2{ field(name:\"Status\"){ ... on ProjectV2SingleSelectField{ options{ name } } } } } }", {"id": board_copy})["node"]["field"]["options"]
        vocabulary = linear(key, "query($k:String!){ teams(filter:{key:{eq:$k}}){ nodes{ states{ nodes{ name type } } } } projectStatuses{ nodes{ name type } } }", {"k": team})
        states = vocabulary["teams"]["nodes"][0]["states"]["nodes"]
        open_state = next(state["name"] for state in states if state["type"] == "unstarted")
        done_state = next(state["name"] for state in states if state["type"] == "completed")
        project_status = next(status["name"] for status in vocabulary["projectStatuses"]["nodes"] if status["type"] in ("planned", "backlog"))

        # A Linear create is sent the id its create key derives from the source item, and a
        # record a previous run deleted still holds its id in Linear's trash.
        authored_name = f"authored-{int(time.time())}"
        sandbox = out / "sandbox"
        authored = sandbox / "authored"
        for folder in ("projects", "tasks", "documents"):
            (authored / folder).mkdir(parents=True, exist_ok=True)
        target = (authored / "tasks" / "T.md").resolve()

        def title(what: str) -> str:
            return f"{PREFIX}{run.stamp} {what}"

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
            authored_name: {"plugin": "local-md", "config": {"root": str(authored)}},
            "board": {"plugin": "github-projects", "config": {"owner": owner, "project_number": copied["number"], "repository": SCRATCH, "status_mapping": {"todo": options[0]["name"], "backlog": None, "queued": None, "in-progress": None}}},
            "linear": {"plugin": "linear", "config": {"team": team, "status_mapping": {"todo": {"task": open_state, "project": project_status}, "done": {"task": done_state}}}},
        }}))
        environment = {name: value for name, value in os.environ.items() if not name.startswith("ONETASKGRAPH_")}
        environment.update({"XDG_CONFIG_HOME": str(sandbox / "config"), "LINEAR_API_KEY": key, "GH_PROJECTS_TOKEN": token})

        def binary(*arguments: str) -> dict:
            done = subprocess.run([str(BINARY), "--json", *arguments], cwd=sandbox, env=environment, capture_output=True, text=True)
            if done.returncode != 0:
                raise RuntimeError(f"onetaskgraph {' '.join(arguments)} exited {done.returncode}: {done.stderr}next: fix what it names, then run again")
            return json.loads(done.stdout)

        def location(record: Record) -> str:
            return binary(record.verb, "show", record.id)["items"][0]["item"]["location"]["url"]

        def copy(source: dict[str, Record], to: str) -> dict[str, Record]:
            reports = []
            try:
                reports.append(binary("project", "copy", source["P"].id, "--to", to))
                reports.append(binary("document", "copy", source["D"].id, "--to", to))
            finally:
                # Everything a copy reported landing is removed, whichever later step failed.
                for report in reports:
                    for item in report["items"]:
                        if not item.get("destination"):
                            continue
                        verb = next(record.verb for record in source.values() if record.id == item["source"])
                        landed = Record("", verb, item["destination"])
                        if to == "board":
                            created_issues.append(location(landed))
                        else:
                            created_linear.append(landed)
            by_source = {item["source"]: item["destination"] for report in reports for item in report["items"]}
            return {name: Record(name, record.verb, by_source[record.id]) for name, record in source.items()}

        source = {name: Record(name, verb, f"{authored_name}:{name}") for name, verb in (("P", "project"), ("T", "task"), ("U", "task"), ("D", "document"))}
        on_board = copy(source, "board")
        in_linear = copy(source, "linear")
        onward = copy(on_board, "linear")

        run.github = {name: location(record) for name, record in on_board.items()}
        raw = {name: request(scratch_issue(url), f"bearer {token}")["body"] for name, url in run.github.items()}
        for heading in TARGET_HEADINGS:
            run.check(f"GitHub: the target issue carries <a id=\"{heading.github}\"></a> before its heading", f'<a id="{heading.github}"></a>\n#' in raw["T"], raw["T"][:300])
        run.check("GitHub: the design issue carries <a id=\"overview\"></a>", '<a id="overview"></a>\n## Overview' in raw["D"], raw["D"][:300])
        hrefs: dict[Link, str] = {}
        for link in LINKS:
            fragment = f"#user-content-{link.heading.github}"
            expected = fragment if link.names is None else run.github[link.names].replace("https://github.com/", "https://www.github.com/", 1) + fragment
            run.check(f"GitHub: {link.holder}'s [{link.text}] is {expected}", f"[{link.text}]({expected})" in raw[link.holder], raw[link.holder][:300])
            hrefs[link] = run.github[link.holder] + expected if link.names is None else expected

        def stored(record: Record) -> Stored:
            match record.verb:
                case "task":
                    held = linear(key, "query($id:String!){ issue(id:$id){ url description descriptionState } }", {"id": record.native})["issue"]
                    return Stored(linear_url(held["url"]), held["description"] or "", held["descriptionState"])
                case "project" | "document":
                    held = linear(key, f"query($id:String!){{ {record.verb}(id:$id){{ url content contentState }} }}", {"id": record.native})[record.verb]
                    return Stored(linear_url(held["url"]), held["content"] or "", held["contentState"])
                case other:
                    raise RuntimeError(f"no Linear record kind {other}")

        for hop, landed in (("local-md -> TES2", in_linear), ("board -> TES2", onward)):
            records = {name: stored(record) for name, record in landed.items()}
            run.linear[hop] = {name: record.url for name, record in records.items()}
            for name, record in records.items():
                run.check(f"Linear {hop}: {name} holds no anchor markup", "<a " not in record.body, record.body[:300])
                if hop.startswith("board"):
                    run.check(f"Linear {hop}: {name} names no GitHub issue", "github.com" not in record.body, record.body[:300])
            for link in LINKS:
                at = records[link.names or link.holder]
                ids = [held_id for held_text, held_id in headings_of(at.state) if held_text == link.heading.text]
                match = re.search(r"\[" + re.escape(link.text) + r"\]\(([^)]*)\)", records[link.holder].body)
                # Linear stores every link destination in angle brackets.
                written = match.group(1).removeprefix("<").removesuffix(">") if match else ""
                id8 = ids[link.heading.ordinal].replace("-", "")[:8] if len(ids) > link.heading.ordinal else None
                expected = f"{at.url}#heading-{link.heading.linear}-{id8}"
                run.check(f"Linear {hop}: {link.holder}'s [{link.text}] names the stored id of {link.heading.text} #{link.heading.ordinal}", id8 is not None and written == expected, f"wrote {written!r}, stored id8 {id8}")

        run.browser = browse(run, hrefs)
    # llmlint: ignore[changed_behavior_has_e2e] This script is itself the credentialed live check, run by hand and by no gate, so no journey can drive it; each run's cleanup is the behaviour it exercises against the real trackers, and its report names every record it could not remove. The dispatch that wrote it verified after each run that GitHub answers 410 for every issue it created, that its board copy no longer resolves, and that its Linear records read back trashed.
    finally:
        for url in created_issues:
            try:
                node = request(scratch_issue(url), f"bearer {token}")["node_id"]
                answer = github(token, "mutation($id:ID!){ deleteIssue(input:{issueId:$id}){ clientMutationId } }", {"id": node})
                if "deleteIssue" not in answer:
                    raise RuntimeError(f"deleteIssue answered {answer}")
            except Exception as error:  # noqa: BLE001 — every cleanup failure is collected and fails the run below
                run.cleanup_failures.append(f"issue {url}: {error}; next: delete it by hand on GitHub")
        if board_copy:
            try:
                answer = github(token, "mutation($id:ID!){ deleteProjectV2(input:{projectId:$id}){ clientMutationId } }", {"id": board_copy})
                if "deleteProjectV2" not in answer:
                    raise RuntimeError(f"deleteProjectV2 answered {answer}")
            except Exception as error:  # noqa: BLE001 — every cleanup failure is collected and fails the run below
                run.cleanup_failures.append(f"board {run.board}: {error}; next: delete it by hand on GitHub")
        for record in created_linear:
            mutation = {"task": "issueDelete", "document": "documentDelete", "project": "projectDelete"}[record.verb]
            try:
                answer = linear(key, f"mutation($id:String!){{ {mutation}(id:$id){{ success }} }}", {"id": record.native})
                if not answer[mutation]["success"]:
                    raise RuntimeError(f"{mutation} answered success false")
            except Exception as error:  # noqa: BLE001 — every cleanup failure is collected and fails the run below
                run.cleanup_failures.append(f"Linear {record.verb} {record.native}: {error}; next: delete it by hand in TES2")
        for failure in run.cleanup_failures:
            print(f"CLEANUP {failure}", file=sys.stderr)
        (out / "observations.json").write_text(json.dumps(dataclasses.asdict(run), indent=2, default=str) + "\n")
    failed = [check for check in run.checks if not check.held]
    print(f"{len(run.checks) - len(failed)} of {len(run.checks)} checks held; observations: {out / 'observations.json'}")
    return 1 if failed or run.cleanup_failures else 0


MEASURE = """(slug) => {
  const anchor = document.getElementById('user-content-' + slug);
  return {url: location.href, scrollY: Math.round(scrollY),
          anchorTop: anchor ? Math.round(anchor.getBoundingClientRect().top) : null};
}"""


def landed(seen: dict) -> bool:
    return seen["anchorTop"] is not None and seen["scrollY"] > 40 and -80 <= seen["anchorTop"] <= 250


def browse(run: Run, hrefs: dict[Link, str]) -> list[Observation]:
    from playwright.sync_api import sync_playwright

    executable = sorted(glob.glob(str(pathlib.Path.home() / ".cache/ms-playwright/chromium-*/chrome-linux*/chrome")))[-1]
    observations = []
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(executable_path=executable)
        context = browser.new_context(viewport={"width": 1280, "height": 800})
        for link, href in hrefs.items():
            seen = Observation(f"{link.holder}: [{link.text}]", href, link.heading.github)
            page = context.new_page()
            try:
                page.goto(href, wait_until="load", timeout=60000)
                time.sleep(4)
                seen.direct = page.evaluate(MEASURE, seen.slug)
            finally:
                page.close()
            page = context.new_page()
            try:
                page.goto(run.github[link.holder], wait_until="load", timeout=60000)
                time.sleep(4)
                anchor = page.locator(".markdown-body a", has_text=re.compile("^" + re.escape(link.text) + "$")).first
                anchor.scroll_into_view_if_needed(timeout=15000)
                before = set(context.pages)
                anchor.click()
                time.sleep(5)
                opened = [candidate for candidate in context.pages if candidate not in before]
                target = opened[0] if opened else page
                seen.click = target.evaluate(MEASURE, seen.slug)
                if target is not page:
                    target.close()
            finally:
                page.close()
            run.check(f"Chromium direct: {seen.link} lands on #user-content-{seen.slug}", landed(seen.direct), json.dumps(seen.direct))
            run.check(f"Chromium click: {seen.link} lands on #user-content-{seen.slug}", landed(seen.click), json.dumps(seen.click))
            observations.append(seen)
        browser.close()
    return observations


if __name__ == "__main__":
    sys.exit(main())
