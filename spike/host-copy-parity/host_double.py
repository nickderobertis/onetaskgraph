"""spike-host-copy-parity: the plan copy against ai-orchestrator's own GitHub double.

Imports that repository's `tests/github_board.py` read-only (no bytecode is written), serves
its board double filled to a board size, writes the scenario's plan into a folder of Markdown
and copies it three times through a onetaskgraph binary — first, after one task changed, and
unchanged — printing each copy's operations and points as that double names and prices them.

    AIO=<ai-orchestrator checkout> <its .venv>/bin/python -B spike/host-copy-parity/host_double.py \
        [--binary target/debug/onetaskgraph] [--items 2000] [--cache] [--one-edge] [--preserve]
        [--private] [--instant] [--fixed-double] [--keep DIR]

`SPIKE_FIXES` in the environment reaches the binary, as `measure.sh` passes it.

The board source's owner, number and repository are read from that checkout's own
`onetaskgraph.yaml` at run time, so nothing here names them. Throwaway measurement code.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

AIO = Path(os.environ.get("AIO", Path.home() / "ai-orchestrator"))
sys.path[:0] = [str(AIO / "tests"), str(AIO / "tests" / "e2e"), str(AIO)]
sys.dont_write_bytecode = True

import github_board as g  # noqa: E402
import yaml  # noqa: E402

EDGES = [(2, 1), (3, 1), (3, 2), (4, 3), (5, 3), (6, 4), (6, 5), (7, 6), (8, 7), (8, 2)]


def author(root: Path, edges: list[tuple[int, int]]) -> None:
    for directory in ("projects", "tasks", "documents/D.assets"):
        (root / directory).mkdir(parents=True, exist_ok=True)
    (root / "projects/P.md").write_text(
        "---\ntitle: Copy the plan\nstatus: todo\n---\nThe plan.\n", encoding="utf-8"
    )
    for n in range(1, 9):
        write_task(root, n, f"Step {n} of the plan, its body.\n", edges)
    rows = "".join(f"| T-{n} | `{root / 'tasks' / f'T-{n}.md'}` |\n" for n in range(1, 9))
    (root / "documents/D.assets/diagram.png").write_bytes(
        b"\x89PNG\r\n\x1a\n" + bytes(range(256)) * 16
    )
    (root / "documents/D.md").write_text(
        "---\ntitle: Design of the plan copy\nproject: P\n---\nThe design.\n\n"
        f"| Task | Where |\n| --- | --- |\n{rows}\n![diagram](./diagram.png)\n",
        encoding="utf-8",
    )


def write_task(root: Path, n: int, body: str, edges: list[tuple[int, int]]) -> None:
    path = root / "tasks" / f"T-{n}.md"
    kept = ""
    if path.exists():
        front = path.read_text(encoding="utf-8").split("\n---\n", 1)[0]
        if "metadata:" in front:
            kept = front[front.index("metadata:") :] + "\n"
    depends = [f"T-{on}" for task, on in edges if task == n]
    depends_line = f"depends_on: [{', '.join(depends)}]\n" if depends else ""
    path.write_text(
        f"---\ntitle: Step {n} of the plan\nstatus: todo\nproject: P\n{depends_line}{kept}---\n"
        f"{body}",
        encoding="utf-8",
    )


def fix_sub_issues() -> None:
    """Answer `SUB_ISSUES` as GitHub does: the project issue's own `BoardIssue` fields and
    `blockedBy`, and each member's `blockedBy`, all of which the store's query selects and the
    double leaves out. Patched on the imported class in this process alone."""

    def blocked_by(board: g._Board, issue: g._Issue) -> dict[str, object]:
        return {
            "nodes": [board._issue(held).content() for held in issue.blocked_by],
            "pageInfo": {"hasNextPage": False, "endCursor": None},
        }

    def sub_issues_response(self: g._Board, node_id: object) -> dict[str, object]:
        for issue in self.issues:
            if issue.content_id != node_id:
                continue
            children = [held for held in self.issues if held.parent_id == issue.content_id]
            return {
                "data": {
                    "node": issue.board_issue()
                    | {
                        "blockedBy": blocked_by(self, issue),
                        "subIssues": {
                            "pageInfo": {"hasNextPage": False, "endCursor": None},
                            "nodes": [
                                child.board_issue() | {"blockedBy": blocked_by(self, child)}
                                for child in children
                            ],
                        },
                    }
                }
            }
        return {"data": {"node": None}}

    g._Board.sub_issues_response = sub_issues_response


#: The onevcs release onetaskgraph's journeys pin (`ONEVCS_CLI` in onetaskgraph-e2e-support).
ONEVCS_CLI = "onevcs-cli==0.44.0"


def synthetic_onevcs(work: Path, repository: str) -> tuple[Path, str]:
    """An onevcs home under ``work`` declaring ``repository`` public by rule, so no host is
    asked, and the pinned onevcs's path — as `OnevcsHome` lays one for the Rust journeys."""
    onevcs = subprocess.run(
        ["uv", "tool", "run", "--from", ONEVCS_CLI, "python", "-c",
         "import shutil, sys; sys.stdout.write(shutil.which('onevcs') or '')"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    owner, name = repository.split("/")
    home = work / "onevcs-home"
    home.mkdir(parents=True, exist_ok=True)
    (home / "rules.yml").write_text(
        "version: 4\nrules:\n"
        f"  - match: {{host: github.com, owner: {owner}, name: {name}}}\n    visibility: public\n"
        "default: {publication: change-open, approvals: required}\n",
        encoding="utf-8",
    )
    checkout = work / "onevcs-checkouts" / f"{owner}-{name}"
    checkout.mkdir(parents=True, exist_ok=True)
    (checkout / "README.md").write_text(f"# {name}\n", encoding="utf-8")
    for argv in (
        ["init", "--quiet", "--initial-branch=main"],
        ["add", "--all"],
        ["-c", "user.name=Synthetic", "-c", "user.email=synthetic@example.invalid",
         "commit", "--quiet", "--message", "synthetic identity"],
    ):
        subprocess.run(["git", *argv], cwd=checkout, check=True)
    subprocess.run(
        [onevcs, "register", str(checkout), "--origin", f"https://github.com/{repository}.git"],
        env={**os.environ, "ONEVCS_HOME": str(home)}, check=True, capture_output=True,
    )
    return home, onevcs


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/debug/onetaskgraph")
    parser.add_argument("--items", type=int, default=g.PLANS_BOARD_SCALE)
    parser.add_argument("--cache", action="store_true")
    parser.add_argument("--one-edge", action="store_true")
    parser.add_argument("--preserve", action="store_true")
    parser.add_argument("--instant", action="store_true", help="place every issue at once")
    parser.add_argument("--keep", help="work in this directory and keep it")
    parser.add_argument(
        "--reviewed",
        action="store_true",
        help="record review metadata as `just review-plan` does: every record before the first "
        "copy, then the revised task and the project before the re-copy",
    )
    parser.add_argument(
        "--private",
        action="store_true",
        help="declare the notes source private, screening through the pinned onevcs",
    )
    parser.add_argument(
        "--fixed-double",
        action="store_true",
        help="answer the sub-issue listing with every field its query selects (in memory only)",
    )
    args = parser.parse_args()
    edges = EDGES[:1] if args.one_edge else EDGES
    if args.fixed_double:
        fix_sub_issues()
    host = yaml.safe_load((AIO / "onetaskgraph.yaml").read_text(encoding="utf-8"))
    plans = host["sources"]["plans"]
    binary = str(Path(args.binary).resolve())
    with tempfile.TemporaryDirectory() as scratch:
        work = Path(args.keep or scratch)
        work.mkdir(parents=True, exist_ok=True)
        notes = work / "notes"
        author(notes, edges)
        placement = () if args.instant else g.MEASURED_PLACEMENT_SECONDS
        with g._serving_board(placement=placement) as remote:
            g.populate(g.BOARD, args.items)
            board = {"plugin": "github-projects", "config": dict(plans["config"])}
            board["config"]["endpoint"] = remote["ONETASKGRAPH_SOURCES__PLANS__CONFIG__ENDPOINT"]
            if args.cache:
                board["cache"] = {"enabled": True, "dir": str(work / "cache")}
            notes_source: dict[str, object] = {"plugin": "local-md", "config": {"root": str(notes)}}
            config: dict[str, object] = {"sources": {"notes": notes_source, "plans": board}}
            onevcs_home = None
            if args.private:
                notes_source["visibility"] = "private"
                onevcs_home, onevcs = synthetic_onevcs(work, plans["config"]["repository"])
                config["write_policy"] = {
                    "visibility_command": [onevcs, "boundary", "inspect", "--input", "-"],
                    "check_command": [
                        onevcs, "boundary", "check", "--destination", "public", "--input", "-",
                    ],
                }
            (work / "onetaskgraph.yaml").write_text(json.dumps(config), encoding="utf-8")
            environment = {
                "PATH": os.environ["PATH"],
                "HOME": str(work),
                "XDG_CONFIG_HOME": str(work / "config"),
                "XDG_CACHE_HOME": str(work / "xdg-cache"),
                "GH_PROJECTS_TOKEN": remote["GH_PROJECTS_TOKEN"],
            }
            if onevcs_home is not None:
                environment["ONEVCS_HOME"] = str(onevcs_home)
            if "SPIKE_FIXES" in os.environ:
                environment["SPIKE_FIXES"] = os.environ["SPIKE_FIXES"]

            def run(*argv: str) -> str:
                done = subprocess.run(
                    [binary, *argv, "--json"],
                    cwd=work,
                    env=environment,
                    capture_output=True,
                    text=True,
                    check=False,
                )
                if done.returncode != 0:
                    raise SystemExit(f"{argv}: {done.stdout}{done.stderr}")
                if args.keep:
                    with (work / "reports.jsonl").open("a", encoding="utf-8") as kept:
                        kept.write(json.dumps({"argv": argv, "out": done.stdout}) + "\n")
                return done.stdout

            def copy(name: str) -> None:
                g._GitHubFixture.requests.clear()
                phases = []
                for kind, item in (("project", "notes:P"), ("document", "notes:D")):
                    before = len(g._GitHubFixture.requests)
                    extra = ["--status-policy", "preserve-destination"] if (
                        args.preserve and kind == "project"
                    ) else []
                    run(kind, "copy", item, "--to", "plans", *extra)
                    sent = g._GitHubFixture.requests[before:]
                    phases.append((kind, [str(request.operation) for request in sent]))
                requests = list(g._GitHubFixture.requests)
                points = sum(g._request_points(request) for request in requests)
                print(
                    json.dumps(
                        {
                            "copy": name,
                            "points": points,
                            "operations": dict(Counter(str(r.operation) for r in requests)),
                            "phases": phases,
                        }
                    )
                )

            def review(round_: int, ids: list[tuple[str, str]]) -> None:
                if not args.reviewed:
                    return
                for kind, item in ids:
                    value = json.dumps({"by": "planning-closeout", "key": f"{item}-{round_}",
                                        "reviewed_at": f"2026-10-26T13:0{round_}:00+00:00"})
                    run(kind, "metadata", "set", item, "orchestrator.plan-review", value)

            review(0, [("task", f"notes:T-{n}") for n in range(1, 9)] + [("project", "notes:P")])
            copy("first")
            write_task(notes, 4, "Step 4 of the plan, its body.\nAn edit.\n", edges)
            review(1, [("task", "notes:T-4"), ("project", "notes:P")])
            copy("changed")
            copy("unchanged")


if __name__ == "__main__":
    main()
