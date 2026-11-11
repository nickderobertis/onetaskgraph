#!/usr/bin/env python3
"""Observe copied expandable blocks on the real trackers: run once by hand, never by a gate.

The credentialed check of README "Expandable blocks", by the manager's ruling on node
`onetaskgraph-expandable-blocks` (the section-links precedent, `scripts/live-section-links.py`):
it drives the real binary, named by `--binary`, within the live lane's write rules and reports
what it observed as evidence. Nothing selects it; `just` and Nx never run it.

1. Copies board #1 (`GH_PROJECTS_OWNER` / `GH_PROJECTS_NUMBER`) to a per-run board titled
   `onetaskgraph live board <stamp>`, never writing to board #1 itself; its issues are filed in
   `GH_PROJECTS_REPOSITORY`, which must be `nickderobertis/onetaskgraph-live-scratch`.
2. Writes a project into a scratch folder — its description, a task and a design document, each
   holding authored blocks whose summaries carry `<code>`, `<strong>` and `&amp;`, `&lt;`,
   `&gt;`, and whose bodies hold a list, a heading, fenced code and a table — every title
   carrying `onetaskgraph live cleanup <stamp>`.
3. Copies it to that board, and from the board onward to Linear team `LINEAR_WRITE_TEAM`
   (which must be `TES2`): a real tracker-to-tracker hop.
4. Reads every stored body back through each tracker's own API: on GitHub each block in the
   authored form, byte for byte; on Linear each as `+++ <summary>` … `+++` with its summary
   translated, no authored markup left, and each stored as a `collapsible_section` node of the
   record's editor state, decoded here with `pycrdt` rather than with the plugin's decoder, its
   `collapsible_heading` holding the summary's text. Then reads each Linear record back through
   the store, which has to give the authored blocks back.
5. Deletes everything it created — the issues, the board copy, the Linear records — however it
   ended, and names on stderr anything it could not.

Run from the repository root, after building the binary, naming the file the build wrote:

    uv run --no-project --with pycrdt python -I scripts/live-expandable-blocks.py \\
        --binary <the built onetaskgraph> --out <directory> --linear-env <a .env holding LINEAR_API_KEY and LINEAR_WRITE_TEAM>

Quiet on success but for one summary line; each failed check goes to stderr. Writes
`observations.json` into `--out` — every stored body and editor-state node it read — and exits
non-zero when a check failed or anything it created could not be removed.
"""

# llmlint: ignore-file[async_typed_clients_at_boundaries] A hand-run, strictly sequential observation, as `scripts/live-section-links.py` is: every step consumes the previous step's answer — the board copy before the copies, the copies before the reads — so an async client would add an event loop and nothing could run concurrently; the payloads are mapped into the dataclasses below as soon as they arrive.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] The forms are restated here on purpose: this script is the independent observer the README's expandable forms are held to on the real trackers, and deriving them from `ExpandableForm` would make it compare the engine with itself. The locally gated drift check is the journeys in `crates/onetaskgraph-e2e/tests/e2e/expandable_blocks.rs`, which restate the same forms literally.

from __future__ import annotations

import argparse
import base64
import dataclasses
import json
import os
import pathlib
import re
import socket
import subprocess
import sys
import time
import urllib.request

PREFIX = "onetaskgraph live cleanup "
SCRATCH = "nickderobertis/onetaskgraph-live-scratch"


@dataclasses.dataclass(frozen=True)
class Summary:
    """One block's summary: as authored, as Linear's form writes it, and as its editor shows it."""

    authored: str
    linear: str
    shown: str


SUMMARIES = [
    Summary("Run <code>copy --dry-run</code> first", "Run `copy --dry-run` first", "Run copy --dry-run first"),
    Summary("<strong>Decision</strong>: keep the cache", "**Decision**: keep the cache", "Decision: keep the cache"),
    Summary("Costs &amp; limits &lt;per hour&gt;", "Costs & limits <per hour>", "Costs & limits <per hour>"),
]


def block(summary: Summary, n: int) -> str:
    """One authored block numbered `n`: a paragraph, a list, a heading, fenced code and a table."""
    return (
        f"<details>\n<summary>{summary.authored}</summary>\n\nParagraph of block {n}.\n\n"
        f"- first item of block {n}\n- second item of block {n}\n\n### Detail of block {n}\n\n"
        f"```text\ncopy --dry-run {n}\n```\n\n| Budget | Limit |\n| --- | --- |\n| reads {n} | 27 |\n\n</details>"
    )


def body(what: str) -> str:
    """A record's authored body: a line naming it, then every block."""
    return f"The {what}.\n\n" + "\n\n".join(block(summary, n + 1) for n, summary in enumerate(SUMMARIES)) + "\n"


@dataclasses.dataclass(frozen=True)
class Record:
    """One record a copy landed: its key, the command line's verb for it, and its qualified id."""

    name: str
    verb: str
    id: str

    @property
    def native(self) -> str:
        return self.id.split(":", 1)[1]


@dataclasses.dataclass
class Check:
    what: str
    held: bool
    detail: str


@dataclasses.dataclass
class Run:
    stamp: str
    board: str | None = None
    github: dict[str, str] = dataclasses.field(default_factory=dict)
    github_bodies: dict[str, str] = dataclasses.field(default_factory=dict)
    linear: dict[str, str] = dataclasses.field(default_factory=dict)
    linear_bodies: dict[str, str] = dataclasses.field(default_factory=dict)
    linear_sections: dict[str, list[dict]] = dataclasses.field(default_factory=dict)
    read_back: dict[str, str] = dataclasses.field(default_factory=dict)
    checks: list[Check] = dataclasses.field(default_factory=list)
    cleanup_failures: list[str] = dataclasses.field(default_factory=list)

    def check(self, what: str, held: bool, detail: str = "") -> None:
        self.checks.append(Check(what, held, "" if held else detail))
        if not held:
            print(f"FAILED {what}: {detail}", file=sys.stderr)


class Failure(RuntimeError):
    """A step that cannot go on, with its next action in its message."""


def request(url: str, authorization: str, payload: dict | None = None) -> dict:
    data = None if payload is None else json.dumps(payload).encode()
    sent = urllib.request.Request(url, data=data, method="POST" if data else "GET", headers={
        "Authorization": authorization, "Content-Type": "application/json", "User-Agent": "onetaskgraph-live-expandable-blocks"})
    with urllib.request.urlopen(sent, timeout=60) as answer:
        held = json.loads(answer.read())
    if isinstance(held, dict) and held.get("errors"):
        raise Failure(f"{url} answered errors {held['errors']}; next: read them, then run again")
    return held


def at(value: object, *path: str | int) -> object:
    for step in path:
        if isinstance(step, int) and isinstance(value, list) and step < len(value):
            value = value[step]
        elif isinstance(step, str) and isinstance(value, dict):
            value = value.get(step)
        else:
            raise Failure(f"no {step!r} in the answer; next: compare the answer with the query")
    return value


def text_at(value: object, *path: str | int) -> str:
    held = at(value, *path)
    if not isinstance(held, str):
        raise Failure(f"{'.'.join(map(str, path))} is {held!r}, not text; next: compare the answer with the query")
    return held


def github(token: str, query: str, variables: dict) -> dict:
    return at(request("https://api.github.com/graphql", f"bearer {token}", {"query": query, "variables": variables}), "data")


def linear(key: str, query: str, variables: dict) -> dict:
    return at(request("https://api.linear.app/graphql", key, {"query": query, "variables": variables}), "data")


def scratch_issue(url: str) -> str:
    match = re.fullmatch(rf"https://github\.com/{re.escape(SCRATCH)}/issues/(\d+)", url)
    if not match:
        raise Failure(f"{url} is no issue of {SCRATCH}; next: check where the board files its issues")
    return f"https://api.github.com/repos/{SCRATCH}/issues/{match.group(1)}"


def sections_of(state: str | None) -> list[dict]:
    """Every `collapsible_section` node of a Linear editor state: its heading's text and marks,
    and the types of the nodes inside it beside the heading."""
    import pycrdt

    if not state:
        return []
    doc = pycrdt.Doc()
    doc.apply_update(base64.b64decode(state))
    found: list[dict] = []

    def text(node) -> tuple[str, set[str]]:
        held, marks = "", set()
        if isinstance(node, pycrdt.XmlText):
            for piece, attributes in node.diff():
                held += piece
                marks.update((attributes or {}).keys())
        elif isinstance(node, pycrdt.XmlElement):
            for child in node.children:
                more, inner = text(child)
                held += more
                marks |= inner
        return held, marks

    def walk(node) -> None:
        if isinstance(node, pycrdt.XmlElement):
            if node.tag == "collapsible_section":
                children = list(node.children)
                heading = next((child for child in children if isinstance(child, pycrdt.XmlElement) and child.tag == "collapsible_heading"), None)
                shown, marks = text(heading) if heading is not None else ("", set())
                found.append({"summary": shown.strip(), "marks": sorted(marks),
                              "body": [child.tag for child in children if isinstance(child, pycrdt.XmlElement) and child is not heading]})
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
        print(f"live-expandable-blocks: {failure}", file=sys.stderr)
        return 1
    except Exception as error:  # noqa: BLE001 — any other failure is reported with its next action rather than as a traceback
        print(f"live-expandable-blocks: {type(error).__name__}: {error}; next: read observations.json and the trackers, then run again", file=sys.stderr)
        return 1


# llmlint: ignore-block[changed_behavior_has_e2e] This orchestrator is itself the credentialed live check, run by hand and by no gate by the manager's ruling on node onetaskgraph-expandable-blocks, so no journey can drive it against the real trackers it exists to observe; the expandable-block journeys in `crates/onetaskgraph-e2e` hold the behaviour it observes.
def observe() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--linear-env", required=True)
    args = parser.parse_args()
    program = pathlib.Path(args.binary).resolve()
    if not program.is_file():
        raise Failure(f"{args.binary} is not a file; next: build the binary and name the file the build wrote")
    out = pathlib.Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    token, owner = required("GH_PROJECTS_TOKEN"), required("GH_PROJECTS_OWNER")
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

        def named(rows: object, types: tuple[str, ...]) -> str:
            for row in rows if isinstance(rows, list) else []:
                if isinstance(row, dict) and row.get("type") in types and isinstance(row.get("name"), str):
                    return row["name"]
            raise Failure(f"TES2 holds no state of type {types}; next: check the team's workflow")

        open_state = named(at(vocabulary, "teams", "nodes", 0, "states", "nodes"), ("unstarted",))
        project_status = named(at(vocabulary, "projectStatuses", "nodes"), ("planned", "backlog"))

        authored_name = f"authored-{int(time.time())}"
        sandbox = out / "sandbox"
        authored = sandbox / "authored"
        for folder in ("projects", "tasks", "documents"):
            (authored / folder).mkdir(parents=True, exist_ok=True)

        def title(what: str) -> str:
            return f"{PREFIX}{run.stamp} {what}"

        bodies = {"P": body("project"), "T": body("task"), "D": body("document")}
        for path, front, key_name in (
            ("projects/P.md", f"title: {title('Block plan')}\nstatus: todo", "P"),
            ("tasks/T.md", f"title: {title('Blocks')}\nstatus: todo\nproject: P", "T"),
            ("documents/D.md", f"title: {title('Block design')}\nproject: P", "D"),
        ):
            (authored / path).write_text(f"---\n{front}\n---\n{bodies[key_name]}")
        (sandbox / "config").mkdir(exist_ok=True)
        (sandbox / "onetaskgraph.yaml").write_text(json.dumps({"sources": {
            authored_name: {"plugin": "local-md", "config": {"root": str(authored)}},
            "board": {"plugin": "github-projects", "config": {"owner": owner, "project_number": board_number, "repository": SCRATCH, "status_mapping": {"todo": first_option, "backlog": None, "queued": None, "in-progress": None}}},
            "linear": {"plugin": "linear", "config": {"team": team, "status_mapping": {"todo": {"task": open_state, "project": project_status}}}},
        }}))
        environment = {name: value for name, value in os.environ.items() if not name.startswith("ONETASKGRAPH_")}
        environment.update({"XDG_CONFIG_HOME": str(sandbox / "config"), "LINEAR_API_KEY": key, "GH_PROJECTS_TOKEN": token})

        def binary(*arguments: str) -> dict:
            done = subprocess.run([str(program), "--json", *arguments], cwd=sandbox, env=environment, capture_output=True, text=True)
            if done.returncode != 0:
                raise Failure(f"onetaskgraph {' '.join(arguments)} exited {done.returncode}: {done.stderr}next: fix what it names, then run again")
            return json.loads(done.stdout)

        def shown(record: Record) -> dict:
            return at(binary(record.verb, "show", record.id), "items", 0, "item")

        def copy(source: dict[str, Record], to: str) -> dict[str, Record]:
            reports = []
            try:
                reports.append(binary("project", "copy", source["P"].id, "--to", to))
                reports.append(binary("document", "copy", source["D"].id, "--to", to))
            finally:
                # Everything a copy reported landing is removed, whichever later step failed.
                for report in reports:
                    for item in at(report, "items") or []:
                        destination = item.get("destination") if isinstance(item, dict) else None
                        if not isinstance(destination, str):
                            continue
                        verb = next(record.verb for record in source.values() if record.id == item.get("source"))
                        landed = Record("", verb, destination)
                        if to == "board":
                            created_issues.append(text_at(shown(landed), "location", "url"))
                        else:
                            created_linear.append(landed)
            by_source = {item["source"]: item.get("destination") for report in reports for item in at(report, "items")}
            landed_at = {}
            for name, record in source.items():
                destination = by_source.get(record.id)
                if not isinstance(destination, str):
                    raise Failure(f"{record.id} did not land in {to}; next: read the copy's report")
                landed_at[name] = Record(name, record.verb, destination)
            return landed_at

        source = {name: Record(name, verb, f"{authored_name}:{name}") for name, verb in (("P", "project"), ("T", "task"), ("D", "document"))}
        on_board = copy(source, "board")
        onward = copy(on_board, "linear")

        # GitHub: every block in the authored form, byte for byte, its heading anchored.
        run.github = {name: text_at(shown(record), "location", "url") for name, record in on_board.items()}
        for name, url in run.github.items():
            stored = text_at(request(scratch_issue(url), f"bearer {token}"), "body")
            run.github_bodies[name] = stored
            for n, summary in enumerate(SUMMARIES, start=1):
                opening = f"<details>\n<summary>{summary.authored}</summary>\n\nParagraph of block {n}."
                run.check(f"GitHub {name}: block {n} is stored in the authored form", opening in stored and f"| reads {n} | 27 |\n\n</details>" in stored, stored[:400])
                run.check(f"GitHub {name}: block {n}'s heading carries its anchor", f'<a id="detail-of-block-{n}"></a>\n### Detail of block {n}' in stored, stored[:400])

        # Linear: every block a collapsible section, stored as one in its editor state.
        for name, record in onward.items():
            if record.verb == "task":
                held = at(linear(key, "query($id:String!){ issue(id:$id){ url description descriptionState } }", {"id": record.native}), "issue")
                stored, state = at(held, "description") or "", at(held, "descriptionState")
            else:
                held = at(linear(key, f"query($id:String!){{ {record.verb}(id:$id){{ url content contentState }} }}", {"id": record.native}), record.verb)
                stored, state = at(held, "content") or "", at(held, "contentState")
            run.linear[name] = re.sub(r"https://linear\.app/[^/]+/", "https://linear.app/<workspace>/", text_at(held, "url"))
            run.linear_bodies[name] = stored
            sections = sections_of(state if isinstance(state, str) else None)
            run.linear_sections[name] = sections
            lines = stored.split("\n")
            for n, summary in enumerate(SUMMARIES, start=1):
                header = f"+++ {summary.linear}"
                run.check(f"Linear {name}: block {n} is stored as `{header}`, a blank line after it", header in lines and lines[lines.index(header) + 1] == "", stored[:400])
            run.check(f"Linear {name}: every section is closed by `+++`", lines.count("+++") == len(SUMMARIES), stored[:400])
            run.check(f"Linear {name}: no authored markup is left", not any(line.startswith(("<details", "<summary>")) or line == "</details>" for line in lines), stored[:400])
            run.check(f"Linear {name}: its editor state holds one collapsible_section per block, its heading the summary", [section["summary"] for section in sections] == [summary.shown for summary in SUMMARIES], json.dumps(sections))
            for n, section in enumerate(sections, start=1):
                for kind in ("paragraph", "heading", "code_block", "table"):
                    run.check(f"Linear {name}: section {n} keeps a {kind} inside it", kind in section["body"], json.dumps(section))
                run.check(f"Linear {name}: section {n} keeps its list inside it", any(kind.endswith("_list") for kind in section["body"]), json.dumps(section))

        # Read back through the store: the authored blocks.
        for name, record in onward.items():
            content = shown(record).get("content") or ""
            run.read_back[name] = content
            for n, summary in enumerate(SUMMARIES, start=1):
                run.check(f"store read of Linear {name}: block {n} reads back in the authored form", f"<details>\n<summary>{summary.authored}</summary>\n\nParagraph of block {n}." in content and content.count("</details>") == len(SUMMARIES), content[:400])
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
            mutation = {"task": "issueDelete", "document": "documentDelete", "project": "projectDelete"}[record.verb]
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


if __name__ == "__main__":
    sys.exit(main())
