"""Synthetic source-reachability and screening cases, offline: `run.py reach-<case>`.

Each case drives the real CLI as a subprocess, with `write_policy` naming the real onevcs
boundary commands against an onevcs home this run owns. That home holds synthetic identities
only (a git repository made in the sandbox, registered under an invented owner and declared
public or private by a rule, so no host is asked), exactly as the engine's own `onevcs.rs`
journeys set one up. No real private source is configured or read: the two private sources
are synthetic, a Linear source and a GitHub board, both pointed at one loopback recorder that
answers every request 503 — a configured private service held unavailable — and records it.

The copy goes from one folder of Markdown to another declared public, so the only hosted APIs
in a case are the recorder's doubles. What a case keeps (`reach.json`): each step's exit code
and its stderr's last lines (synthetic text), every request the recorder received, every
onevcs call with a digest of what it was handed, and whether the destination folder changed.

Cases:

- `reach-public` — a public plan (the brief's 10-item shape) copied while both private sources
  are configured and unavailable. Nothing names them.
- `reach-private-item` — the same plan with one task declared `classification: private`.
- `reach-private-ref` — one task depending on an item of the private Linear source.
- `reach-unresolvable-ref` — one task depending on an item of a source nothing configures.
"""

from __future__ import annotations

import hashlib
import http.server
import json
import os
import subprocess
import threading
import time
from pathlib import Path

import workload

HERE = Path(__file__).resolve().parent
CASES = (
    "reach-public",
    "reach-private-item",
    "reach-private-ref",
    "reach-unresolvable-ref",
)
PUBLIC = ("synthco", "public-widgets")
PRIVATE = ("synthco", "secret-gadgets")
PLAN = "reach-plan"


class Recorder:
    """A loopback HTTP server answering every request 503, recording each one."""

    def __init__(self) -> None:
        self.requests: list[dict] = []
        recorder = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def _answer(self) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                operation = None
                try:
                    query = str(json.loads(body).get("query", ""))
                    words = query.replace("{", " { ").replace("(", " ( ").split()
                    if words and words[0] in ("query", "mutation") and len(words) > 1:
                        operation = f"{words[0]} {words[1]}"
                    elif words:
                        operation = "anonymous query"
                    if operation and operation.endswith(" ("):
                        # Anonymous with variables: name it by its first selected field.
                        selection = query.split("{", 1)[1].split("(")[0].split("{")[0]
                        operation = f"{words[0]} <{selection.strip()}>"
                except (ValueError, AttributeError):
                    pass
                recorder.requests.append(
                    {
                        "at": time.time(),
                        "method": self.command,
                        "path": self.path,
                        "host": self.headers.get("Host"),
                        "operation": operation,
                        # Synthetic configuration only: the query's head names what was asked.
                        "query_head": " ".join(body.decode(errors="replace").split())[
                            :200
                        ],
                        "bytes": len(body),
                    }
                )
                self.send_response(503)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"{}")

            do_GET = do_POST = _answer

            def log_message(self, *args) -> None:  # quiet
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def stop(self) -> None:
        self.server.shutdown()


def onevcs_home(root: Path, onevcs: Path) -> Path:
    """An onevcs home holding the two synthetic identities, declared by rule."""
    home = root / "onevcs-home"
    checkouts = root / "checkouts"
    home.mkdir(parents=True)
    rules = "version: 4\nrules:\n"
    for (owner, name), visibility in ((PUBLIC, "public"), (PRIVATE, "private")):
        rules += (
            f"  - match: {{host: github.com, owner: {owner}, name: {name}}}\n"
            f"    visibility: {visibility}\n"
        )
    rules += "default: {publication: change-open, approvals: required}\n"
    (home / "rules.yml").write_text(rules)
    for owner, name in (PUBLIC, PRIVATE):
        checkout = checkouts / f"{owner}-{name}"
        checkout.mkdir(parents=True)
        (checkout / "README.md").write_text(f"# {name}\n")
        git = ["git", "-c", "user.name=Synthetic", "-c", "user.email=s@example.invalid"]
        for args in (
            ["init", "-q", "-b", "main"],
            ["add", "-A"],
            ["commit", "-qm", "synthetic"],
        ):
            subprocess.run(git + args, cwd=checkout, check=True, capture_output=True)
        subprocess.run(
            [
                onevcs,
                "register",
                checkout,
                "--origin",
                f"https://github.com/{owner}/{name}.git",
            ],
            env=dict(os.environ, ONEVCS_HOME=str(home)),
            check=True,
            capture_output=True,
        )
    return home


def write_plan(root: Path, case: str) -> None:
    """The brief's 10-item shape, every record naming the synthetic public repository, and
    the one change the case makes."""
    workload.write_plan(root, PLAN, 0)
    public = f"github.com/{PUBLIC[0]}/{PUBLIC[1]}"
    for path in root.rglob("*.md"):
        path.write_text(path.read_text().replace(workload.REPOSITORY, public))
    document = (
        workload.front_matter({"title": f"Design: {PLAN}", "project": PLAN})
        + "\n# Design\n\n"
        + "\n".join(
            f"- [{workload.task_title(PLAN, n)}]({root / 'tasks' / PLAN / f't{n}.md'})"
            for n in range(1, 9)
        )
        + "\n"
    )
    (root / "documents" / f"{PLAN}-design.md").write_text(document)
    task = root / "tasks" / PLAN / "t1.md"
    text = task.read_text()
    # llmlint: ignore[structural_pattern_matching] This repository declares requires-python >=3.9 and ruff targets it; `match` is a syntax error there.
    if case == "reach-private-item":
        text = text.replace("\nstatus:", '\nclassification: "private"\nstatus:', 1)
    elif case in ("reach-private-ref", "reach-unresolvable-ref"):
        far = "priv-linear:SYN-1" if case == "reach-private-ref" else "ghost:G-1"
        text = text.replace(
            "\nstatus:",
            f'\ndepends_on: [{{"id": "{far}", "item": "task"}}]\nstatus:',
            1,
        )
    task.write_text(text)


def config(root: Path, recorder: str, shim: list[str]) -> dict:
    return {
        "default_sources": ["src"],
        "write_policy": {
            "visibility_command": shim + ["inspect", "--input", "-"],
            "check_command": shim
            + ["check", "--destination", "public", "--input", "-"],
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
            # The unrelated private services: synthetic, configured, unavailable.
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
                    "owner": PRIVATE[0],
                    "project_number": 9,
                    "repository": f"{PRIVATE[0]}/{PRIVATE[1]}",
                    "token_env": "SYN_GH_TOKEN",
                    "endpoint": f"{recorder}/graphql",
                },
            },
        },
    }


def tree(root: Path) -> dict[str, str]:
    return {
        str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(root.rglob("*"))
        if p.is_file()
    }


def run(run_dir: Path, case: str, exe: Path, onevcs: Path, python: str) -> dict:
    root = run_dir / "sandbox"
    (root / "src").mkdir(parents=True)
    (root / "dest").mkdir()
    (root / "xdg").mkdir()
    home = onevcs_home(root, onevcs)
    write_plan(root / "src", case)
    recorder = Recorder()
    shim = [
        python,
        str(HERE / "screen_shim.py"),
        str(run_dir / "screen.jsonl"),
        str(onevcs),
        "boundary",
    ]
    (root / "onetaskgraph.yaml").write_text(
        json.dumps(config(root, recorder.url, shim), indent=1)
    )
    env = {
        k: v
        for k, v in os.environ.items()
        if not k.startswith(("ONETASKGRAPH_", "GH_", "LINEAR_"))
    }
    env |= {
        "XDG_CONFIG_HOME": str(root / "xdg"),
        "ONEVCS_HOME": str(home),
        "SYN_LINEAR_KEY": "synthetic-placeholder",
        "SYN_GH_TOKEN": "synthetic-placeholder",
    }
    version = subprocess.run(
        [exe, "--version"], capture_output=True, text=True, check=False
    ).stdout.strip()
    before = tree(root / "dest")
    steps = []
    try:
        for name, argv in (
            (
                "project-copy",
                ["project", "copy", f"src:{PLAN}", "--to", "dest", "--json"],
            ),
            (
                "document-copy",
                ["document", "copy", f"src:{PLAN}-design", "--to", "dest", "--json"],
            ),
        ):
            seen = len(recorder.requests)
            start = time.time()
            done = subprocess.run(
                [exe, *argv],
                cwd=root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            steps.append(
                {
                    "step": name,
                    "argv": argv,
                    "returncode": done.returncode,
                    "seconds": round(time.time() - start, 3),
                    "stderr_tail": done.stderr.strip().splitlines()[-6:],
                    # Whether the command's own output says anything of the private
                    # reference: its id, its source, or the word the engine withholds with.
                    "output_names_the_reference": any(
                        word in done.stdout + done.stderr
                        for word in ("priv-linear", "SYN-1")
                    ),
                    "output_says_withheld": "withh"
                    in (done.stdout + done.stderr).lower(),
                    "recorder_requests": recorder.requests[seen:],
                }
            )
            print(
                f"  {name}: exit {done.returncode}, {len(recorder.requests) - seen} request(s) to the private doubles",
                flush=True,
            )
            if done.returncode not in (0, 4):
                break
    finally:
        recorder.stop()
    screens = (
        [
            json.loads(line)
            for line in (run_dir / "screen.jsonl").read_text().splitlines()
        ]
        if (run_dir / "screen.jsonl").exists()
        else []
    )
    after = tree(root / "dest")
    result = {
        "case": case,
        "binary": version,
        "onevcs": subprocess.run(
            [onevcs, "--version"], capture_output=True, text=True, check=False
        ).stdout.strip(),
        "steps": steps,
        "private_requests": len(recorder.requests),
        "destination_changed": before != after,
        "destination_files": len(after),
        "screens": {
            "inspect": sum(1 for s in screens if s["name"] == "inspect"),
            "check": sum(1 for s in screens if s["name"] == "check"),
            "distinct_inspect_inputs": len(
                {s["input_sha256"] for s in screens if s["name"] == "inspect"}
            ),
            "seconds": round(sum(s["end"] - s["start"] for s in screens), 3),
            "verdicts": sorted({(s["name"], s["returncode"]) for s in screens}),
        },
    }
    (run_dir / "reach.json").write_text(json.dumps(result, indent=1))
    return result


if __name__ == "__main__":  # pragma: no cover - run through run.py
    raise SystemExit("run through spike/copy-timing.sh reach-<case>")
