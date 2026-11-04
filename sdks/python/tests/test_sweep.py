"""`sources sanitize` through the SDK: its report comes back on exits 1 and 3, not as an error.

A sweep answers its report whenever it ran — exit 1 when something is left unfixable, exit 3
when its search could not cover every class of record — and refuses with a failure document
only when it could not run. Every call drives the real binary, under a `write_policy` of two
small commands: one listing the store's own replacements as the terms, and one rewriting them
and refusing any identifier that carries one, the way onevcs's sanitise command does.
"""

from __future__ import annotations

import asyncio
import json
import os
import re
import sys
from collections.abc import Coroutine
from pathlib import Path
from typing import TypedDict, get_args

import pytest
from pydantic import BaseModel, JsonValue, TypeAdapter
from sweep_source import DependencyDirection, Label, Status, TaskId

from onetaskgraph_sdk import Client, OnetaskgraphError, StatusCategory

TERMS = """
import json, sys
received = json.loads(sys.stdin.read())
print(json.dumps({"terms": [
    {"term": entry["term"], "mode": "whole-word", "case_sensitive": False,
     "action": "replace", "with": entry["with"]}
    for entry in received.get("replacements", []) if "with" in entry]}))
"""

SANITIZE = """
import json, sys
received = json.loads(sys.stdin.read())
terms = [(entry["term"], entry["with"]) for entry in received.get("replacements", [])]
replaced = 0
def rewrite(value):
    global replaced
    for term, with_ in terms:
        replaced += value.count(term)
        value = value.replace(term, with_)
    return value
answer = {surface: [rewrite(value) for value in received.get(surface, [])]
          for surface in ("text", "paths", "metadata")}
locators = received.get("locators", [])
carrying = [index for index, locator in enumerate(locators)
            if any(term in locator["value"] for term, _ in terms)]
answer["replaced"] = replaced
answer["locators"] = []
ordinal = 0
for index, locator in enumerate(locators):
    if index in carrying:
        ordinal += 1
        answer["locators"].append({"redacted": True, "kind": locator["kind"], "ordinal": ordinal})
    else:
        answer["locators"].append(locator["value"])
if carrying:
    answer.update(verdict="refuse", surface="locators",
                  unfixed=[{"surface": "locators", "index": index, "fixed": True}
                           for index in carrying])
    print(json.dumps(answer))
    sys.exit(1)
answer.update(verdict="pass", unfixed=[])
print(json.dumps(answer))
"""

TERM = "emberline"


class PeerError(BaseModel):
    """The protocol peer's refused-request payload."""

    kind: str
    message: str


class PeerReply(BaseModel):
    """A refused response from the real peer subprocess."""

    id: str
    error: PeerError


async def peer_exchange(store: Path, request: dict[str, JsonValue]) -> tuple[int, bytes, bytes]:
    """Exchange a typed JSON request with the peer's real subprocess."""
    process = await asyncio.create_subprocess_exec(
        sys.executable,
        str(Path(__file__).with_name("sweep_source.py")),
        str(store),
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, stderr = await process.communicate((json.dumps(request) + "\n").encode())
    assert process.returncode is not None
    return process.returncode, stdout, stderr


async def peer_request(store: Path, request: dict[str, JsonValue]) -> PeerReply:
    """Send a malformed wire request and validate the peer's response."""
    code, stdout, stderr = await peer_exchange(store, request)
    assert code == 0, stderr.decode()
    return PeerReply.model_validate_json(stdout)


@pytest.mark.parametrize(
    "held",
    [
        {"requests": True, "tasks": []},
        {"requests": -1, "tasks": []},
        {"requests": 0, "tasks": [{"id": "t1", "title": "No content"}]},
    ],
)
def test_the_search_peer_refuses_malformed_stored_state(
    tmp_path: Path, held: dict[str, JsonValue]
) -> None:
    """A malformed store is refused before any task can be read from it."""
    store = tmp_path / "tracker.json"
    before = json.dumps(held)
    store.write_text(before, encoding="utf-8")
    code, _, stderr = run(peer_exchange(store, {"id": "health", "method": "health"}))
    assert code != 0
    assert "not this peer's state" in stderr.decode()
    assert "Traceback" not in stderr.decode()
    assert store.read_text(encoding="utf-8") == before


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("status", {"category": "invented", "name": "Todo"}),
        ("labels", [7]),
        ("project", 7),
        ("location", {"url": "https://example.invalid", "path": "/local"}),
        ("created_at", 7),
        ("updated_at", 7),
    ],
)
def test_the_search_peer_validates_every_stored_task_field(
    tmp_path: Path,
    field: str,
    value: JsonValue,
) -> None:
    """Do not return malformed persisted fields as if they were a task from the protocol."""
    item: dict[str, JsonValue] = {
        "id": "t1",
        "title": "Plain",
        "content": "Plain.",
        "status": {"category": "todo", "name": "Todo"},
        "labels": [],
        "project": None,
        "url": None,
        "location": None,
        "created_at": None,
        "updated_at": None,
    }
    item[field] = value
    store = tmp_path / "tracker.json"
    before = json.dumps({"requests": 0, "tasks": [item]})
    store.write_text(before, encoding="utf-8")
    code, _, stderr = run(peer_exchange(store, {"id": "health", "method": "health"}))
    assert code != 0
    assert "not this peer's state" in stderr.decode()
    assert "Traceback" not in stderr.decode()
    assert store.read_text(encoding="utf-8") == before


@pytest.mark.parametrize("identifier", [None, 7, []])
def test_the_search_peer_refuses_malformed_task_identifiers(
    tmp_path: Path, identifier: JsonValue
) -> None:
    """A malformed identifier is refused instead of answering that no task exists."""
    store = tmp_path / "tracker.json"
    store.write_text(json.dumps({"requests": 0, "tasks": [task("t1", "Plain.")]}), encoding="utf-8")
    request: dict[str, JsonValue] = {
        "id": "bad-id",
        "method": "get_task",
        "params": {"id": identifier},
    }
    assert run(peer_request(store, request)).error.kind == "malformed"


@pytest.mark.parametrize("method", [None, 7, []])
def test_the_search_peer_ignores_malformed_request_methods(
    tmp_path: Path, method: JsonValue
) -> None:
    """Malformed envelopes do not enter dispatch or increment its request counter."""
    store = tmp_path / "tracker.json"
    before = json.dumps({"requests": 0, "tasks": [task("t1", "Plain.")]})
    store.write_text(before, encoding="utf-8")
    code, out, err = run(peer_exchange(store, {"id": "bad-method", "method": method}))
    assert code == 0 and not out
    assert "not a request" in err.decode()
    assert store.read_text(encoding="utf-8") == before


def test_the_search_peer_refuses_non_string_terms(tmp_path: Path) -> None:
    """Malformed protocol searches receive a response without changing a stored task."""
    store = tmp_path / "tracker.json"
    tasks = [task("t1", "A plain task.")]
    store.write_text(json.dumps({"requests": 0, "tasks": tasks}), encoding="utf-8")
    for terms, fields in [
        (7, "title-or-content"),
        (["a"], "title-or-content"),
        ({"term": "a"}, "title-or-content"),
        ("plain", "elsewhere"),
        ("plain", 7),
    ]:
        request: dict[str, JsonValue] = {
            "id": "bad-search",
            "method": "query_tasks",
            "params": {
                "query": {"text": {"terms": terms, "fields": fields}},
                "page": {"limit": 100, "cursor": None},
            },
        }
        assert run(peer_request(store, request)).error.kind == "malformed"
        assert json.loads(store.read_text(encoding="utf-8"))["tasks"] == tasks


def test_the_search_peer_refuses_malformed_labels_without_dropping_them(tmp_path: Path) -> None:
    """A malformed write receives a refusal and leaves the stored task whole."""
    store = tmp_path / "tracker.json"
    tasks = [task("t1", "A plain task.")]
    store.write_text(json.dumps({"requests": 0, "tasks": tasks}), encoding="utf-8")
    for label in [7, {}, {"id": 7, "name": "Bad"}]:
        request: dict[str, JsonValue] = {
            "id": "bad-label",
            "method": "write_task",
            "params": {
                "write": {
                    "target": "t1",
                    "item": {
                        "id": "t1",
                        "title": "Changed",
                        "content": "Changed.",
                        "status": {"category": "todo", "name": "Todo"},
                        "labels": [label],
                    },
                },
            },
        }
        assert run(peer_request(store, request)).error.kind == "malformed"
        assert json.loads(store.read_text(encoding="utf-8"))["tasks"] == tasks


def test_the_search_peer_refuses_unknown_status_categories(tmp_path: Path) -> None:
    """Reject an out-of-vocabulary status before replacing the stored record."""
    store = tmp_path / "tracker.json"
    tasks = [task("t1", "A plain task.")]
    store.write_text(json.dumps({"requests": 0, "tasks": tasks}), encoding="utf-8")
    item: dict[str, JsonValue] = {
        "id": "t1",
        "title": "Changed",
        "content": "Changed.",
        "labels": [],
        "status": {"category": "invented", "name": "Todo"},
    }
    request: dict[str, JsonValue] = {
        "id": "bad-category",
        "method": "write_task",
        "params": {"write": {"target": "t1", "item": item}},
    }
    assert run(peer_request(store, request)).error.kind == "malformed"
    assert json.loads(store.read_text(encoding="utf-8"))["tasks"] == tasks


async def peer_result(store: Path, request: dict[str, JsonValue]) -> dict[str, JsonValue]:
    """Read a successful protocol reply without discarding any of its JSON fields."""
    code, out, err = await peer_exchange(store, request)
    assert code == 0, err.decode()
    reply = TypeAdapter(dict[str, JsonValue]).validate_json(out)
    result = reply.get("result")
    assert isinstance(result, dict), reply
    return result


@pytest.mark.parametrize("method", ["query_tasks", "task_comments", "task_dependencies", "labels"])
@pytest.mark.parametrize(
    "page", [None, [], {"limit": 0}, {"limit": True}, {"limit": 101}, {"limit": 1, "cursor": "bad"}]
)
def test_the_peer_refuses_malformed_pages_for_every_paged_read(
    tmp_path: Path, method: str, page: JsonValue
) -> None:
    """Invalid pages never become an apparently complete empty or unbounded result."""
    store = tmp_path / "tracker.json"
    tasks = [task("t1", "Plain.")]
    store.write_text(json.dumps({"requests": 0, "tasks": tasks}), encoding="utf-8")
    params: dict[str, JsonValue] = {
        "id": "t1",
        "task": "t1",
        "direction": "depends-on",
        "query": {},
        "page": page,
    }
    assert (
        run(peer_request(store, {"id": "bad-page", "method": method, "params": params})).error.kind
        == "malformed"
    )
    assert json.loads(store.read_text(encoding="utf-8"))["tasks"] == tasks


def test_the_peer_returns_only_the_requested_task_page(tmp_path: Path) -> None:
    """A real request limit and continuation split two tasks into two pages."""
    store = tmp_path / "tracker.json"
    store.write_text(
        json.dumps({"requests": 0, "tasks": [task("t1", "First."), task("t2", "Second.")]}),
        encoding="utf-8",
    )
    first = run(
        peer_result(
            store,
            {
                "id": "first",
                "method": "query_tasks",
                "params": {"query": {}, "page": {"cursor": None, "limit": 1}},
            },
        )
    )
    assert first == {"items": [task("t1", "First.")], "next": "1"}
    second = run(
        peer_result(
            store,
            {
                "id": "second",
                "method": "query_tasks",
                "params": {"query": {}, "page": {"cursor": first["next"], "limit": 1}},
            },
        )
    )
    assert second == {"items": [task("t2", "Second.")], "next": None}


@pytest.mark.parametrize(
    ("method", "params"),
    [
        ("task_comments", {"task": 7, "page": {"limit": 1}}),
        ("task_dependencies", {"id": [], "direction": "depends-on", "page": {"limit": 1}}),
        ("task_dependencies", {"id": "t1", "direction": "sideways", "page": {"limit": 1}}),
    ],
)
def test_the_peer_refuses_malformed_comment_and_dependency_targets(
    tmp_path: Path, method: str, params: dict[str, JsonValue]
) -> None:
    """Malformed identifiers and directions cannot masquerade as empty traversals."""
    store = tmp_path / "tracker.json"
    store.write_text(json.dumps({"requests": 0, "tasks": [task("t1", "Plain.")]}), encoding="utf-8")
    assert (
        run(
            peer_request(store, {"id": "bad-target", "method": method, "params": params})
        ).error.kind
        == "malformed"
    )


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("metadata", []),
        ("repositories", [7]),
        ("classification", "everyone"),
        ("priority", "soon"),
        ("project", []),
        ("delivers", [7]),
        ("delivered_by", [7]),
        ("version", 7),
        ("created_at", 7),
    ],
)
def test_the_peer_validates_the_whole_incoming_task(
    tmp_path: Path, field: str, value: JsonValue
) -> None:
    """Reject malformed work fields before replacing any stored record."""
    store = tmp_path / "tracker.json"
    tasks = [task("t1", "Before.")]
    store.write_text(json.dumps({"requests": 0, "tasks": tasks}), encoding="utf-8")
    item = TypeAdapter(dict[str, JsonValue]).validate_python(task("t1", "Changed."))
    item[field] = value
    assert (
        run(
            peer_request(
                store,
                {
                    "id": "bad-task",
                    "method": "write_task",
                    "params": {"write": {"target": "t1", "item": item, "depends_on": []}},
                },
            )
        ).error.kind
        == "malformed"
    )
    assert json.loads(store.read_text(encoding="utf-8"))["tasks"] == tasks


@pytest.mark.parametrize(
    "edge",
    [
        7,
        {},
        {"from": {"id": "t1", "kind": "task"}, "to": {"id": 7, "kind": "task"}, "kind": "blocks"},
        {
            "from": {"id": "t1", "kind": "task"},
            "to": {"id": "t2", "kind": "task"},
            "kind": "unknown",
        },
    ],
)
def test_the_peer_refuses_malformed_edges_before_writing(tmp_path: Path, edge: JsonValue) -> None:
    """Invalid relationships cannot be silently discarded by a whole-task write."""
    store = tmp_path / "tracker.json"
    tasks = [task("t1", "Before.")]
    store.write_text(json.dumps({"requests": 0, "tasks": tasks}), encoding="utf-8")
    item = TypeAdapter(dict[str, JsonValue]).validate_python(task("t1", "Changed."))
    assert (
        run(
            peer_request(
                store,
                {
                    "id": "bad-edge",
                    "method": "write_task",
                    "params": {"write": {"target": "t1", "item": item, "depends_on": [edge]}},
                },
            )
        ).error.kind
        == "malformed"
    )
    assert json.loads(store.read_text(encoding="utf-8"))["tasks"] == tasks


def test_the_peer_preserves_written_work_fields_and_dependencies(tmp_path: Path) -> None:
    """Write through the protocol and read back the full task and its forward edge."""
    store = tmp_path / "tracker.json"
    store.write_text(
        json.dumps({"requests": 0, "tasks": [task("t1", "Before.")]}), encoding="utf-8"
    )
    item = TypeAdapter(dict[str, JsonValue]).validate_python(task("source-id", "Changed."))
    item.update(
        {
            "metadata": {"answer": "keep", "number": 7},
            "repositories": ["github.com/example/work"],
            "classification": "private",
            "priority": "high",
            "project": "p1",
            "url": "https://example.invalid/t1",
            "location": {"url": "https://example.invalid/t1"},
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
            "delivers": ["other:t2"],
            "delivered_by": ["origin:t1"],
        }
    )
    edge: dict[str, JsonValue] = {
        "from": {"id": "source-id", "kind": "task"},
        "to": {"id": "other:t2", "kind": "task"},
        "kind": "blocks",
    }
    assert run(
        peer_result(
            store,
            {
                "id": "write",
                "method": "write_task",
                "params": {"write": {"target": "t1", "item": item, "depends_on": [edge]}},
            },
        )
    ) == {"id": "t1"}
    read = run(peer_result(store, {"id": "read", "method": "get_task", "params": {"id": "t1"}}))[
        "task"
    ]
    assert isinstance(read, dict)
    assert read == {**item, "id": "t1"}
    page = run(
        peer_result(
            store,
            {
                "id": "deps",
                "method": "task_dependencies",
                "params": {"id": "t1", "direction": "depends-on", "page": {"limit": 1}},
            },
        )
    )
    assert page == {"items": [{**edge, "from": {"id": "t1", "kind": "task"}}], "next": None}


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one public async SDK call to completion."""
    return asyncio.run(call)


def policy(*, terms: bool = True) -> dict[str, object]:
    """A `write_policy` replacing `TERM`, listing its terms only when `terms` is set."""
    configured: dict[str, object] = {
        "check_command": [sys.executable, "-c", "import sys; sys.exit(2)"],
        "sanitize_command": [sys.executable, "-c", SANITIZE],
        "replacements": [{"term": TERM, "with": "ember-x"}],
    }
    if terms:
        configured["terms_command"] = [sys.executable, "-c", TERMS]
    return configured


def client(
    binary: Path, tmp_path: Path, sources: dict[str, object], **policy_options: bool
) -> Client:
    """A client over `sources` under [`policy`]."""
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps({"write_policy": policy(**policy_options), "sources": sources}),
        encoding="utf-8",
    )
    return Client(binary, cwd=tmp_path, environment=dict(os.environ))


class Task(TypedDict):
    """One task as `sweep_source.py` holds it and sends it (§4.4)."""

    id: TaskId
    title: str
    content: str
    status: Status
    labels: list[Label]
    project: None
    url: None
    location: None
    created_at: None
    updated_at: None


def task(identifier: str, content: str) -> Task:
    """One task as the peer holds it and sends it (§4.4)."""
    return {
        "id": identifier,
        "title": identifier.title(),
        "content": content,
        "status": {"category": StatusCategory.StatusCategoryTodo, "name": "Todo"},
        "labels": [],
        "project": None,
        "url": None,
        "location": None,
        "created_at": None,
        "updated_at": None,
    }


def test_a_write_sanitization_preview_is_typed_and_leaves_the_item_unchanged(
    binary: Path, tmp_path: Path
) -> None:
    """Preview an older item, then apply the same priority change through the SDK."""
    folder = tmp_path / "plans"
    (folder / "tasks").mkdir(parents=True)
    item = folder / "tasks" / "t1.md"
    before = f"---\ntitle: Retry\nstatus: todo\n---\nRetry as {TERM} does.\n"
    item.write_text(before, encoding="utf-8")
    sdk = client(
        binary,
        tmp_path,
        {"plans": {"plugin": "local-md", "visibility": "public", "config": {"root": str(folder)}}},
    )
    answer = run(sdk.task_priority_set("plans:t1", "high", sanitize_dry_run=True))
    from onetaskgraph_sdk import SanitizeDryRun

    assert isinstance(answer, SanitizeDryRun)
    assert answer.would_pass is True
    assert answer.sanitization.dry_run is True
    assert item.read_text(encoding="utf-8") == before
    written_answer = run(sdk.task_priority_set("plans:t1", "high"))
    assert written_answer.sanitization is not None
    assert written_answer.sanitization.side_effect
    written = item.read_text(encoding="utf-8")
    assert TERM not in written
    assert "ember-x" in written
    assert "priority: high" in written
    unchanged = run(sdk.task_priority_set("plans:t1", "low"))
    assert unchanged.sanitization is None


def test_a_search_sweep_answers_its_report_on_exit_three(binary: Path, tmp_path: Path) -> None:
    """A metered source is searched, and the classes a search cannot cover come back named."""
    store = tmp_path / "tracker.json"
    store.write_text(
        json.dumps(
            {
                "requests": 0,
                "tasks": [task("t1", f"Retry as {TERM} does."), task("t2", "Nothing here.")],
            }
        ),
        encoding="utf-8",
    )
    sdk = client(
        binary,
        tmp_path,
        {
            "tracker": {
                "plugin": "subprocess",
                "visibility": "public",
                "config": {
                    "command": sys.executable,
                    "args": [str(Path(__file__).with_name("sweep_source.py")), str(store)],
                },
            }
        },
    )
    report = run(sdk.sources_sanitize("tracker"))
    assert report.coverage.complete is False
    assert [item.value for item in report.coverage.uncovered] == [
        "metadata",
        "comments",
        "normalized-spellings",
    ]
    assert [(item.id, item.outcome.value) for item in report.items] == [("tracker:t1", "rewritten")]
    assert report.requests.total > 0
    held = json.loads(store.read_text(encoding="utf-8"))["tasks"]
    assert [entry["content"] for entry in held if entry["id"] == "t1"] == ["Retry as ember-x does."]


def test_a_sweep_leaving_an_identifier_unfixable_answers_its_report_on_exit_one(
    binary: Path, tmp_path: Path
) -> None:
    """An item whose own id carries the term is reported unfixable, its id redacted."""
    tasks = tmp_path / "site" / "tasks"
    tasks.mkdir(parents=True)
    (tasks / f"{TERM}-note.md").write_text(
        "---\ntitle: Note\nstatus: todo\n---\nPlain.\n", encoding="utf-8"
    )
    sdk = client(
        binary,
        tmp_path,
        {"site": {"plugin": "local-md", "config": {"root": "site"}, "visibility": "public"}},
    )
    report = run(sdk.sources_sanitize("site"))
    assert report.coverage.complete is True
    assert len(report.items) == 1
    unfixable = report.items[0]
    assert unfixable.outcome.value == "unfixable"
    assert unfixable.id == {"redacted": True, "kind": "item", "ordinal": 1}
    field = unfixable.fields[0].root
    assert field.outcome == "unfixable"
    assert getattr(field, "why", None) == "identifier carries a private term"


def test_a_sweep_that_cannot_run_still_raises(binary: Path, tmp_path: Path) -> None:
    """With no `terms_command` there is no report, and the refusal is the SDK's error."""
    (tmp_path / "site").mkdir()
    sdk = client(
        binary,
        tmp_path,
        {"site": {"plugin": "local-md", "config": {"root": "site"}, "visibility": "public"}},
        terms=False,
    )
    with pytest.raises(OnetaskgraphError) as refused:
        run(sdk.sources_sanitize("site"))
    assert refused.value.exit_code == 1
    assert "terms_command" in str(refused.value)


def test_the_peers_dependency_directions_match_the_plugin_api() -> None:
    """Reconcile the unexported Direction enum, including any newly added variant."""
    # Direction has no CLI schema root. Read its authoritative declaration; unlike an
    # example list, consuming every variant makes an API addition fail this drift check.
    source = (
        Path(__file__).resolve().parents[3] / "crates/onetaskgraph-plugin-api/src/work.rs"
    ).read_text()
    declared = re.search(
        r'#[\[]serde\(rename_all = "kebab-case"\)[\]]\s*pub enum Direction \{([^}]+)\}', source
    )
    assert declared is not None, "Direction must still use the protocol's kebab-case vocabulary"
    variants = []
    for line in declared[1].splitlines():
        line = line.strip()
        if not line or line.startswith("//"):
            continue
        assert re.fullmatch(r"[A-Z][A-Za-z0-9]*,", line), (
            f"review the changed Direction variant: {line}"
        )
        variants.append(re.sub(r"(?<!^)(?=[A-Z])", "-", line[:-1]).lower())
    assert get_args(DependencyDirection) == tuple(variants)
