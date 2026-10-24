"""The spike harness's own tests: the recording proxy driven as a subprocess against a
loopback stand-in for GitHub, the sanitizer over what it recorded, the quota rule, the
workload generator and the attribution sweep.

Run: `.venv/bin/pytest spike/tests` (stdlib only; no credential, no network).
"""

from __future__ import annotations

import json
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import ClassVar

import pytest

HARNESS = Path(__file__).resolve().parent.parent / "harness"
sys.path.insert(0, str(HARNESS))

import analyze
import consumption
import quota
import sanitize
import workload

BOARD = json.loads((HARNESS / "documents.json").read_text())["BOARD"]
NODE = "I_kwDOTXCWqs8AAAABWeUpaQ"


class StandIn(BaseHTTPRequestHandler):
    """Answers like GitHub: a GraphQL page naming a node, its limit headers, an upload."""

    protocol_version = "HTTP/1.1"
    seen: ClassVar[list[tuple[str, str, bytes]]] = []

    def log_message(self, *args: object) -> None:
        return

    def answer(self, status: int, payload: dict) -> None:
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("x-ratelimit-limit", "5000")
        self.send_header("x-ratelimit-remaining", "4990")
        self.send_header("x-ratelimit-used", "10")
        self.send_header("x-ratelimit-resource", "graphql")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:
        body = self.rfile.read(int(self.headers["Content-Length"]))
        StandIn.seen.append((self.command, self.path, body))
        if self.path.startswith("/user-attachments/"):
            self.answer(201, {"url": "https://github.com/user-attachments/assets/x"})
            return
        self.answer(
            200,
            {
                "data": {
                    "rateLimit": {"cost": 3, "remaining": 4990},
                    "node": {
                        "id": NODE,
                        "title": "feat(spike): seeded task 1 of bcs-x",
                    },
                }
            },
        )

    def do_GET(self) -> None:
        StandIn.seen.append((self.command, self.path, b""))
        self.answer(
            200, {"id": 1, "full_name": "nickderobertis/onetaskgraph-live-scratch"}
        )


@pytest.fixture
def github():
    StandIn.seen = []
    server = ThreadingHTTPServer(("127.0.0.1", 0), StandIn)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{server.server_address[1]}"
    server.shutdown()


@pytest.fixture
def proxy(tmp_path: Path, github: str, request: pytest.FixtureRequest):
    writes, allowed = getattr(request, "param", ("any", []))
    port_file = tmp_path / "port"
    argv = [
        sys.executable,
        str(HARNESS / "proxy.py"),
        "--log",
        str(tmp_path / "raw.jsonl"),
    ]
    argv += ["--port-file", str(port_file), "--upstream", github, "--uploads", github]
    argv += ["--writes", writes] + [a for i in allowed for a in ("--allow", i)]
    process = subprocess.Popen(argv)
    for _ in range(100):
        if port_file.exists() and port_file.read_text():
            break
        time.sleep(0.05)
    yield f"http://127.0.0.1:{port_file.read_text()}", tmp_path
    process.terminate()
    process.wait(timeout=10)


def post(url: str, payload: dict) -> tuple[int, dict]:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode(),
        headers={"Authorization": "Bearer secret", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read())


def raw(directory: Path) -> list[dict]:
    return [
        json.loads(line) for line in (directory / "raw.jsonl").read_text().splitlines()
    ]


def test_the_proxy_forwards_and_records_a_request_by_the_plugin_constant_it_came_from(
    proxy,
):
    url, directory = proxy
    priced = BOARD.replace("{", "{rateLimit{cost} ", 1)
    status, answer = post(
        f"{url}/graphql", {"query": priced, "variables": {"first": 100}}
    )
    assert status == 200
    assert answer["data"]["node"]["id"] == NODE
    assert StandIn.seen[0][1] == "/graphql"
    assert b"Bearer" not in json.dumps(raw(directory)).encode()
    (record,) = raw(directory)
    assert (record["kind"], record["op"], record["cost"], record["status"]) == (
        "query",
        "BOARD",
        3,
        200,
    )
    assert record["limits"]["x-ratelimit-used"] == "10"
    assert record["end"] >= record["start"]
    assert (directory / "raw.bodies" / "000001.resp").exists()


def test_an_upload_goes_to_the_uploads_host_and_a_repository_read_to_the_api(proxy):
    url, directory = proxy
    request = urllib.request.Request(
        f"{url}/user-attachments/assets?name=a.png", data=b"\x89PNG", method="POST"
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        assert response.status == 201
    with urllib.request.urlopen(f"{url}/repos/o/r", timeout=10) as response:
        assert response.status == 200
    ops = [r["op"] for r in raw(directory)]
    assert ops == ["POST /user-attachments/assets", "GET /repos/o/r"]


@pytest.mark.parametrize(
    "proxy", [("allowed", ["R_scratch", "PVT_board"])], indirect=True
)
def test_a_mutation_naming_another_repository_never_leaves(proxy):
    url, directory = proxy
    create = "mutation($input: CreateIssueInput!) { createIssue(input: $input) { issue { id } } }"
    status, answer = post(
        f"{url}/graphql",
        {"query": create, "variables": {"input": {"repositoryId": "R_core"}}},
    )
    assert status == 403
    assert answer["errors"][0]["type"] == "SPIKE_REFUSED"
    assert StandIn.seen == []
    status, _ = post(
        f"{url}/graphql",
        {"query": create, "variables": {"input": {"repositoryId": "R_scratch"}}},
    )
    assert status == 200
    assert [r["status"] for r in raw(directory)] == [403, 200]


@pytest.mark.parametrize("proxy", [("none", [])], indirect=True)
def test_a_read_only_run_sends_no_mutation_and_no_upload(proxy):
    url, _ = proxy
    status, _ = post(
        f"{url}/graphql",
        {"query": "mutation { deleteIssue(input: {}) { clientMutationId } }"},
    )
    assert status == 403
    request = urllib.request.Request(
        f"{url}/user-attachments/assets", data=b"x", method="POST"
    )
    with pytest.raises(urllib.error.HTTPError):
        urllib.request.urlopen(request, timeout=10)
    status, _ = post(f"{url}/graphql", {"query": BOARD, "variables": {}})
    assert status == 200
    assert [s[1] for s in StandIn.seen] == ["/graphql"]


def test_sanitized_logs_carry_timings_names_and_limits_and_no_id_or_title(
    proxy, tmp_path
):
    url, directory = proxy
    post(f"{url}/graphql", {"query": BOARD, "variables": {"id": NODE}})
    run = tmp_path / "run"
    run.mkdir()
    (run / "proxy.jsonl").write_text((directory / "raw.jsonl").read_text())
    (run / "steps.jsonl").write_text(
        json.dumps(
            {
                "step": "s",
                "argv": ["/x/onetaskgraph", "task", "show", f"plans:{NODE}"],
                "start": 1.0,
                "end": 2.0,
                "returncode": 0,
            }
        )
        + "\n"
    )
    out = sanitize.write(run, tmp_path / "out")
    text = "".join(p.read_text() for p in out.iterdir())
    assert NODE not in text and "seeded task" not in text
    (record,) = [
        json.loads(line) for line in (out / "requests.jsonl").read_text().splitlines()
    ]
    assert (record["op"], record["limits"]["used"], record["path"]) == (
        "BOARD",
        10,
        "/graphql",
    )
    step = json.loads((out / "steps.jsonl").read_text())
    assert step["argv"] == ["task", "show", "plans:<id>"]


def test_the_leak_scan_names_a_node_id_or_a_title_wherever_it_appears(tmp_path):
    (tmp_path / "a.jsonl").write_text('{"op": "BOARD"}\n{"x": "' + NODE + '"}\n')
    (tmp_path / "b.json").write_text('{"t": "feat(spike): seeded task 3"}\n')
    assert sanitize.leaks(tmp_path) == ["a.jsonl:2", "b.json:1"]
    assert not sanitize.NODE_ID.search("MAX_NODE_LIMIT_EXCEEDED")


def reading(remaining: int, used: int, reset: float) -> dict:
    bucket = {"limit": 5000, "remaining": remaining, "used": used, "reset": reset}
    return {"core": dict(bucket), "graphql": dict(bucket)}


def test_quota_pauses_under_forty_percent_until_the_reset_and_records_both(tmp_path):
    log = tmp_path / "quota.jsonl"
    now = [1000.0]
    answers = [reading(1500, 3500, 1600.0), reading(5000, 0, 5200.0)]
    slept: list[float] = []

    def sleep(seconds: float) -> None:
        slept.append(seconds)
        now[0] += seconds

    entry = quota.gate(
        "t", log, "s", ("graphql",), lambda _: answers.pop(0), sleep, lambda: now[0]
    )
    assert slept == [605.0]
    assert entry["verdicts"]["graphql"]["proceed"]
    events = [json.loads(line)["event"] for line in log.read_text().splitlines()]
    assert events == ["reading", "pause", "reading"]


def test_quota_proceeds_under_forty_percent_when_the_burn_resets_it_first():
    previous = {"at": 0.0, "buckets": reading(1900, 3100, 400.0)}
    ok, why = quota.verdict("graphql", reading(1890, 3110, 400.0), 100.0, previous)
    assert ok, why
    ok, _ = quota.verdict("graphql", reading(1000, 4000, 4000.0), 100.0, previous)
    assert not ok


def test_the_workload_is_the_briefs_shape_and_names_only_the_scratch_repository(
    tmp_path,
):
    workload.main_args(
        tmp_path / "md", "bcs-x", tmp_path / "out", variant=0, rewrite=False
    )
    tasks = sorted((tmp_path / "md/tasks/bcs-x").glob("*.md"))
    assert len(tasks) == 8
    texts = [t.read_text() for t in tasks]
    assert sum(t.split("\n---\n", 1)[0].count("bcs-x/t") for t in texts) == 10
    for text in [*texts, (tmp_path / "md/projects/bcs-x.md").read_text()]:
        assert (
            'repositories: ["github.com/nickderobertis/onetaskgraph-live-scratch"]'
            in text
        )
        assert "metadata:\n  " in text
    sizes = [len(t.split("\n---\n", 1)[1].encode()) for t in texts]
    assert all(45_000 <= s <= 49_500 for s in sizes)
    assert 100_000 <= (tmp_path / "out/plan-dag.png").stat().st_size <= 108_000
    before = texts[0]
    workload.main_args(
        tmp_path / "md", "bcs-x", tmp_path / "out", variant=1, rewrite=True
    )
    after = tasks[0].read_text()
    assert after.split("\n---\n", 1)[0] == before.split("\n---\n", 1)[0]
    assert after != before


def test_each_instant_goes_to_one_category_and_uncovered_time_is_unattributed():
    intervals = [
        (0.0, 10.0, "CLI (configuration, engine build, output)"),
        (1.0, 4.0, "engine and local-source work"),
        (2.0, 3.0, "writes"),
        (3.0, 3.5, "pacing waits"),
    ]
    split = analyze.sweep(intervals, 0.0, 12.0)
    assert split == {
        "CLI (configuration, engine build, output)": 7.0,
        "engine and local-source work": 1.5,
        "writes": 1.0,
        "pacing waits": 0.5,
        "unattributed": 2.0,
    }


class Limits(BaseHTTPRequestHandler):
    """`/rate_limit` claiming a full GraphQL bucket while GraphQL's own `rateLimit` says not."""

    def log_message(self, *args: object) -> None:
        return

    def reply(self, payload: dict) -> None:
        body = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        bucket = {"limit": 5000, "used": 201, "remaining": 4799, "reset": 2000}
        self.reply({"resources": {"core": bucket, "graphql": bucket}})

    def do_POST(self) -> None:
        self.rfile.read(int(self.headers["Content-Length"]))
        own = {
            "limit": 5000,
            "used": 4117,
            "remaining": 883,
            "resetAt": "1970-01-01T00:33:20Z",
        }
        self.reply({"data": {"rateLimit": own}})


def test_the_graphql_bucket_is_held_to_the_lower_of_its_two_readings():
    server = ThreadingHTTPServer(("127.0.0.1", 0), Limits)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        now = quota.read("t", f"http://127.0.0.1:{server.server_address[1]}")
    finally:
        server.shutdown()
    assert now["graphql"]["remaining"] == 883
    assert now["graphql (GET /rate_limit)"]["remaining"] == 4799
    assert now["core"]["remaining"] == 4799
    ok, why = quota.verdict("graphql", now, 1000.0, None)
    assert not ok, why


@pytest.mark.parametrize("proxy", [("none", [])], indirect=True)
def test_an_operation_the_run_refuses_never_leaves(proxy, tmp_path):
    url, directory = proxy
    search = json.loads((HARNESS / "documents.json").read_text())["SEARCH_ISSUES"]
    process = subprocess.Popen(
        [sys.executable, str(HARNESS / "proxy.py"), "--log", str(tmp_path / "r2.jsonl")]
        + [
            "--port-file",
            str(tmp_path / "p2"),
            "--upstream",
            url,
            "--refuse-op",
            "SEARCH_ISSUES",
        ]
    )
    try:
        for _ in range(100):
            if (tmp_path / "p2").exists() and (tmp_path / "p2").read_text():
                break
            time.sleep(0.05)
        second = f"http://127.0.0.1:{(tmp_path / 'p2').read_text()}"
        status, answer = post(f"{second}/graphql", {"query": search, "variables": {}})
        assert (status, answer["errors"][0]["type"]) == (403, "SPIKE_REFUSED")
        status, _ = post(f"{second}/graphql", {"query": BOARD, "variables": {}})
        assert status == 200
    finally:
        process.terminate()
        process.wait(timeout=10)
    assert [s[1] for s in StandIn.seen] == ["/graphql"]
    assert [r["op"] for r in raw(directory)] == ["BOARD"]


def rendered_folder(root: Path, document: str) -> None:
    workload.write_plan(root, "bcs-x", 0)
    front = "---\ntitle: 'Design: bcs-x'\nproject: bcs-x\n---\n"
    (root / "documents" / "bcs-x-design.md").write_text(
        front + document + "\n<!-- onetaskgraph:template-answers\nwhat: x\n-->\n",
        encoding="utf-8",
    )


def test_rendered_sizes_reads_each_body_in_bytes_and_characters(tmp_path):
    # 30,000 one-byte and 1,500 three-byte characters, and the line ending the body.
    rendered_folder(tmp_path, "x" * 30_000 + "—" * 1_500)
    sizes = workload.rendered_sizes(tmp_path, "bcs-x")
    assert sizes["document"] == (34_501, 31_501)
    assert 49_000 <= sizes["project"][0] <= 52_000
    assert all(44_500 <= sizes[f"task t{n}"][0] <= 50_000 for n in range(1, 9))


@pytest.mark.parametrize(
    "document",
    ["x" * 20_000, "x" * 77_000],
    ids=["a 20 KB body", "a 77 KB body, which the manager ruled out"],
)
def test_rendered_sizes_refuses_a_document_that_is_not_the_briefs(tmp_path, document):
    rendered_folder(tmp_path, document)
    with pytest.raises(ValueError, match="document body"):
        workload.rendered_sizes(tmp_path, "bcs-x")


def raw_request(seq: int, op: str, kind: str = "mutation") -> dict:
    return {
        "seq": seq,
        "start": 10.0 + seq,
        "end": 10.5 + seq,
        "ms": 500.0,
        "method": "POST",
        "host": "api.github.com",
        "path": "/graphql",
        "kind": kind,
        "op": op,
        "req_bytes": 100,
        "resp_bytes": 50,
        "status": 200,
        "cost": None,
        "errors": [],
        "limits": {},
    }


def test_consumption_counts_every_request_and_creation_from_the_committed_logs(
    tmp_path,
):
    # One run as the runner leaves it: the gate's reading (its last is quota-before), the
    # body-limit probe's requests, and the reading after the run that the gate never saw.
    logs = tmp_path / "logs"
    (logs / "superseded").mkdir(parents=True)
    own = {"limit": 5000, "used": 1, "remaining": 4999, "reset": 9e9}
    three = reading(4999, 1, 9e9) | {"graphql (rateLimit)": own | {"readings": [1] * 3}}
    entry = quota.gate(
        "t", logs / "quota-readings.jsonl", "s", ("graphql",), lambda _: three
    )
    run = tmp_path / "raw" / "probe-body-limit-stock-1"
    run.mkdir(parents=True)
    ops = [("ProbeRepo", "query"), ("CreateWide77KB", "mutation")]
    ops += [("CreateAscii66000", "mutation"), ("ProbeDelete", "mutation")]
    (run / "proxy.jsonl").write_text(
        "".join(
            json.dumps(raw_request(n, op, kind)) + "\n"
            for n, (op, kind) in enumerate(ops, 1)
        )
    )
    (run / "quota-before.json").write_text(json.dumps(entry))
    (run / "quota-after.json").write_text(json.dumps({"at": 1.0, "buckets": three}))
    sanitize.write(run, logs / run.name)
    sanitize.write(run, logs / "superseded" / "probe-body-limit-stock-0")

    figures = consumption.run_figures(logs / run.name, {})
    assert (figures["requests"], figures["mutations"], figures["points"]) == (4, 3, 4)
    assert figures["content_creating"] == 2  # both createIssue attempts, refused or not
    dirs = consumption.run_dirs(logs)
    assert [d.name for d in dirs] == [run.name, "probe-body-limit-stock-0"]
    # One gate reading and two after-run readings: three `GET /rate_limit`, and each
    # reading's three `rateLimit` queries counted one by one.
    assert consumption.quota_figures(logs, dirs) == {
        "gate readings": 1,
        "after-run readings": 2,
        "GET /rate_limit": 3,
        "GraphQL rateLimit": 9,
    }
    # A reading taken before the gate listed its reads sent one `rateLimit` query.
    assert consumption.rate_limit_queries({"graphql (rateLimit)": own}) == 1
    assert consumption.rate_limit_queries(reading(1, 1, 1)) == 0


def test_the_committed_consumption_is_what_the_committed_logs_add_up_to():
    logs = HARNESS.parent / "logs"
    committed = json.loads((logs / "consumption.json").read_text())
    prices = json.loads((logs / "pricing.json").read_text())
    dirs = consumption.run_dirs(logs)
    assert {d.name: consumption.run_figures(d, prices) for d in dirs} == committed[
        "runs"
    ]
    assert consumption.quota_figures(logs, dirs) == committed["quota_readings"]
