#!/usr/bin/env python3
"""One measurement of the open-item list against a named GitHub board.

    python3 spikes/store-defects/measure_open_list.py --board OWNER/NUMBER --mode MODE \
        [--binary target/debug/onetaskgraph] [--env-file PATH] [--max-points N] \
        [--cap-seconds 600] [--out FILE]

MODE is one of:

``today``
    Runs the real binary's ``task list --all`` of every open status (``draft``, ``backlog``,
    ``todo``, ``queued``, ``in-progress``, ``unknown``; ``--statuses`` overrides) against the
    board, through a local recording proxy configured as the source's GraphQL ``endpoint``. The
    proxy forwards to api.github.com, adds GitHub's own ``rateLimit{cost}`` to every document
    (and strips it from the answer), records each request's phase, items and time, and
    **refuses every mutation**, so the board cannot be written. The run is killed at
    ``--cap-seconds``.

``capture``
    The same run against a proxy that forwards nothing: it records the first board-listing
    document the binary sends and answers it with an error, costing no points. ``prototype``
    reuses that document, so it reads exactly the fields the plugin reads.

``prototype``
    Pages ``ProjectV2.items(query: "is:open")`` (``--item-query`` overrides) with the captured
    document, then reports what the open-only read returned: drafts, items at an option the
    mapping does not name, open issues under a terminal card, closed items, and any nested
    connection that would need a follow-up read. ``--probe`` also asks ``totalCount`` under
    several candidate filters, one point each.

Prints one JSON document of requests, pages, points and seconds per phase; ``--out`` writes it.
"""

from __future__ import annotations

import argparse
import http.server
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ghlive import (  # noqa: E402
    API,
    Recorder,
    SpikeLimits,
    SpikeStop,
    graphql,
    inject_rate,
    load_token,
    rate_headers,
    send,
)

OPEN_STATUSES = ["draft", "backlog", "todo", "queued", "in-progress", "unknown"]
# The `followups` board's own mapping, as ai-orchestrator's onetaskgraph.yaml states it.
FOLLOWUPS_MAPPING = {
    "backlog": "Proposal",
    "draft": "Deferred",
    "queued": "Queued",
    "done": "Done",
    "cancelled": "Cancelled",
}
# The shipped defaults the mapping leaves in place.
DEFAULT_OPTIONS = {"todo": "Todo", "in-progress": "In Progress"}
TERMINAL_OPTIONS = {FOLLOWUPS_MAPPING["done"], FOLLOWUPS_MAPPING["cancelled"]}
PROBES = [
    "",
    "is:open",
    "-is:closed",
    "is:open is:issue",
    "is:draft",
    "is:open -status:Done,Cancelled",
    "no:status",
    "is:open no:status",
]


def phase_of(document: str) -> str:
    if document.lstrip().startswith("mutation"):
        return "mutation"
    if "fragment Board on ProjectV2" in document:
        return "board-items"
    if "search(" in document:
        return "issue-search"
    head = " ".join(document.split())[:60]
    return f"other: {head}"


def items_of(phase: str, answer: dict[str, Any]) -> int | None:
    data = answer.get("data") or {}
    if phase == "board-items":
        nodes = (((data.get("owner") or {}).get("projectV2") or {}).get("items") or {}).get(
            "nodes"
        )
        return len(nodes) if isinstance(nodes, list) else None
    if phase == "issue-search":
        for value in data.values():
            if isinstance(value, dict) and isinstance(value.get("nodes"), list):
                return len(value["nodes"])
    return None


class Proxy(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, token: str, limits: SpikeLimits, capture: bool) -> None:
        super().__init__(("127.0.0.1", 0), Handler)
        self.token = token
        self.limits = limits
        self.capture = capture
        self.records: list[dict[str, Any]] = []
        self.captured: dict[str, Any] | None = None
        self.lock = threading.Lock()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server: Proxy

    def log_message(self, *_args: Any) -> None:  # quiet
        return

    def reply(self, status: int, body: bytes, headers: dict[str, str] | None = None) -> None:
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def refuse(self, message: str) -> None:
        self.reply(200, json.dumps({"errors": [{"message": message}]}).encode())

    def do_GET(self) -> None:
        with self.server.lock:
            self.server.records.append({"phase": f"rest GET {self.path}", "at": time.time()})
        status, headers, payload, seconds = send("GET", API + self.path, self.server.token, None)
        with self.server.lock:
            self.server.records[-1].update(
                {"status": status, "seconds": round(seconds, 3), "headers": rate_headers(headers)}
            )
        self.reply(status, payload, rate_headers(headers))

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        document = body.get("query", "")
        phase = phase_of(document)
        if self.server.capture:
            with self.server.lock:
                if self.server.captured is None and phase == "board-items":
                    self.server.captured = body
                self.server.records.append({"phase": phase, "captured_only": True})
            self.refuse("spike capture: nothing is forwarded")
            return
        if phase == "mutation":
            with self.server.lock:
                self.server.records.append({"phase": phase, "refused": "read-only spike"})
            self.refuse("spike: mutations are refused, this measurement is read-only")
            return
        began = time.time()
        with self.server.lock:
            try:
                self.server.limits.admit()
            except SpikeStop as stop:
                self.server.records.append({"phase": phase, "refused": str(stop), "at": began})
                self.refuse(f"spike: {stop}")
                return
        forwarded = json.dumps(
            {"query": inject_rate(document), "variables": body.get("variables") or {}}
        ).encode()
        status, headers, payload, seconds = send(
            "POST", f"{API}/graphql", self.server.token, forwarded
        )
        try:
            answer = json.loads(payload)
        except json.JSONDecodeError:
            answer = {}
        rate = (answer.get("data") or {}).pop("spikeRate", None) or {}
        record = {
            "phase": phase,
            "at": began,
            "status": status,
            "seconds": round(seconds, 3),
            "cost": rate.get("cost"),
            "remaining": rate.get("remaining"),
            "items": items_of(phase, answer),
            "variables": {
                k: v for k, v in (body.get("variables") or {}).items() if k != "query"
            },
            "errors": answer.get("errors"),
            "headers": rate_headers(headers),
        }
        with self.server.lock:
            if isinstance(rate.get("cost"), int):
                self.server.limits.spent += rate["cost"]
            if isinstance(rate.get("remaining"), int):
                self.server.limits.last_remaining = rate["remaining"]
            self.server.records.append(record)
        self.reply(status, json.dumps(answer).encode(), rate_headers(headers))


def config_for(owner: str, number: int, endpoint: str) -> str:
    mapping = {**FOLLOWUPS_MAPPING}
    return json.dumps(
        {
            "default_sources": ["board"],
            "sources": {
                "board": {
                    "plugin": "github-projects",
                    "config": {
                        "owner": owner,
                        "project_number": number,
                        "repository": "nickderobertis/ai-orchestrator",
                        "token_env": "GH_PROJECTS_TOKEN",
                        "endpoint": endpoint,
                        "status_mapping": mapping,
                    },
                }
            },
        },
        indent=2,
    )


def summarise(records: list[dict[str, Any]], seconds: float) -> dict[str, Any]:
    phases: dict[str, dict[str, Any]] = {}
    for record in records:
        phase = phases.setdefault(
            record["phase"],
            {"requests": 0, "points": 0, "items": 0, "seconds": 0.0, "refused": 0},
        )
        if record.get("refused") or record.get("captured_only"):
            phase["refused"] += 1
            continue
        phase["requests"] += 1
        phase["points"] += record.get("cost") or 0
        phase["items"] += record.get("items") or 0
        phase["seconds"] = round(phase["seconds"] + (record.get("seconds") or 0), 3)
    sent = [r for r in records if not r.get("refused") and not r.get("captured_only")]
    return {
        "wall_seconds": round(seconds, 3),
        "requests": len(sent),
        "points": sum(r.get("cost") or 0 for r in sent),
        "phases": phases,
    }


def run_binary(args: argparse.Namespace, token: str, capture: bool) -> dict[str, Any]:
    owner, _, number = args.board.partition("/")
    limits = SpikeLimits(max_points=args.max_points)
    proxy = Proxy(token, limits, capture)
    thread = threading.Thread(target=proxy.serve_forever, daemon=True)
    thread.start()
    endpoint = f"http://127.0.0.1:{proxy.server_address[1]}/graphql"
    statuses = args.statuses.split(",") if args.statuses else OPEN_STATUSES
    with tempfile.TemporaryDirectory(prefix="spike-open-list-") as work:
        root = Path(work)
        (root / "onetaskgraph.yaml").write_text(config_for(owner, int(number), endpoint))
        for sub in ("xdg-config", "xdg-cache", "xdg-data", "xdg-state"):
            (root / sub).mkdir()
        env = {
            "PATH": os.environ.get("PATH", ""),
            "HOME": str(root),
            "XDG_CONFIG_HOME": str(root / "xdg-config"),
            "XDG_CACHE_HOME": str(root / "xdg-cache"),
            "XDG_DATA_HOME": str(root / "xdg-data"),
            "XDG_STATE_HOME": str(root / "xdg-state"),
            "GH_PROJECTS_TOKEN": token,
        }
        command = [str(Path(args.binary).resolve()), "--output", "json", "task", "list", "--all"]
        command += ["--source", "board"]
        for status in statuses:
            command += ["--status", status]
        began = time.monotonic()
        process = subprocess.Popen(
            command, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        )
        timed_out = False
        try:
            out, err = process.communicate(timeout=args.cap_seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            process.kill()
            out, err = process.communicate()
        seconds = time.monotonic() - began
    proxy.shutdown()
    result: dict[str, Any] = {
        "mode": "capture" if capture else "today",
        "board": args.board,
        "command": " ".join(command[1:]),
        "exit": process.returncode,
        "timed_out": timed_out,
        "stderr_tail": err.decode("utf-8", "replace")[-1500:],
        **summarise(proxy.records, seconds),
    }
    if capture:
        result["captured"] = proxy.captured
    else:
        result["stdout_bytes"] = len(out)
        if args.out:
            Path(args.out).with_suffix(".stdout.json").write_bytes(out)
        try:
            listed = json.loads(out)
            items = listed.get("items", [])
            result["listed"] = len(items)
            result["listed_by_status"] = {}
            for entry in items:
                item = entry.get("item", entry)
                status = item.get("status") or {}
                key = f"{status.get('category')}:{status.get('name')}"
                result["listed_by_status"][key] = result["listed_by_status"].get(key, 0) + 1
            result["listed_ids"] = sorted(str(e.get("item", e).get("id")) for e in items)
        except (json.JSONDecodeError, AttributeError) as error:
            result["listed"] = None
            result["parse_error"] = f"{error}: {out[:300]!r}"
        result["records"] = proxy.records
    return result


def status_option(node: dict[str, Any]) -> str | None:
    """The `Status` single-select name a board item node carries, wherever its values sit."""
    stack: list[Any] = [node]
    while stack:
        value = stack.pop()
        if isinstance(value, dict):
            field = value.get("field")
            if (
                isinstance(field, dict)
                and field.get("name") == "Status"
                and isinstance(value.get("name"), str)
            ):
                return value["name"]
            if value is not node and value.get("__typename") in {"Issue", "DraftIssue"}:
                continue
            stack.extend(value.values())
        elif isinstance(value, list):
            stack.extend(value)
    return None


def nested_more(node: dict[str, Any]) -> list[str]:
    """Paths of nested connections in ``node`` that report another page."""
    found: list[str] = []

    def walk(value: Any, path: str) -> None:
        if isinstance(value, dict):
            info = value.get("pageInfo")
            if isinstance(info, dict) and info.get("hasNextPage"):
                found.append(path)
            for key, child in value.items():
                walk(child, f"{path}.{key}")
        elif isinstance(value, list):
            for child in value:
                walk(child, path)

    walk(node.get("content") or {}, "content")
    for key, value in node.items():
        if key != "content":
            walk(value, key)
    return found


def prototype(args: argparse.Namespace, token: str) -> dict[str, Any]:
    captured = json.loads(Path(args.captured).read_text())
    captured = captured.get("captured", captured)
    document: str = captured["query"]
    marker = "items(first:$first,after:$after)"
    if marker not in document:
        raise SystemExit(f"the captured document has no `{marker}`; capture again")
    document = document.replace(marker, "items(first:$first,after:$after,query:$itemQuery)")
    document = document.replace("$after:String,", "$after:String,$itemQuery:String,", 1)
    variables = dict(captured.get("variables") or {})
    variables["itemQuery"] = args.item_query
    limits = SpikeLimits(max_points=args.max_points)
    recorder = Recorder()
    known = set(FOLLOWUPS_MAPPING.values()) | set(DEFAULT_OPTIONS.values())
    counts = {
        "items": 0,
        "draft": 0,
        "issue_open": 0,
        "issue_closed": 0,
        "pull_request": 0,
        "other_content": 0,
        "no_status": 0,
        "unknown_option": 0,
        "open_issue_terminal_card": 0,
        "nested_follow_ups": 0,
    }
    by_option: dict[str, int] = {}
    ids: list[str] = []
    examples: dict[str, list[str]] = {"unknown_option": [], "open_issue_terminal_card": []}
    began = time.monotonic()
    after = None
    stopped = None
    while not args.probe_only:
        variables["after"] = after
        try:
            answer, record = graphql(document, variables, token, limits, recorder, "open-items")
        except SpikeStop as stop:
            stopped = str(stop)
            break
        if answer.get("errors"):
            stopped = json.dumps(answer["errors"])[:600]
            break
        items = answer["data"]["owner"]["projectV2"]["items"]
        record["items"] = len(items["nodes"])
        for node in items["nodes"]:
            counts["items"] += 1
            content = node.get("content") or {}
            kind = content.get("__typename")
            option = status_option(node)
            by_option[option or "<none>"] = by_option.get(option or "<none>", 0) + 1
            if kind == "DraftIssue":
                counts["draft"] += 1
            elif kind == "Issue":
                closed = content.get("state") == "CLOSED" or content.get("closed") is True
                counts["issue_closed" if closed else "issue_open"] += 1
                if not closed and option in TERMINAL_OPTIONS:
                    counts["open_issue_terminal_card"] += 1
                    if len(examples["open_issue_terminal_card"]) < 10:
                        examples["open_issue_terminal_card"].append(
                            f"#{content.get('number')} {option}"
                        )
                if content.get("id"):
                    ids.append(content["id"])
            elif kind == "PullRequest":
                counts["pull_request"] += 1
            else:
                counts["other_content"] += 1
            if option is None:
                counts["no_status"] += 1
            elif option not in known:
                counts["unknown_option"] += 1
                if len(examples["unknown_option"]) < 10:
                    examples["unknown_option"].append(option)
            if nested_more(node):
                counts["nested_follow_ups"] += 1
        info = items["pageInfo"]
        if not info["hasNextPage"]:
            break
        after = info["endCursor"]
    seconds = time.monotonic() - began
    probes = {}
    if args.probe:
        count_doc = (
            "query($owner:String!,$number:Int!,$q:String){owner:repositoryOwner(login:$owner)"
            "{... on ProjectV2Owner{projectV2(number:$number){items(first:1,query:$q)"
            "{totalCount}}}}}"
        )
        owner, _, number = args.board.partition("/")
        for probe in args.probe_query or PROBES:
            try:
                answer, _record = graphql(
                    count_doc,
                    {"owner": owner, "number": int(number), "q": probe},
                    token,
                    limits,
                    recorder,
                    "probe",
                )
            except SpikeStop as stop:
                probes[probe] = f"not sent: {stop}"
                continue
            probes[probe] = answer.get("errors") or answer["data"]["owner"]["projectV2"][
                "items"
            ]["totalCount"]
    result = {
        "mode": "prototype",
        "board": args.board,
        "item_query": args.item_query,
        "stopped": stopped,
        **summarise(recorder.records, seconds),
        "counts": counts,
        "by_option": by_option,
        "examples": examples,
        "probes": probes,
        "issue_ids": sorted(ids),
        "records": recorder.records,
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--board", required=True, help="OWNER/NUMBER")
    parser.add_argument("--mode", choices=["today", "capture", "prototype"], required=True)
    parser.add_argument("--binary", default="target/debug/onetaskgraph")
    parser.add_argument("--env-file")
    parser.add_argument("--statuses", help="comma-separated; default every open status")
    parser.add_argument("--max-points", type=int, default=120)
    parser.add_argument("--cap-seconds", type=int, default=600)
    parser.add_argument("--captured", help="the JSON `capture` wrote, for `prototype`")
    parser.add_argument("--item-query", default="is:open")
    parser.add_argument("--probe", action="store_true")
    parser.add_argument(
        "--probe-query", action="append", help="a totalCount filter to probe; repeatable"
    )
    parser.add_argument(
        "--probe-only", action="store_true", help="send the probes alone, no item pages"
    )
    parser.add_argument("--out")
    args = parser.parse_args()
    token = load_token(args.env_file)
    if args.mode == "prototype":
        if not args.captured:
            raise SystemExit("--mode prototype needs --captured FILE from --mode capture")
        result = prototype(args, token)
    else:
        result = run_binary(args, token, capture=args.mode == "capture")
        if args.mode == "capture" and result.get("captured") is None:
            raise SystemExit(f"no board-listing document was captured: {result['stderr_tail']}")
    text = json.dumps(result, indent=2)
    if args.out:
        Path(args.out).write_text(text + "\n", encoding="utf-8")
    brief = {k: v for k, v in result.items() if k not in {"records", "listed_ids", "issue_ids"}}
    if args.mode == "capture":
        brief["captured"] = "(written to --out)" if args.out else result["captured"]
    print(json.dumps(brief, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
