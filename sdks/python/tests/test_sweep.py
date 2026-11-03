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
import sys
from collections.abc import Coroutine
from pathlib import Path
from typing import TypedDict

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError

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

    id: str
    title: str
    content: str
    status: dict[str, str]
    labels: list[dict[str, str]]
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
        "status": {"category": "todo", "name": "Todo"},
        "labels": [],
        "project": None,
        "url": None,
        "location": None,
        "created_at": None,
        "updated_at": None,
    }


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
