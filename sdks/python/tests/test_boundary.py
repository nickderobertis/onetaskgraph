"""The public boundary through the SDK: a write's `term_scope` reaches the store unchanged.

Every call drives the real binary over real folders of Markdown, under a `write_policy` whose
check command records the input the store hands it — which is what makes "unchanged" something
a test can read — and passes it, or refuses one carrying `REFUSE` the way a check refuses.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
from collections.abc import Coroutine
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError

CHECK = """
import json, os, sys
received = sys.stdin.read()
with open(os.environ["BOUNDARY_RECORD"], "a", encoding="utf-8") as record:
    record.write(json.dumps(json.loads(received)) + "\\n")
if "REFUSE" in received:
    print(json.dumps({"verdict": "refuse", "surface": "text"}))
    sys.exit(1)
print(json.dumps({"verdict": "pass"}))
"""


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one public async SDK call to completion."""
    return asyncio.run(call)


def store(binary: Path, tmp_path: Path) -> tuple[Client, Path]:
    """A client over a public folder `site` and a private one `plan`, and the check's record."""
    for folder in ("site", "plan"):
        (tmp_path / folder).mkdir()
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps(
            {
                "write_policy": {"check_command": [sys.executable, "-c", CHECK]},
                "sources": {
                    "site": {
                        "plugin": "local-md",
                        "config": {"root": "site"},
                        "visibility": "public",
                    },
                    "plan": {
                        "plugin": "local-md",
                        "config": {"root": "plan"},
                        "visibility": "private",
                    },
                },
            }
        ),
        encoding="utf-8",
    )
    record = tmp_path / "checked.jsonl"
    environment = dict(os.environ)
    environment["BOUNDARY_RECORD"] = str(record)
    return Client(binary, cwd=tmp_path, environment=environment), record


def scopes(record: Path) -> list[list[str] | None]:
    """The scope of every input the check was handed, `None` where none was sent."""
    return [
        json.loads(line).get("scope")
        for line in record.read_text(encoding="utf-8").splitlines()
        if line
    ]


def test_a_create_status_metadata_and_update_pass_their_term_scope_through_unchanged(
    binary: Path, tmp_path: Path
) -> None:
    """A list, an empty list and none each reach the check exactly as they were passed."""
    client, record = store(binary, tmp_path)
    named = run(
        client.task_create(
            "site",
            "p",
            "Named",
            body="Generic.",
            term_scope=["github.com/example-org/widget", "github.com/example-org/gadget"],
        )
    )
    task = named.items[0].id.root
    run(client.task_create("site", "p", "Empty", body="Generic.", term_scope=[]))
    run(client.task_create("site", "p", "Unscoped", body="Generic."))
    run(client.task_status_set(task, "done", term_scope=["github.com/example-org/widget"]))
    run(client.task_metadata_set(task, "team.note", '"generic"', term_scope=[]))
    run(client.task_update(task, title="Renamed", term_scope=["github.com/example-org/widget"]))
    assert scopes(record) == [
        ["github.com/example-org/widget", "github.com/example-org/gadget"],
        [],
        None,
        ["github.com/example-org/widget"],
        [],
        ["github.com/example-org/widget"],
    ]
    # A bare string is deliberately the wrong type: this proves the SDK refuses it at run time
    # rather than sending each of its characters as a repository.
    with pytest.raises(TypeError, match="term_scope"):
        run(client.task_create("site", "p", "Bad", body="x", term_scope="github.com/a/b"))  # ty: ignore[invalid-argument-type]


def test_a_private_item_stays_in_a_private_source_and_a_refusal_reaches_the_caller(
    binary: Path, tmp_path: Path
) -> None:
    """Classification is carried and enforced, and the check's refusal is the SDK's error."""
    client, record = store(binary, tmp_path)
    secret = run(
        client.task_create("plan", "p", "Secret", body="Generic.", classification="private")
    )
    assert secret.items[0].item.classification == "private"
    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_copy([secret.items[0].id.root], to="site", term_scope=[]))
    assert "not declared private" in str(refused.value)
    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_create("site", "p", "Refused", body="REFUSE this", term_scope=[]))
    assert "term of a private repository in its text" in str(refused.value)
    assert not list((tmp_path / "site").rglob("*.md"))
    assert scopes(record) == [[]]
