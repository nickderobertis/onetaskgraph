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
    # Each holds the project `p` its items are filed under, public on `site` and private on
    # `plan`: while the boundary is active, a project its source does not hold is no project to
    # file under.
    for folder, classification in (("site", "public"), ("plan", "private")):
        (tmp_path / folder / "projects").mkdir(parents=True)
        (tmp_path / folder / "projects" / "p.md").write_text(
            f"---\ntitle: P\nstatus: todo\nclassification: {classification}\n---\n",
            encoding="utf-8",
        )
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
    assert not list((tmp_path / "site" / "tasks").rglob("*.md"))
    assert scopes(record) == [[]]


OPENCO_PUBLIC = """
import json, sys
asked = sys.stdin.read()
print(json.dumps({"visibility": "public" if "openco" in asked else "private"}))
"""


def projecting_store(binary: Path, tmp_path: Path) -> Client:
    """A private plan `plan:rollout` in `plan`, `vault` private and routing `openco` work to `site`.

    The plan holds a private producer, of the private `hiddenco/quietharbor`, and public work
    of `openco/widget` depending on it.
    """
    (tmp_path / "plan" / "projects").mkdir(parents=True)
    (tmp_path / "plan" / "tasks").mkdir(parents=True)
    (tmp_path / "vault").mkdir()
    (tmp_path / "site").mkdir()
    (tmp_path / "plan" / "projects" / "rollout.md").write_text(
        "---\ntitle: Quietharbor rollout\nstatus: todo\n---\nThe private plan.\n",
        encoding="utf-8",
    )
    (tmp_path / "plan" / "tasks" / "producer.md").write_text(
        "---\ntitle: Harden the ingest\nstatus: todo\nproject: rollout\n"
        "repositories: [github.com/hiddenco/quietharbor]\n---\nPrivate work.\n",
        encoding="utf-8",
    )
    (tmp_path / "plan" / "tasks" / "widget.md").write_text(
        "---\ntitle: Add retry to the widget\nstatus: todo\nproject: rollout\n"
        "repositories: [github.com/openco/widget]\ndepends_on: [producer]\n---\nGeneric.\n",
        encoding="utf-8",
    )
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps(
            {
                "write_policy": {
                    "visibility_command": [sys.executable, "-c", OPENCO_PUBLIC],
                    "check_command": [sys.executable, "-c", CHECK],
                },
                "sources": {
                    "plan": {
                        "plugin": "local-md",
                        "config": {"root": "plan"},
                        "visibility": "private",
                    },
                    "vault": {
                        "plugin": "local-md",
                        "config": {"root": "vault"},
                        "visibility": "private",
                        "routes": [{"repositories": ["github.com/openco/*"], "to": "site"}],
                    },
                    "site": {
                        "plugin": "local-md",
                        "config": {"root": "site"},
                        "visibility": "public",
                    },
                },
            }
        ),
        encoding="utf-8",
    )
    environment = dict(os.environ)
    environment["BOUNDARY_RECORD"] = str(tmp_path / "checked.jsonl")
    return Client(binary, cwd=tmp_path, environment=environment)


def test_a_project_copy_records_its_public_projections_in_the_shape_the_sdk_models(
    binary: Path, tmp_path: Path
) -> None:
    """The map the binary records reads back through the generated model and round-trips."""
    from onetaskgraph_sdk._generated.public_projections import PublicProjections

    client = projecting_store(binary, tmp_path)
    report = run(client.project_copy("plan:rollout", to="vault"))
    landed: dict[str, str] = {}
    for item in report.items:
        # A failed or skipped outcome has no destination; every outcome here landed.
        destination = getattr(item.root, "destination", None)
        if destination is not None:
            landed[item.root.source.root] = destination.root
    home = landed["plan:rollout"]
    shown = run(client.project_show(home))
    recorded = shown.items[0].item.metadata["onetaskgraph.public_projections"]
    projections = PublicProjections.model_validate(recorded)
    assert projections.schema_version == 1
    assert [entry.task.root for entry in projections.tasks] == [landed["plan:widget"]]
    view = projections.tasks[0].projection.root
    assert view.startswith("site:pt-")
    assert projections.model_dump(mode="json") == recorded
    # The store owns each task's visibility: the producer is private, the widget public.
    producer = run(client.task_show(landed["plan:producer"])).items[0].item
    widget = run(client.task_show(landed["plan:widget"])).items[0].item
    assert producer.classification == "private"
    assert widget.classification == "public"
    # A canonical status change reaches the view as its category alone.
    status = run(client.task_status_set(landed["plan:widget"], "done"))
    assert status.projection is not None
    assert status.projection.root == view
    assert run(client.task_show(view)).items[0].item.status.category == "done"
    # A malformed map is refused by the model exactly as the engine refuses it.
    with pytest.raises(ValueError):
        PublicProjections.model_validate({"schema_version": 1, "tasks": [], "extra": True})
