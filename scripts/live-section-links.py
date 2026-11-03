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
import enum
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


class Verb(enum.StrEnum):
    """The kind of record, as the command line names it."""

    TASK = "task"
    PROJECT = "project"
    DOCUMENT = "document"


QualifiedId = str
NativeId = str


@dataclasses.dataclass(frozen=True)
class Record:
    """One record of the copied project, by its key, its kind and its qualified id."""

    key: str
    verb: Verb
    id: QualifiedId

    @property
    def native(self) -> NativeId:
        return self.id.split(":", 1)[1]


@dataclasses.dataclass(frozen=True)
class StoredHeading:
    """One heading of a Linear editor state: its text and the id Linear stored it under."""

    text: str
    id: str


@dataclasses.dataclass(frozen=True)
class Measurement:
    """Where a page was, a few seconds after it settled."""

    url: str
    scroll_y: int
    anchor_top: int | None

    def landed(self) -> bool:
        """Whether the named heading's anchor is what the page scrolled to."""
        return self.anchor_top is not None and self.scroll_y > 40 and -80 <= self.anchor_top <= 250


@dataclasses.dataclass(frozen=True)
class AuthoredFile:
    """One record of the authored folder: where it is, its front matter and its body."""

    path: str
    front: str
    body: str


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
    direct: Measurement | None = None
    click: Measurement | None = None


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


class Failure(RuntimeError):
    """A failure this script states with its next action."""


def request(url: str, authorization: str, body: dict | None = None) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    headers = {
        "Content-Type": "application/json",
        "User-Agent": "onetaskgraph-section-links-live",
        "Authorization": authorization,
    }
    try:
        with urllib.request.urlopen(urllib.request.Request(url, data=data, headers=headers), timeout=60) as response:
            answer = json.load(response)
    except (OSError, ValueError) as error:
        raise Failure(f"{url} could not be read: {error}; next: check the network and the credential, then run again") from error
    if not isinstance(answer, dict):
        raise Failure(f"{url} answered {type(answer).__name__}, not a JSON object; next: check the tracker's status")
    if answer.get("errors"):
        raise Failure(
            f"{url} refused: {json.dumps(answer['errors'])[:500]}; next: check the credential's "
            "scopes and the tracker's status, then run again"
        )
    return answer


def at(value: object, *path: str | int) -> object:
    """The member of `value` at `path`, refused with what is missing where an answer lacks it."""
    for step in path:
        match (value, step):
            case (dict(), str()):
                held = value.get(step)
            case (list(), int()) if step < len(value):
                held = value[step]
            case _:
                held = None
        if held is None:
            raise Failure(f"an answer lacks {'.'.join(map(str, path))}; next: compare the query with the tracker's schema")
        value = held
    return value


def optional_text(value: object, field: str) -> str | None:
    """A member an answer may give as a string or as null."""
    held = value.get(field) if isinstance(value, dict) else None
    if held is not None and not isinstance(held, str):
        raise Failure(f"an answer's {field} is not a string; next: compare the query with the tracker's schema")
    return held


@dataclasses.dataclass(frozen=True)
class Named:
    """A Linear workflow state or project status: its name and its type."""

    name: str
    type: str


@dataclasses.dataclass(frozen=True)
class Outcome:
    """One item of a copy's report: its source id and, once it landed, where."""

    source: QualifiedId
    destination: QualifiedId | None


def outcomes(report: dict) -> list[Outcome]:
    items = at(report, "items")
    if not isinstance(items, list):
        raise Failure("a copy's report holds no item list; next: run the copy by hand with --json")
    return [Outcome(qualified(text_at(item, "source")), qualified_or_none(optional_text(item, "destination"))) for item in items]


def qualified(id: str) -> QualifiedId:
    """`id` when it is `<source>:<native id>`, both non-empty."""
    source, colon, native = id.partition(":")
    if not colon or not source or not native.strip():
        raise Failure(f"{id!r} is not a qualified id; next: read the copy's report by hand with --json")
    return id


def qualified_or_none(id: str | None) -> QualifiedId | None:
    return None if id is None else qualified(id)


def text_at(value: object, *path: str | int) -> str:
    held = at(value, *path)
    if not isinstance(held, str):
        raise Failure(f"an answer's {'.'.join(map(str, path))} is not a string; next: compare the query with the tracker's schema")
    return held


def github(token: str, query: str, variables: dict) -> dict:
    return at(request("https://api.github.com/graphql", f"bearer {token}", {"query": query, "variables": variables}), "data")


def linear(key: str, query: str, variables: dict) -> dict:
    return at(request("https://api.linear.app/graphql", key, {"query": query, "variables": variables}), "data")


def scratch_issue(url: str) -> str:
    """The REST address of `url`, which has to be an issue of the scratch repository."""
    if not url.startswith(GITHUB_ISSUES) or not url[len(GITHUB_ISSUES):].isdigit():
        raise Failure(f"{url} is no issue of {SCRATCH}; next: check GH_PROJECTS_REPOSITORY")
    return url.replace("https://github.com/", "https://api.github.com/repos/", 1)


def linear_url(url: str) -> str:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "https" or parsed.hostname != LINEAR_HOST or parsed.port is not None or parsed.username or parsed.password:
        raise Failure(f"{url} is not an https Linear address; next: check the Linear source's endpoint")
    return url


def headings_of(state: str | None) -> list[StoredHeading]:
    import pycrdt

    if not state:
        return []
    doc = pycrdt.Doc()
    doc.apply_update(base64.b64decode(state))
    found: list[StoredHeading] = []

    def walk(node) -> None:
        if isinstance(node, pycrdt.XmlElement):
            if node.tag == "heading":
                text = "".join(piece[0] for child in node.children if isinstance(child, pycrdt.XmlText) for piece in child.diff())
                held = node.attributes.get("id")
                if not isinstance(held, str) or not re.fullmatch(r"[0-9a-fA-F-]+", held) or sum(c.isalnum() for c in held) < 8:
                    raise Failure(f"a stored heading id is {held!r}, not Linear's hex id; next: capture the record's editor state and compare it with the fixtures")
                found.append(StoredHeading(text.strip(), held))
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


def required(name: str, values: dict[str, str] | None = None) -> str:
    held = (values if values is not None else os.environ).get(name)
    if not held:
        raise Failure(f"{name} is not set; next: set it as the live lane's grant names it, then run again")
    return held


def main() -> int:
    try:
        return observe()
    except Failure as failure:
        print(f"live-section-links: {failure}", file=sys.stderr)
        return 1
    except Exception as error:  # noqa: BLE001 — any other failure is reported with its next action rather than as a traceback
        print(f"live-section-links: {type(error).__name__}: {error}; next: read observations.json and the trackers, then run again", file=sys.stderr)
        return 1


# llmlint: ignore-block[changed_behavior_has_e2e] This orchestrator is itself the credentialed live check, run by hand and by no gate by the manager's ruling on node onetaskgraph-section-links, so no journey can drive it against the real trackers it exists to observe; the section-link journeys in `crates/onetaskgraph-e2e` hold the behaviour it observes.
def observe() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--linear-env", required=True)
    args = parser.parse_args()
    out = pathlib.Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    token = required("GH_PROJECTS_TOKEN")
    owner = required("GH_PROJECTS_OWNER")
    if not required("GH_PROJECTS_NUMBER").isdigit() or int(required("GH_PROJECTS_NUMBER")) < 1:
        raise Failure("GH_PROJECTS_NUMBER is not a positive project number; next: nominate board #1's number")
    number = int(required("GH_PROJECTS_NUMBER"))
    if required("GH_PROJECTS_REPOSITORY").lower() != SCRATCH:
        raise Failure(f"GH_PROJECTS_REPOSITORY must be {SCRATCH}; next: nominate the scratch repository")
    try:
        linear_env = env_file(args.linear_env)
    except OSError as error:
        raise Failure(f"{args.linear_env} cannot be read: {error}; next: name the .env holding the TES2 key") from error
    key, team = required("LINEAR_API_KEY", linear_env), required("LINEAR_WRITE_TEAM", linear_env)
    if team != "TES2":
        raise Failure(f"LINEAR_WRITE_TEAM must be TES2, not {team}; next: use the scratch team's key")
    run = Run(stamp=f"{socket.gethostname()}-{os.getpid()}-{int(time.time() * 1e6)}")
    created_issues: list[str] = []
    created_linear: list[Record] = []
    board_copy: str | None = None
    try:
        held = github(token, "query($o:String!,$n:Int!){ repositoryOwner(login:$o){ id ... on User{ projectV2(number:$n){ id } } ... on Organization{ projectV2(number:$n){ id } } } }", {"o": owner, "n": number})
        owner_id, template = text_at(held, "repositoryOwner", "id"), text_at(held, "repositoryOwner", "projectV2", "id")
        copied = github(token, "mutation($p:ID!,$o:ID!,$t:String!){ copyProjectV2(input:{projectId:$p,ownerId:$o,title:$t,includeDraftIssues:false}){ projectV2{ id number url } } }", {"p": template, "o": owner_id, "t": f"onetaskgraph live board {run.stamp}"})
        board_copy = text_at(copied, "copyProjectV2", "projectV2", "id")
        run.board = text_at(copied, "copyProjectV2", "projectV2", "url")
        board_number = at(copied, "copyProjectV2", "projectV2", "number")
        if not isinstance(board_number, int) or board_number < 1:
            raise Failure(f"GitHub answered the board copy's number as {board_number!r}; next: check the copy on GitHub")
        options = github(token, "query($id:ID!){ node(id:$id){ ... on ProjectV2{ field(name:\"Status\"){ ... on ProjectV2SingleSelectField{ options{ name } } } } } }", {"id": board_copy})
        first_option = text_at(options, "node", "field", "options", 0, "name")
        vocabulary = linear(key, "query($k:String!){ teams(filter:{key:{eq:$k}}){ nodes{ states{ nodes{ name type } } } } projectStatuses{ nodes{ name type } } }", {"k": team})
        def decoded(rows: object) -> list[Named]:
            if not isinstance(rows, list):
                raise Failure("Linear answered no list of states; next: check the team key")
            return [Named(text_at(row, "name"), text_at(row, "type")) for row in rows]

        states = decoded(at(vocabulary, "teams", "nodes", 0, "states", "nodes"))
        statuses = decoded(at(vocabulary, "projectStatuses", "nodes"))

        def named(rows: list[Named], types: tuple[str, ...]) -> str:
            for row in rows:
                if row.type in types:
                    return row.name
            raise Failure(f"TES2 holds no state of type {types}; next: check the team's workflow")

        open_state, done_state = named(states, ("unstarted",)), named(states, ("completed",))
        project_status = named(statuses, ("planned", "backlog"))

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

        files = [
            AuthoredFile("projects/P.md", f"title: {title('Section plan')}\nstatus: todo", "The plan: [what it is](../tasks/T.md#what) and [the retry](../tasks/T.md#step-2b-retry-once).\n"),
            AuthoredFile("tasks/T.md", f"title: {title('Target')}\nstatus: todo\nproject: P", target_body()),
            AuthoredFile("tasks/U.md", f"title: {title('Referrer')}\nstatus: todo\nproject: P", f"See [the demo](T.md#budget-demo-quota-seconds) and [the first]({target}#budgets).\n"),
            AuthoredFile("documents/D.md", f"title: {title('Design')}\nproject: P", f"## Overview\n\n| Budget | Detail |\n| --- | --- |\n| first | [first](../tasks/T.md#budgets) |\n| second | [second](../tasks/T.md#budgets-1) |\n| retry | [retry]({target}#step-2b-retry-once) |\n\nBack to the [overview](#overview).\n"),
        ]
        for file in files:
            (authored / file.path).write_text(f"---\n{file.front}\n---\n{file.body}")
        (sandbox / "config").mkdir(exist_ok=True)
        (sandbox / "onetaskgraph.yaml").write_text(json.dumps({"sources": {
            authored_name: {"plugin": "local-md", "config": {"root": str(authored)}},
            "board": {"plugin": "github-projects", "config": {"owner": owner, "project_number": board_number, "repository": SCRATCH, "status_mapping": {"todo": first_option, "backlog": None, "queued": None, "in-progress": None}}},
            "linear": {"plugin": "linear", "config": {"team": team, "status_mapping": {"todo": {"task": open_state, "project": project_status}, "done": {"task": done_state}}}},
        }}))
        environment = {name: value for name, value in os.environ.items() if not name.startswith("ONETASKGRAPH_")}
        environment.update({"XDG_CONFIG_HOME": str(sandbox / "config"), "LINEAR_API_KEY": key, "GH_PROJECTS_TOKEN": token})

        def binary(*arguments: str) -> dict:
            done = subprocess.run([str(BINARY), "--json", *arguments], cwd=sandbox, env=environment, capture_output=True, text=True)
            if done.returncode != 0:
                raise Failure(f"onetaskgraph {' '.join(arguments)} exited {done.returncode}: {done.stderr}next: fix what it names, then run again")
            return json.loads(done.stdout)

        def location(record: Record) -> str:
            return text_at(binary(record.verb, "show", record.id), "items", 0, "item", "location", "url")

        def copy(source: dict[str, Record], to: str) -> dict[str, Record]:
            reports = []
            try:
                reports.append(binary("project", "copy", source["P"].id, "--to", to))
                reports.append(binary("document", "copy", source["D"].id, "--to", to))
            finally:
                # Everything a copy reported landing is removed, whichever later step failed.
                for report in reports:
                    for outcome in outcomes(report):
                        if outcome.destination is None:
                            continue
                        verb = next(record.verb for record in source.values() if record.id == outcome.source)
                        landed = Record("", verb, outcome.destination)
                        if to == "board":
                            created_issues.append(location(landed))
                        else:
                            created_linear.append(landed)
            by_source = {outcome.source: outcome.destination for report in reports for outcome in outcomes(report)}
            landed_at = {}
            for name, record in source.items():
                destination = by_source.get(record.id)
                if destination is None:
                    raise Failure(f"{record.id} did not land in {to}; next: read the copy's report")
                landed_at[name] = Record(name, record.verb, destination)
            return landed_at

        source = {name: Record(name, verb, f"{authored_name}:{name}") for name, verb in (("P", Verb.PROJECT), ("T", Verb.TASK), ("U", Verb.TASK), ("D", Verb.DOCUMENT))}
        on_board = copy(source, "board")
        in_linear = copy(source, "linear")
        onward = copy(on_board, "linear")

        run.github = {name: location(record) for name, record in on_board.items()}
        raw = {name: text_at(request(scratch_issue(url), f"bearer {token}"), "body") for name, url in run.github.items()}
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
                case Verb.TASK:
                    held = at(linear(key, "query($id:String!){ issue(id:$id){ url description descriptionState } }", {"id": record.native}), "issue")
                    return Stored(linear_url(text_at(held, "url")), optional_text(held, "description") or "", optional_text(held, "descriptionState"))
                case Verb.PROJECT | Verb.DOCUMENT:
                    held = linear(key, f"query($id:String!){{ {record.verb}(id:$id){{ url content contentState }} }}", {"id": record.native})
                    held = at(held, str(record.verb))
                    return Stored(linear_url(text_at(held, "url")), optional_text(held, "content") or "", optional_text(held, "contentState"))
                case other:
                    raise Failure(f"no Linear record kind {other}; next: add its read to `stored`, beside the task, project and document reads")

        for hop, landed in (("local-md -> TES2", in_linear), ("board -> TES2", onward)):
            records = {name: stored(record) for name, record in landed.items()}
            run.linear[hop] = {name: record.url for name, record in records.items()}
            for name, record in records.items():
                run.check(f"Linear {hop}: {name} holds no anchor markup", "<a " not in record.body, record.body[:300])
                if hop.startswith("board"):
                    run.check(f"Linear {hop}: {name} names no GitHub issue", "github.com" not in record.body, record.body[:300])
            for link in LINKS:
                at_record = records[link.names or link.holder]
                ids = [held.id for held in headings_of(at_record.state) if held.text == link.heading.text]
                match = re.search(r"\[" + re.escape(link.text) + r"\]\(([^)]*)\)", records[link.holder].body)
                # Linear stores every link destination in angle brackets.
                written = match.group(1).removeprefix("<").removesuffix(">") if match else ""
                id8 = ids[link.heading.ordinal].replace("-", "")[:8] if len(ids) > link.heading.ordinal else None
                expected = f"{at_record.url}#heading-{link.heading.linear}-{id8}"
                run.check(f"Linear {hop}: {link.holder}'s [{link.text}] names the stored id of {link.heading.text} #{link.heading.ordinal}", id8 is not None and written == expected, f"wrote {written!r}, stored id8 {id8}")

        run.browser = browse(run, hrefs)
    finally:
        for url in created_issues:
            try:
                node = text_at(request(scratch_issue(url), f"bearer {token}"), "node_id")
                answer = github(token, "mutation($id:ID!){ deleteIssue(input:{issueId:$id}){ clientMutationId } }", {"id": node})
                if "deleteIssue" not in answer:
                    raise Failure(f"deleteIssue answered {answer}")
            except Exception as error:  # noqa: BLE001 — every cleanup failure is collected and fails the run below
                run.cleanup_failures.append(f"issue {url}: {error}; next: delete it by hand on GitHub")
        if board_copy:
            try:
                answer = github(token, "mutation($id:ID!){ deleteProjectV2(input:{projectId:$id}){ clientMutationId } }", {"id": board_copy})
                if "deleteProjectV2" not in answer:
                    raise Failure(f"deleteProjectV2 answered {answer}")
            except Exception as error:  # noqa: BLE001 — every cleanup failure is collected and fails the run below
                run.cleanup_failures.append(f"board {run.board}: {error}; next: delete it by hand on GitHub")
        for record in created_linear:
            mutation = {Verb.TASK: "issueDelete", Verb.DOCUMENT: "documentDelete", Verb.PROJECT: "projectDelete"}[record.verb]
            try:
                answer = linear(key, f"mutation($id:String!){{ {mutation}(id:$id){{ success }} }}", {"id": record.native})
                if at(answer, mutation, "success") is not True:
                    raise Failure(f"{mutation} answered success false")
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


def measured(page, slug: str) -> Measurement:
    seen = page.evaluate(MEASURE, slug)
    return Measurement(str(seen["url"]), int(seen["scrollY"]), seen["anchorTop"])


def browse(run: Run, hrefs: dict[Link, str]) -> list[Observation]:
    from playwright.sync_api import sync_playwright

    found = sorted(glob.glob(str(pathlib.Path.home() / ".cache/ms-playwright/chromium-*/chrome-linux*/chrome")))
    if not found:
        raise Failure("no Playwright Chromium is installed; next: run `uv run --no-project --with playwright python -m playwright install chromium`")
    executable = found[-1]
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
                seen.direct = measured(page, seen.slug)
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
                seen.click = measured(target, seen.slug)
                if target is not page:
                    target.close()
            finally:
                page.close()
            for mode, measurement in (("direct", seen.direct), ("click", seen.click)):
                run.check(f"Chromium {mode}: {seen.link} lands on #user-content-{seen.slug}", measurement is not None and measurement.landed(), str(measurement))
            observations.append(seen)
        browser.close()
    return observations


# llmlint: ignore-end[changed_behavior_has_e2e]


if __name__ == "__main__":
    sys.exit(main())
