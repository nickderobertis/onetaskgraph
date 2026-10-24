"""`run.py proxy-refusal`: the recording proxy, exactly as a live scenario starts it, refusing
every write that names a destination other than the test board — offline.

The proxy's upstream is a loopback stand-in that records what reaches it, so a write that
leaves would show there. It is allowed the test board's two ids (`--writes allowed`, as every
live scenario runs it) and is sent:

- a `createIssue` naming another repository;
- an `addProjectV2ItemById` naming another project;
- a `createIssue` naming the test board's repository but another project in `projectV2Ids`;
- one aliased request whose second mutation names another repository;
- the same `createIssue` naming only the test board's ids, which must leave (to the stand-in).

`refusal.json` keeps each request's status, the error type the proxy answered with, and how
many requests reached the stand-in.
"""

from __future__ import annotations

import http.server
import json
import subprocess
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO_ID, PROJECT_ID = "R_test_board_repository", "PVT_test_board_project"
ELSEWHERE_REPO, ELSEWHERE_PROJECT = "R_another_repository", "PVT_another_project"
CREATE = "mutation($input: CreateIssueInput!) { createIssue(input: $input) { issue { id } } }"
ADD = "mutation($p: ID!, $c: ID!) { addProjectV2ItemById(input: {projectId: $p, contentId: $c}) { item { id } } }"
ALIASED = (
    "mutation($a: CreateIssueInput!, $b: CreateIssueInput!) {"
    " a: createIssue(input: $a) { issue { id } } b: createIssue(input: $b) { issue { id } } }"
)
CASES = (
    (
        "createIssue in another repository",
        CREATE,
        {"input": {"repositoryId": ELSEWHERE_REPO, "title": "x"}},
        403,
    ),
    (
        "addProjectV2ItemById to another project",
        ADD,
        {"p": ELSEWHERE_PROJECT, "c": "I_any"},
        403,
    ),
    (
        "createIssue on the test repository placed on another project",
        CREATE,
        {
            "input": {
                "repositoryId": REPO_ID,
                "title": "x",
                "projectV2Ids": [ELSEWHERE_PROJECT],
            }
        },
        403,
    ),
    (
        "aliased request, second mutation in another repository",
        ALIASED,
        {
            "a": {"repositoryId": REPO_ID, "title": "x"},
            "b": {"repositoryId": ELSEWHERE_REPO, "title": "y"},
        },
        403,
    ),
    (
        "createIssue naming only the test board",
        CREATE,
        {
            "input": {
                "repositoryId": REPO_ID,
                "title": "x",
                "projectV2Ids": [PROJECT_ID],
            }
        },
        200,
    ),
)


def run(run_dir: Path, python: str, proxy: Path) -> dict:
    seen: list[str] = []

    class StandIn(http.server.BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            self.rfile.read(int(self.headers.get("Content-Length") or 0))
            seen.append(self.path)
            payload = b'{"data": {}}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *args) -> None:
            pass

    upstream = http.server.ThreadingHTTPServer(("127.0.0.1", 0), StandIn)
    threading.Thread(target=upstream.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{upstream.server_address[1]}"
    port_file = run_dir / "proxy.port"
    argv = [
        python,
        str(proxy),
        "--log",
        str(run_dir / "proxy.jsonl"),
        "--port-file",
        str(port_file),
    ]
    argv += ["--upstream", base, "--uploads", base, "--writes", "allowed"]
    argv += ["--allow", REPO_ID, "--allow", PROJECT_ID]
    process = subprocess.Popen(argv)
    try:
        for _ in range(100):
            if port_file.exists() and port_file.read_text():
                break
            time.sleep(0.05)
        url = f"http://127.0.0.1:{port_file.read_text()}/graphql"
        results = []
        for label, query, variables, expected in CASES:
            before = len(seen)
            request = urllib.request.Request(
                url,
                data=json.dumps({"query": query, "variables": variables}).encode(),
                headers={
                    "Authorization": "Bearer placeholder",
                    "Content-Type": "application/json",
                },
            )
            try:
                with urllib.request.urlopen(request, timeout=30) as response:
                    status, body = response.status, json.load(response)
            except urllib.error.HTTPError as error:
                status, body = error.code, json.loads(error.read() or b"{}")
            results.append(
                {
                    "case": label,
                    "status": status,
                    "expected": expected,
                    "error_types": [e.get("type") for e in body.get("errors") or []],
                    "reached_upstream": len(seen) - before,
                }
            )
    finally:
        process.terminate()
        process.wait(timeout=10)
        upstream.shutdown()
    out = {
        "cases": results,
        "all_as_expected": all(
            r["status"] == r["expected"]
            and r["reached_upstream"] == (1 if r["expected"] == 200 else 0)
            for r in results
        ),
    }
    (run_dir / "refusal.json").write_text(json.dumps(out, indent=1))
    return out
