#!/usr/bin/env python3
"""One measurement of the open-item list of a configured GitHub Projects source.

    python3 measure_open_list.py --config-dir DIR --source NAME --mode MODE --out FILE \
        [--binary target/debug/onetaskgraph] [--env-file PATH] [--status S]... \
        [--stop-after-points 120] [--cap-seconds 600] [--captured FILE] [--item-query Q] \
        [--probe Q]... [--probe-only]

The source is the one ``onetaskgraph.yaml`` in ``--config-dir`` configures, read through the
binary's own ``config show``. Its owner, board number, token variable, status mapping and
``endpoint`` all come from there, and requests go to that endpoint: GitHub for a real board,
the loopback board for a test. The host's user layer is not read, because HOME and every
XDG directory are sandboxed in a temporary tree.

MODE is one of:

``today``
    Runs the binary's ``task list --all`` of every open status: each category the binary's
    ``schema`` names other than ``done`` and ``cancelled``, unless ``--status`` overrides.
    It runs through a local recording proxy set as the source's ``endpoint``. The proxy
    forwards, meters each document with GitHub's own ``rateLimit{cost}``, records each
    request's phase, items and time, and **refuses every mutation**. The binary's standard
    output is kept beside ``--out`` as ``<out>.stdout.json`` and parsed into the counts.

``capture``
    The same run against a proxy that forwards nothing. It keeps the first board-listing
    document the binary sends and answers it with an error, costing nothing.

``prototype``
    Pages ``ProjectV2.items(query: "is:open")`` (``--item-query`` overrides) with the
    document ``capture`` kept, then counts what came back by content and by ``Status`` option:
    drafts, open and closed issues, open issues at an option the configuration sends
    ``done`` or ``cancelled`` to, and options the configuration does not name. Each
    ``--probe Q`` also asks ``totalCount`` under the filter Q, at one point each, and
    ``--probe-only`` sends the probes alone.

Every mode sends nothing more once its spend reaches ``--stop-after-points`` (the request in
flight may take it past by its own cost) and once any reading says GraphQL remaining is under
3,000. ``--cap-seconds`` bounds the whole measurement.

Exit status: ``0`` when the measurement completed; ``1`` when it ran but is incomplete (the
binary failed or timed out, its output did not parse, a limit, the deadline or an API error
stopped it, or a probe was refused) or when an answer it reads (the binary's ``config show``
or ``schema``, or the board's) did not have the shape it reads; ``2`` when the invocation, a
file it names or the token is wrong. One JSON document
is printed on stdout in every case where a measurement ran; diagnostics go to stderr.
"""

from __future__ import annotations

import argparse
import enum
import http.server
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path

from ghlive import (
    GRAPHQL_ENDPOINT,
    BoundaryError,
    RequestRecord,
    SpikeError,
    SpikeLimits,
    SpikeStop,
    Token,
    check_out,
    decode,
    expect_bool,
    expect_int,
    expect_list,
    expect_object,
    expect_str,
    graphql,
    inject_rate,
    load_token,
    meter,
    positive,
    rate_headers,
    rest_base,
    send,
    write_out,
    writes,
)

SOURCE_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]*$")
ITEMS_MARKER = "items(first:$first,after:$after)"


class InvocationError(SpikeError):
    """The invocation, or an input it names, is wrong: exit 2."""


class Mode(enum.StrEnum):
    TODAY = "today"
    CAPTURE = "capture"
    PROTOTYPE = "prototype"


@dataclass(frozen=True)
class Categories:
    """The status categories the binary's own schema names, split by whether they close."""

    open: list[str]
    closing: list[str]


@dataclass(frozen=True)
class CapturedRequest:
    """The board-listing request the binary sent, as ``capture`` keeps it."""

    query: str
    variables: dict[str, object]


@dataclass(frozen=True)
class SourceSettings:
    """What the binary's own configuration says about the measured source."""

    name: str
    owner: str
    number: int
    token_env: str
    endpoint: str
    # The option each category names for a task, where the configuration names one.
    task_options: dict[str, str]


@dataclass
class PhaseSummary:
    requests: int = 0
    points: int = 0
    items: int = 0
    seconds: float = 0.0
    refused: int = 0


@dataclass
class Summary:
    wall_seconds: float
    requests: int
    points: int
    # Requests whose answer reported no cost: each is charged one point against the cap.
    unmetered_requests: int
    phases: dict[str, PhaseSummary]


@dataclass
class BinaryRun:
    mode: Mode
    source: str
    board: str
    command: str
    exit: int
    timed_out: bool
    stderr_tail: str
    summary: Summary
    stdout_path: str | None = None
    stdout_bytes: int = 0
    listed: int | None = None
    listed_by_status: dict[str, int] = field(default_factory=dict)
    listed_ids: list[str] = field(default_factory=list)
    parse_error: str | None = None
    captured: CapturedRequest | None = None
    records: list[RequestRecord] = field(default_factory=list)

    def complete(self) -> bool:
        if self.mode == Mode.CAPTURE:
            return self.captured is not None
        return self.exit == 0 and not self.timed_out and self.listed is not None


@dataclass
class ContentCounts:
    items: int = 0
    draft: int = 0
    issue_open: int = 0
    issue_closed: int = 0
    pull_request: int = 0
    other_content: int = 0
    no_status: int = 0
    unnamed_by_configuration: int = 0
    open_issue_at_closing_option: int | None = 0
    nested_follow_ups: int = 0


@dataclass
class Examples:
    """Up to ten instances of each case a reader would want to look at."""

    unnamed_by_configuration: list[str] = field(default_factory=list)
    open_at_closing: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class ProbeResult:
    """One ``totalCount`` probe: the count, or why there is none."""

    total_count: int | None
    refused: str | None


@dataclass
class PrototypeRun:
    mode: Mode
    source: str
    board: str
    item_query: str
    stopped: str | None
    summary: Summary
    counts: ContentCounts
    by_option: dict[str, int]
    closing_options: list[str] | None
    examples: Examples
    probes: dict[str, ProbeResult]
    issue_ids: list[str]
    records: list[RequestRecord]

    def complete(self) -> bool:
        return self.stopped is None


def phase_of(document: str) -> str:
    if writes(document):
        return "mutation"
    if "fragment Board on ProjectV2" in document:
        return "board-items"
    if "search(" in document:
        return "issue-search"
    return "other: " + " ".join(document.split())[:60]


def items_of(phase: str, answer: dict[str, object]) -> int | None:
    """How many items one answer carried: board nodes, or search nodes."""
    data = answer.get("data")
    if not isinstance(data, dict):
        return None
    match phase:
        case "board-items":
            match data:
                case {"owner": {"projectV2": {"items": {"nodes": list() as nodes}}}}:
                    return len(nodes)
                case _:
                    return None
        case "issue-search":
            for value in data.values():
                match value:
                    case {"nodes": list() as nodes}:
                        return len(nodes)
                    case _:
                        pass
            return None
        case _:
            return None


class Proxy(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(
        self, upstream: str, token: Token, limits: SpikeLimits, capture: bool
    ) -> None:
        super().__init__(("127.0.0.1", 0), Handler)
        self.upstream = upstream
        self.token = token
        self.limits = limits
        self.capture = capture
        self.records: list[RequestRecord] = []
        self.captured: CapturedRequest | None = None
        self.lock = threading.Lock()

    @property
    def endpoint(self) -> str:
        return f"http://127.0.0.1:{self.server_address[1]}/graphql"


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server: Proxy

    def log_message(self, format: str, *args: object) -> None:
        """Every request is recorded instead; the default access log would only interleave
        with the measurement's own stderr."""

    def reply(
        self, status: int, body: bytes, headers: dict[str, str] | None = None
    ) -> None:
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def refuse(self, phase: str, reason: str) -> None:
        with self.server.lock:
            self.server.records.append(
                RequestRecord(phase=phase, at=time.time(), refused=reason)
            )
        self.reply(
            200, json.dumps({"errors": [{"message": f"spike: {reason}"}]}).encode()
        )

    def do_GET(self) -> None:
        if self.server.capture:
            self.refuse(
                f"rest GET {self.path.split('?')[0]}", "capture: nothing is forwarded"
            )
            return
        began = time.time()
        response = send(
            "GET", rest_base(self.server.upstream) + self.path, self.server.token, None
        )
        with self.server.lock:
            self.server.records.append(
                RequestRecord(
                    phase=f"rest GET {self.path.split('?')[0]}",
                    at=began,
                    status=response.status,
                    seconds=round(response.seconds, 3),
                    headers=rate_headers(response.headers),
                )
            )
        self.reply(response.status, response.body, rate_headers(response.headers))

    def do_POST(self) -> None:
        length_header = self.headers.get("Content-Length", "")
        if not length_header.isdigit():
            self.refuse(
                "malformed", f"Content-Length is not a number: {length_header!r}"
            )
            return
        try:
            body = decode(self.rfile.read(int(length_header)), "the binary's request")
            document = expect_str(body.get("query"), "the request's query")
            raw = body.get("variables")
            variables = (
                {} if raw is None else expect_object(raw, "the request's variables")
            )
        except BoundaryError as error:
            self.refuse("malformed", str(error))
            return
        phase = phase_of(document)
        if self.server.capture:
            with self.server.lock:
                if self.server.captured is None and phase == "board-items":
                    self.server.captured = CapturedRequest(document, variables)
            self.refuse(phase, "capture: nothing is forwarded")
            return
        if phase == "mutation":
            self.refuse(phase, "mutations are refused, this measurement is read-only")
            return
        began = time.time()
        with self.server.lock:
            try:
                self.server.limits.admit()
            except SpikeStop as stop:
                self.server.records.append(
                    RequestRecord(phase=phase, at=began, refused=str(stop))
                )
                self.reply(
                    200,
                    json.dumps({"errors": [{"message": f"spike: {stop}"}]}).encode(),
                )
                return
        forwarded = json.dumps(
            {"query": inject_rate(document), "variables": variables}
        ).encode()
        response = send("POST", self.server.upstream, self.server.token, forwarded)
        try:
            answer = decode(response.body, "the upstream answer")
            metering = meter(answer, response.headers)
        except BoundaryError as error:
            # Passed on as it came, so the binary reports the upstream's own failure.
            with self.server.lock:
                self.server.records.append(
                    RequestRecord(
                        phase=phase, at=began, status=response.status, errors=str(error)
                    )
                )
            self.reply(response.status, response.body, rate_headers(response.headers))
            return
        record = RequestRecord(
            phase=phase,
            at=began,
            status=response.status,
            seconds=round(response.seconds, 3),
            cost=metering.cost,
            remaining=metering.remaining,
            items=items_of(phase, answer),
            variables={k: v for k, v in variables.items() if k != "query"},
            errors=answer.get("errors"),
            headers=rate_headers(response.headers),
        )
        with self.server.lock:
            self.server.limits.charge(metering)
            self.server.records.append(record)
        self.reply(
            response.status, json.dumps(answer).encode(), rate_headers(response.headers)
        )


def summarise(records: list[RequestRecord], seconds: float) -> Summary:
    phases: dict[str, PhaseSummary] = {}
    for record in records:
        phase = phases.setdefault(record.phase, PhaseSummary())
        if record.refused is not None:
            phase.refused += 1
            continue
        phase.requests += 1
        phase.points += record.cost or 0
        phase.items += record.items or 0
        phase.seconds = round(phase.seconds + (record.seconds or 0), 3)
    sent = [r for r in records if r.refused is None]
    return Summary(
        wall_seconds=round(seconds, 3),
        requests=len(sent),
        points=sum(r.cost or 0 for r in sent),
        unmetered_requests=sum(1 for r in sent if r.cost is None),
        phases=phases,
    )


def sandbox_env(root: Path) -> dict[str, str]:
    """A process environment holding PATH and a temporary HOME and XDG tree alone."""
    env = {"PATH": os.environ.get("PATH", ""), "HOME": str(root)}
    for name in ("CONFIG", "CACHE", "DATA", "STATE"):
        directory = root / f"xdg-{name.lower()}"
        directory.mkdir(exist_ok=True)
        env[f"XDG_{name}_HOME"] = str(directory)
    return env


@dataclass(frozen=True)
class Deadline:
    """``--cap-seconds`` from the moment the command starts, over every step it takes."""

    at: float

    def remaining(self) -> float:
        left = self.at - time.monotonic()
        if left <= 0:
            raise SpikeStop("the --cap-seconds deadline was reached")
        return left

    def request_timeout(self) -> float:
        return min(120.0, self.remaining())


def run_json(
    command: list[str], cwd: Path, env: dict[str, str], what: str, deadline: Deadline
) -> object:
    try:
        done = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            capture_output=True,
            timeout=deadline.remaining(),
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise SpikeStop(
            f"{what} did not finish before the --cap-seconds deadline"
        ) from error
    if done.returncode != 0:
        stderr = done.stderr.decode("utf-8", "replace")[-600:]
        # The binary's own exit 2 is an invocation it refused (a malformed configuration
        # among them); any other failure is a run that did not complete.
        failure = InvocationError if done.returncode == 2 else BoundaryError
        raise failure(f"{what} exited {done.returncode}: {stderr}")
    try:
        return json.loads(done.stdout)
    except json.JSONDecodeError as error:
        raise BoundaryError(f"{what} did not print JSON: {error}") from error


def schema_consts(roots: dict[str, object], name: str) -> list[str]:
    schema = expect_object(roots.get(name), f"the {name} schema")
    return [
        expect_str(
            expect_object(variant, f"a {name} variant").get("const"), "its const"
        )
        for variant in expect_list(schema.get("oneOf"), f"{name}.oneOf")
    ]


def status_categories(binary: Path, root: Path, deadline: Deadline) -> Categories:
    """Every status category the binary's schema names, split by its ``ClosedAs``: the
    categories a closed issue reads as."""
    schema = expect_object(
        run_json(
            [str(binary), "schema"],
            root,
            sandbox_env(root),
            "`onetaskgraph schema`",
            deadline,
        ),
        "the schema bundle",
    )
    roots = expect_object(schema.get("roots"), "the schema's roots")
    names = schema_consts(roots, "StatusCategory")
    closing = schema_consts(roots, "ClosedAs")
    if not closing or not set(closing) <= set(names):
        raise BoundaryError(
            f"ClosedAs {closing} is not a subset of the categories {names}"
        )
    return Categories(
        open=[name for name in names if name not in closing], closing=closing
    )


def source_settings(
    binary: Path, config_dir: Path, name: str, root: Path, deadline: Deadline
) -> SourceSettings:
    """The measured source, as the binary's ``config show`` resolves it in ``config_dir``."""
    shown = expect_object(
        run_json(
            [str(binary), "--output", "json", "config", "show"],
            config_dir,
            sandbox_env(root),
            "`onetaskgraph config show`",
            deadline,
        ),
        "config show",
    )
    prefix = f"sources.{name}."
    values: dict[str, object] = {}
    for setting in expect_list(shown.get("settings"), "config show's settings"):
        entry = expect_object(setting, "a setting")
        key = expect_str(entry.get("key"), "a setting's key")
        if key.startswith(prefix):
            values[key.removeprefix(prefix)] = entry.get("value")
    if values.get("plugin") != "github-projects":
        raise InvocationError(
            f"source {name} in {config_dir} is not a github-projects source "
            f"(plugin {values.get('plugin')!r}); name one that is"
        )
    task_options: dict[str, str] = {}
    mapping_prefix = "config.status_mapping."
    for key, value in values.items():
        if not key.startswith(mapping_prefix) or value is None:
            continue
        category, _, kind = key.removeprefix(mapping_prefix).partition(".")
        if kind in ("", "task"):
            task_options[category] = expect_str(value, f"status_mapping.{category}")
    endpoint = values.get("config.endpoint")
    return SourceSettings(
        name=name,
        owner=expect_str(values.get("config.owner"), f"sources.{name}.config.owner"),
        number=expect_int(
            values.get("config.project_number"), f"sources.{name}.config.project_number"
        ),
        token_env=expect_str(
            values.get("config.token_env"), f"sources.{name}.config.token_env"
        ),
        endpoint=GRAPHQL_ENDPOINT
        if endpoint is None
        else expect_str(endpoint, "endpoint"),
        task_options=task_options,
    )


def parse_listing(stdout: bytes, run: BinaryRun) -> None:
    """Fill ``run``'s counts from ``task list --output json``, whose rows are
    ``{"id": …, "item": {"status": {"category", "name"}, …}}``."""
    try:
        listing = expect_object(json.loads(stdout), "the listing")
        rows = expect_list(listing.get("items"), "the listing's items")
        by_status: dict[str, int] = {}
        ids: list[str] = []
        for row in rows:
            entry = expect_object(row, "a listed row")
            item = expect_object(entry.get("item"), "a row's item")
            status = expect_object(item.get("status"), "an item's status")
            key = (
                f"{expect_str(status.get('category'), 'a category')}:"
                f"{expect_str(status.get('name'), 'a status name')}"
            )
            by_status[key] = by_status.get(key, 0) + 1
            ids.append(expect_str(entry.get("id"), "a row's id"))
    except (json.JSONDecodeError, BoundaryError) as error:
        run.parse_error = f"{error}: {stdout[:300]!r}"
        return
    run.listed = len(rows)
    run.listed_by_status = by_status
    run.listed_ids = sorted(ids)


def run_binary(
    args: argparse.Namespace,
    source: SourceSettings,
    categories: Categories,
    token: Token,
    root: Path,
) -> BinaryRun:
    capture = args.mode == Mode.CAPTURE
    proxy = Proxy(
        source.endpoint,
        token,
        SpikeLimits(stop_after_points=args.stop_after_points),
        capture,
    )
    thread = threading.Thread(target=proxy.serve_forever, daemon=True)
    thread.start()
    statuses = args.status or categories.open
    command = [
        str(args.binary),
        "--output",
        "json",
        "--set",
        f"sources.{source.name}.config.endpoint={proxy.endpoint}",
        "task",
        "list",
        "--all",
        "--source",
        source.name,
    ]
    for status in statuses:
        command += ["--status", status]
    env = {**sandbox_env(root), source.token_env: token}
    timeout = args.deadline.remaining()
    began = time.monotonic()
    process = subprocess.Popen(
        command,
        cwd=args.config_dir,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    timed_out = False
    try:
        out, err = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        process.kill()
        out, err = process.communicate()
    seconds = time.monotonic() - began
    proxy.shutdown()
    run = BinaryRun(
        mode=args.mode,
        source=source.name,
        board=f"{source.owner}/{source.number}",
        command=" ".join(command[1:]),
        exit=process.returncode,
        timed_out=timed_out,
        stderr_tail=err.decode("utf-8", "replace")[-1500:],
        summary=summarise(proxy.records, seconds),
        stdout_bytes=len(out),
    )
    if capture:
        run.captured = proxy.captured
        return run
    stdout_path = args.out.with_suffix(".stdout.json")
    write_out(stdout_path, out)
    run.stdout_path = str(stdout_path)
    parse_listing(out, run)
    run.records = proxy.records
    return run


def status_option(node: dict[str, object]) -> str | None:
    """The ``Status`` option a board item node carries in its own field values."""
    values = expect_object(node.get("fieldValues"), "an item's fieldValues")
    for raw in expect_list(values.get("nodes"), "fieldValues.nodes"):
        value = expect_object(raw, "a field value")
        match value:
            case {"field": {"name": "Status"}}:
                return expect_str(value.get("name"), "the Status option's name")
            case _:
                pass
    return None


def nested_more(value: object) -> bool:
    """Whether any connection nested in ``value`` reports another page."""
    match value:
        case {"pageInfo": info}:
            more = expect_object(info, "a nested pageInfo").get("hasNextPage")
            return expect_bool(more, "a nested hasNextPage") or any(
                nested_more(child) for key, child in value.items() if key != "pageInfo"
            )
        case dict():
            return any(nested_more(child) for child in value.values())
        case list():
            return any(nested_more(child) for child in value)
        case _:
            return False


def page_size(value: object, what: str) -> int:
    size = expect_int(value, what)
    if not 1 <= size <= 100:
        raise BoundaryError(f"{what} is {size}, outside GitHub's 1 to 100")
    return size


def captured_variables(raw: object) -> dict[str, object]:
    """The captured request's bindings this harness forwards, each checked; the board's owner
    and number and the cursor are set from the configured source and the walk instead."""
    variables = expect_object(raw, "the captured variables")
    known = {"owner", "number", "first", "after", "nestedFirst", "duplicates"}
    unknown = sorted(set(variables) - known)
    if unknown:
        raise BoundaryError(
            f"the capture binds variables this harness does not know: {unknown}"
        )
    return {
        "first": page_size(variables.get("first"), "the captured first"),
        "nestedFirst": page_size(
            variables.get("nestedFirst"), "the captured nestedFirst"
        ),
        "duplicates": expect_bool(
            variables.get("duplicates"), "the captured duplicates"
        ),
    }


def captured_document(path: Path) -> tuple[str, dict[str, object]]:
    try:
        stored = expect_object(json.loads(path.read_text(encoding="utf-8")), str(path))
        request = expect_object(stored.get("captured"), "the captured request")
        document = expect_str(request.get("query"), "the captured query")
        variables = captured_variables(request.get("variables"))
    except (OSError, json.JSONDecodeError, BoundaryError) as error:
        raise InvocationError(
            f"cannot read the capture {path}: {error}; run --mode capture again"
        ) from error
    if ITEMS_MARKER not in document or "$after:String," not in document:
        raise InvocationError(
            f"the captured document has no `{ITEMS_MARKER}`; run --mode capture again"
        )
    document = document.replace(
        ITEMS_MARKER, "items(first:$first,after:$after,query:$itemQuery)"
    )
    document = document.replace("$after:String,", "$after:String,$itemQuery:String,", 1)
    return document, variables


def board_items(answer: dict[str, object]) -> dict[str, object]:
    data = expect_object(answer.get("data"), "the answer's data")
    owner = expect_object(data.get("owner"), "the board's owner")
    project = expect_object(
        owner.get("projectV2"), "the board (null: not found or not visible)"
    )
    return expect_object(project.get("items"), "the board's items")


def prototype(
    args: argparse.Namespace,
    source: SourceSettings,
    categories: Categories,
    token: Token,
) -> PrototypeRun:
    document, variables = captured_document(args.captured)
    variables["itemQuery"] = args.item_query
    # The configured source is what is measured, whichever board the capture came from.
    variables["owner"] = source.owner
    variables["number"] = source.number
    limits = SpikeLimits(stop_after_points=args.stop_after_points)
    records: list[RequestRecord] = []
    named = set(source.task_options.values())
    closing: set[str] | None = {
        source.task_options[c] for c in categories.closing if c in source.task_options
    }
    if closing is not None and len(closing) < len(categories.closing):
        # The rest are the plugin's shipped defaults, which this harness does not restate.
        closing = None
    counts = ContentCounts(open_issue_at_closing_option=None if closing is None else 0)
    by_option: dict[str, int] = {}
    ids: list[str] = []
    examples = Examples()
    deadline: Deadline = args.deadline
    began = time.monotonic()
    after: str | None = None
    stopped: str | None = None
    try:
        while not args.probe_only:
            variables["after"] = after
            answer, record = graphql(
                source.endpoint,
                document,
                variables,
                token,
                limits,
                records,
                "open-items",
                timeout=deadline.request_timeout(),
            )
            if answer.get("errors"):
                raise SpikeStop(
                    f"the board answered errors: {json.dumps(answer['errors'])[:600]}"
                )
            items = board_items(answer)
            nodes = expect_list(items.get("nodes"), "the items' nodes")
            record.items = len(nodes)
            for raw in nodes:
                node = expect_object(raw, "a board item")
                counts.items += 1
                # GitHub answers `content: null` for an item the token cannot see.
                raw_content = node.get("content")
                content = (
                    {}
                    if raw_content is None
                    else expect_object(raw_content, "an item's content")
                )
                option = status_option(node)
                by_option[option or "<none>"] = by_option.get(option or "<none>", 0) + 1
                match content:
                    case {"__typename": "DraftIssue"}:
                        counts.draft += 1
                    case {"__typename": "Issue", "state": "CLOSED"}:
                        counts.issue_closed += 1
                    case {"__typename": "Issue", "state": "OPEN"}:
                        counts.issue_open += 1
                        ids.append(expect_str(content.get("id"), "an issue's id"))
                        if closing is not None and option in closing:
                            counts.open_issue_at_closing_option = (
                                counts.open_issue_at_closing_option or 0
                            ) + 1
                            if len(examples.open_at_closing) < 10:
                                examples.open_at_closing.append(
                                    f"#{expect_int(content.get('number'), 'an issue number')} "
                                    f"{option}"
                                )
                    case {"__typename": "Issue"}:
                        raise BoundaryError(
                            f"an issue's state is {content.get('state')!r}, not OPEN or CLOSED"
                        )
                    case {"__typename": "PullRequest"}:
                        counts.pull_request += 1
                    case _:
                        counts.other_content += 1
                if option is None:
                    counts.no_status += 1
                elif option not in named:
                    counts.unnamed_by_configuration += 1
                    if len(examples.unnamed_by_configuration) < 10:
                        examples.unnamed_by_configuration.append(option)
                if nested_more(node):
                    counts.nested_follow_ups += 1
            info = expect_object(items.get("pageInfo"), "the items' pageInfo")
            if not expect_bool(info.get("hasNextPage"), "hasNextPage"):
                break
            cursor = expect_str(info.get("endCursor"), "endCursor")
            if cursor == after:
                raise BoundaryError(f"the board repeated its cursor {cursor!r}")
            after = cursor
    except (SpikeStop, BoundaryError) as stop:
        stopped = str(stop)
    probes: dict[str, ProbeResult] = {}
    count_doc = (
        "query($owner:String!,$number:Int!,$q:String){owner:repositoryOwner(login:$owner)"
        "{... on ProjectV2Owner{projectV2(number:$number){items(first:1,query:$q)"
        "{totalCount}}}}}"
    )
    for probe in args.probe:
        try:
            answer, _record = graphql(
                source.endpoint,
                count_doc,
                {"owner": source.owner, "number": source.number, "q": probe},
                token,
                limits,
                records,
                "probe",
                timeout=deadline.request_timeout(),
            )
            errors = answer.get("errors")
            if errors:
                raise SpikeStop(f"the probe was refused: {json.dumps(errors)[:300]}")
            count = expect_int(board_items(answer).get("totalCount"), "totalCount")
            probes[probe] = ProbeResult(total_count=count, refused=None)
        except (SpikeStop, BoundaryError) as stop:
            probes[probe] = ProbeResult(total_count=None, refused=str(stop))
            stopped = stopped or str(stop)
    seconds = time.monotonic() - began
    return PrototypeRun(
        mode=Mode.PROTOTYPE,
        source=source.name,
        board=f"{source.owner}/{source.number}",
        item_query=args.item_query,
        stopped=stopped,
        summary=summarise(records, seconds),
        counts=counts,
        by_option=by_option,
        closing_options=None if closing is None else sorted(closing),
        examples=examples,
        probes=probes,
        issue_ids=sorted(ids),
        records=records,
    )


def parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--config-dir", type=Path, required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--mode", type=Mode, choices=list(Mode), required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument(
        "--binary", type=Path, default=Path("target/debug/onetaskgraph")
    )
    parser.add_argument("--env-file")
    parser.add_argument(
        "--status", action="append", help="a category to list; repeatable"
    )
    parser.add_argument("--stop-after-points", type=positive, default=120)
    parser.add_argument("--cap-seconds", type=positive, default=600)
    parser.add_argument("--captured", type=Path, help="the JSON `capture` wrote")
    parser.add_argument("--item-query", default="is:open")
    parser.add_argument(
        "--probe",
        action="append",
        default=[],
        help="a totalCount filter to ask; repeatable",
    )
    parser.add_argument(
        "--probe-only", action="store_true", help="send the probes alone"
    )
    return parser


def validate(args: argparse.Namespace) -> None:
    if not SOURCE_NAME.match(args.source):
        raise InvocationError(f"--source {args.source!r} is not a source name")
    if not (args.config_dir / "onetaskgraph.yaml").is_file():
        raise InvocationError(f"{args.config_dir} holds no onetaskgraph.yaml")
    if not args.binary.is_file():
        raise InvocationError(f"no binary at {args.binary}; build it with `just` first")
    args.binary = args.binary.resolve()
    args.config_dir = args.config_dir.resolve()
    check_out(args.out)
    check_out(args.out.with_suffix(".stdout.json"))
    if args.mode == Mode.PROTOTYPE and args.captured is None:
        raise InvocationError(
            "--mode prototype needs --captured FILE from --mode capture"
        )
    if args.mode != Mode.PROTOTYPE and (args.probe or args.probe_only):
        raise InvocationError(
            "--probe and --probe-only apply to --mode prototype alone"
        )
    if args.probe_only and not args.probe:
        raise InvocationError("--probe-only needs at least one --probe")


def main() -> int:
    args = parser().parse_args()
    args.deadline = Deadline(time.monotonic() + args.cap_seconds)
    try:
        validate(args)
        token = load_token(args.env_file)
        with tempfile.TemporaryDirectory(prefix="spike-open-list-") as work:
            root = Path(work)
            source = source_settings(
                args.binary, args.config_dir, args.source, root, args.deadline
            )
            categories = status_categories(args.binary, root, args.deadline)
            result: BinaryRun | PrototypeRun = (
                prototype(args, source, categories, token)
                if args.mode == Mode.PROTOTYPE
                else run_binary(args, source, categories, token, root)
            )
    except (BoundaryError, SpikeStop) as error:
        print(f"measure_open_list: {error}", file=sys.stderr)
        return 1
    except SpikeError as error:
        print(f"measure_open_list: {error}", file=sys.stderr)
        return 2
    document = asdict(result)
    try:
        write_out(args.out, (json.dumps(document, indent=2) + "\n").encode())
    except SpikeError as error:
        print(f"measure_open_list: {error}", file=sys.stderr)
        return 2
    brief = {
        k: v
        for k, v in document.items()
        if k not in {"records", "listed_ids", "issue_ids"}
    }
    print(json.dumps(brief, indent=2))
    if not result.complete():
        print(
            "measure_open_list: the measurement is incomplete; see stopped, exit, timed_out "
            f"or parse_error in {args.out}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
