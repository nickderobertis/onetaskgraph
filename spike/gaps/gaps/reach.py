"""Synthetic source-reachability cases, offline: the real CLI and the real onevcs.

Each case copies the brief's 10-item plan from one folder of Markdown to another declared
public, with `write_policy` naming the real onevcs boundary commands (through `gaps.shim`)
against an onevcs home of invented identities. Two private sources are configured and
unrelated to the plan — a Linear source and a GitHub board, both synthetic — and both point at
one loopback `Recorder` answering every request 503: a configured private service held
unavailable. No real private source is configured or read.

- `reach-public`: the plan, nothing naming the private sources.
- `reach-private-item`: one task declared `classification: private`.
- `reach-private-ref`: one task depending on an item of the private Linear source.
- `reach-unresolvable-ref`: one task depending on an item of a source nothing configures.
"""

from __future__ import annotations

import asyncio
import hashlib
import json
import os
import sys
import time
from dataclasses import dataclass, field
from enum import StrEnum
from pathlib import Path

from gaps import boundary, proc
from gaps.boundary import Identity, Visibility
from gaps.harness import workload
from gaps.json_shape import Json, ShapeError, encode, load_lines, obj, parse, text
from gaps.shim import Command, ScreenCall

PUBLIC = Identity("synthco", "public-widgets", Visibility.PUBLIC)
PRIVATE = Identity("synthco", "secret-gadgets", Visibility.PRIVATE)
PLAN = "reach-plan"
PACKAGE_ROOT = Path(__file__).resolve().parents[1]
STEP_TIMEOUT = 600.0


class Case(StrEnum):
    """The synthetic reachability cases."""

    PUBLIC = "reach-public"
    PRIVATE_ITEM = "reach-private-item"
    PRIVATE_REF = "reach-private-ref"
    UNRESOLVABLE_REF = "reach-unresolvable-ref"


@dataclass(frozen=True)
class Recorded:
    """One request a private double received."""

    at: float
    method: str
    path: str
    operation: str
    query_head: str
    bytes: int


def operation_of(body: bytes) -> tuple[str, str]:
    """A GraphQL request's operation, named by its first field when anonymous, and its head."""
    head = " ".join(body.decode(errors="replace").split())[:200]
    try:
        query = text(
            obj(parse(body.decode(errors="replace"), "request"), "request").get("query"), "q"
        )
    except ShapeError:
        return "not GraphQL", head
    words = query.replace("{", " { ").replace("(", " ( ").split()
    match words:
        case [("query" | "mutation") as kind, "(", *_]:
            selection = query.split("{", 1)[1].split("(")[0].split("{")[0].strip()
            return f"{kind} <{selection}>", head
        case [("query" | "mutation") as kind, name, *_]:
            return f"{kind} {name}", head
        case _:
            return "anonymous query", head


@dataclass
class Recorder:
    """A loopback HTTP server answering every request 503, recording each one."""

    requests: list[Recorded] = field(default_factory=list)
    url: str = ""

    async def handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        """Read one request, record it, answer 503 and close."""
        try:
            head = await reader.readuntil(b"\r\n\r\n")
            lines = head.decode("latin-1").split("\r\n")
            method, path, *_ = lines[0].split(" ")
            length = next(
                (
                    int(line.split(":", 1)[1])
                    for line in lines
                    if line.lower().startswith("content-length:")
                ),
                0,
            )
            body = await reader.readexactly(length) if length else b""
            operation, query_head = operation_of(body)
            self.requests.append(
                Recorded(time.time(), method, path, operation, query_head, len(body))
            )
            writer.write(
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\n"
                b"Content-Type: application/json\r\nConnection: close\r\n\r\n{}"
            )
            await writer.drain()
        except (asyncio.IncompleteReadError, ValueError):
            pass
        finally:
            writer.close()


def write_plan(root: Path, case: Case) -> None:
    """The brief's 10-item plan naming only the synthetic public repository.

    Task t1 carries the one change `case` makes.
    """
    workload.write_plan(root, PLAN, 0)
    for path in root.rglob("*.md"):
        path.write_text(path.read_text().replace(workload.REPOSITORY, PUBLIC.qualified))
    links = "\n".join(
        f"- [{workload.task_title(PLAN, n)}]({root / 'tasks' / PLAN / f't{n}.md'})"
        for n in range(1, 9)
    )
    document = workload.front_matter({"title": f"Design: {PLAN}", "project": PLAN})
    (root / "documents" / f"{PLAN}-design.md").write_text(f"{document}\n# Design\n\n{links}\n")
    task = root / "tasks" / PLAN / "t1.md"
    content = task.read_text()
    match case:
        case Case.PRIVATE_ITEM:
            content = content.replace("\nstatus:", '\nclassification: "private"\nstatus:', 1)
        case Case.PRIVATE_REF | Case.UNRESOLVABLE_REF:
            far = "priv-linear:SYN-1" if case is Case.PRIVATE_REF else "ghost:G-1"
            edge = f'\ndepends_on: [{{"id": "{far}", "item": "task"}}]\nstatus:'
            content = content.replace("\nstatus:", edge, 1)
        case Case.PUBLIC:
            pass
    task.write_text(content)


def configuration(root: Path, recorder: str, shim: list[str]) -> dict[str, Json]:
    """The store configuration: two folders, the two unavailable private doubles, the policy."""
    return {
        "default_sources": ["src"],
        "write_policy": {
            "visibility_command": [*shim, "inspect", "--input", "-"],
            "check_command": [*shim, "check", "--destination", "public", "--input", "-"],
        },
        "sources": {
            "src": {
                "plugin": "local-md",
                "config": {"root": str(root / "src")},
                "visibility": "public",
            },
            "dest": {
                "plugin": "local-md",
                "config": {"root": str(root / "dest")},
                "visibility": "public",
            },
            "priv-linear": {
                "plugin": "linear",
                "visibility": "private",
                "config": {
                    "team": "SYN",
                    "endpoint": f"{recorder}/graphql",
                    "api_key_env": "SYN_LINEAR_KEY",
                },
            },
            "priv-board": {
                "plugin": "github-projects",
                "visibility": "private",
                "config": {
                    "owner": PRIVATE.owner,
                    "project_number": 9,
                    "repository": PRIVATE.key,
                    "token_env": "SYN_GH_TOKEN",
                    "endpoint": f"{recorder}/graphql",
                },
            },
        },
    }


def tree(root: Path) -> dict[str, str]:
    """Every file under `root` by its digest."""
    return {
        str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(root.rglob("*"))
        if p.is_file()
    }


@dataclass(frozen=True)
class Step:
    """One CLI command of a case."""

    step: str
    argv: tuple[str, ...]
    returncode: int
    seconds: float
    stderr_tail: tuple[str, ...]
    output_names_the_reference: bool
    output_says_withheld: bool
    recorder_requests: tuple[Recorded, ...]


@dataclass(frozen=True)
class Screens:
    """The `write_policy` calls the store made, counted."""

    inspect: int
    check: int
    distinct_inspect_inputs: int
    seconds: float
    verdicts: tuple[tuple[str, int], ...]


@dataclass(frozen=True)
class ReachResult:
    """Everything one case observed."""

    case: Case
    binary: str
    onevcs: str
    steps: tuple[Step, ...]
    private_requests: int
    destination_changed: bool
    destination_files: int
    screens: Screens

    def to_json(self) -> Json:
        """The result as `result.json` holds it."""
        return encode(self)

    @property
    def succeeded(self) -> bool:
        """Whether every step exited 0."""
        return all(step.returncode == 0 for step in self.steps)


def screen_log(run_dir: Path, step: str) -> Path:
    """Where one copy command's `write_policy` calls are recorded."""
    return run_dir / f"screen-{step}.jsonl"


def screens(logs: list[Path]) -> tuple[list[ScreenCall], Screens]:
    """The recorded `write_policy` calls of `logs`, validated, and their counts."""
    calls = [ScreenCall.parse(line, f"{log}") for log in logs for line in load_lines(log)]
    inspects = [c for c in calls if c.name is Command.INSPECT]
    return calls, Screens(
        inspect=len(inspects),
        check=sum(1 for c in calls if c.name is Command.CHECK),
        distinct_inspect_inputs=len({c.input_sha256 for c in inspects}),
        seconds=round(sum(c.end - c.start for c in calls), 3),
        verdicts=tuple(sorted({(c.name.value, c.returncode) for c in calls})),
    )


async def run(run_dir: Path, case: Case, exe: Path, onevcs: Path) -> ReachResult:
    """Run one case into `run_dir`."""
    root = run_dir / "sandbox"
    for directory in ("src", "dest", "xdg"):
        (root / directory).mkdir(parents=True)
    home = await boundary.make_home(root, onevcs, (PUBLIC, PRIVATE))
    write_plan(root / "src", case)
    recorder = Recorder()
    server = await asyncio.start_server(recorder.handle, "127.0.0.1", 0)
    recorder.url = f"http://127.0.0.1:{server.sockets[0].getsockname()[1]}"
    env = {
        k: v for k, v in os.environ.items() if not k.startswith(("ONETASKGRAPH_", "GH_", "LINEAR_"))
    }
    env |= {
        "XDG_CONFIG_HOME": str(root / "xdg"),
        "SYN_LINEAR_KEY": "synthetic-placeholder",
        "SYN_GH_TOKEN": "synthetic-placeholder",
        "PYTHONPATH": os.pathsep.join([str(PACKAGE_ROOT), str(PACKAGE_ROOT.parent / "harness")]),
    } | home.env()
    version = (await proc.check([exe, "--version"])).out.strip()
    before = tree(root / "dest")
    steps: list[Step] = []
    try:
        for name, argv in (
            ("project-copy", ("project", "copy", f"src:{PLAN}", "--to", "dest", "--json")),
            ("document-copy", ("document", "copy", f"src:{PLAN}-design", "--to", "dest", "--json")),
        ):
            # Each command screens into its own log, so the union experiment reads which
            # command made each call from where it was written.
            shim = [sys.executable, "-m", "gaps.shim", str(screen_log(run_dir, name)), str(onevcs)]
            config = configuration(root, recorder.url, [*shim, "boundary"])
            (root / "onetaskgraph.yaml").write_text(json.dumps(config, indent=1))
            seen = len(recorder.requests)
            done = await proc.run([exe, *argv], cwd=root, env=env, timeout=STEP_TIMEOUT)
            said = done.out + done.err
            steps.append(
                Step(
                    step=name,
                    argv=argv,
                    returncode=done.returncode,
                    seconds=round(done.seconds, 3),
                    stderr_tail=tuple(done.err.strip().splitlines()[-6:]),
                    output_names_the_reference=any(w in said for w in ("priv-linear", "SYN-1")),
                    output_says_withheld="withh" in said.lower(),
                    recorder_requests=tuple(recorder.requests[seen:]),
                )
            )
            if done.returncode != 0:
                break
    finally:
        server.close()
        await server.wait_closed()
    _, counted = screens([screen_log(run_dir, step.step) for step in steps])
    after = tree(root / "dest")
    result = ReachResult(
        case=case,
        binary=version,
        onevcs=(await proc.check([onevcs, "--version"])).out.strip(),
        steps=tuple(steps),
        private_requests=len(recorder.requests),
        destination_changed=before != after,
        destination_files=len(after),
        screens=counted,
    )
    (run_dir / "result.json").write_text(json.dumps(result.to_json(), indent=1))
    return result
