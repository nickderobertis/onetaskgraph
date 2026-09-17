"""Public-boundary tests for the generated SDK."""

from __future__ import annotations

import asyncio
import json
import os
import subprocess
import sys
import threading
import tomllib
from collections.abc import Coroutine
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

from onetaskgraph_sdk import (
    Client,
    Delivered,
    GlobalId,
    MetadataSet,
    OnetaskgraphError,
    SourceName,
    StatusCategory,
    TaskStatusSet,
    __version__,
)
from onetaskgraph_sdk._generated.copy_report import CopyOutcome
from onetaskgraph_sdk._generated.models import QueryResponseOfQualifiedTask
from onetaskgraph_sdk._generated.task_status_set import DeliveredFailed

WORKSPACE = Path(__file__).parents[3]


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one public async SDK call to completion in a script-shaped test.

    A coroutine rather than any awaitable: `asyncio.run` accepts other awaitables only from
    Python 3.14, and this package supports 3.13.
    """
    return asyncio.run(call)


def configured(tmp_path: Path, *, failing: bool = False) -> Path:
    """Create real Markdown and in-memory sources, optionally with one broken source."""
    markdown = tmp_path / "markdown" / "tasks"
    markdown.mkdir(parents=True)
    (markdown / "M-1.md").write_text(
        "---\ntitle: Markdown task\nstatus: Todo\n---\nBody from disk\n", encoding="utf-8"
    )
    sources: dict[str, object] = {
        "memory": {
            "plugin": "in-memory",
            "config": {
                # A source holding documents has to say so: the engine reads the
                # declaration once at the handshake and never asks a source that says it
                # has none, so a `documents:` list without this key is refused.
                "capabilities": {"documents": "native"},
                "documents": [
                    {
                        "id": "D-1",
                        "title": "Memory document",
                        "content": "the engine core, reviewed",
                        "project": "P-1",
                        "labels": [{"id": "L-1", "name": "sdk"}],
                        "location": {"url": "https://example.invalid/D-1"},
                        "metadata": {"caller.note": "a string"},
                        "repositories": ["github.com/nickderobertis/onetaskgraph"],
                    }
                ],
                "tasks": [
                    {
                        "id": "T-1",
                        "title": "Memory task",
                        "status": {"category": "todo", "name": "Todo"},
                        "labels": [],
                        "project": "P-1",
                        "metadata": {
                            "onepipeline.turn_budget": 12,
                            "caller.flags": [True, None],
                            "caller.shape": {"nested": "value"},
                        },
                        "repositories": ["github.com/nickderobertis/onetaskgraph"],
                    }
                ],
                "projects": [
                    {
                        "id": "P-1",
                        "title": "Memory project",
                        "status": {"category": "todo", "name": "Todo"},
                        "labels": [],
                        "metadata": {"onepipeline.publication": {"mode": "review"}},
                        "repositories": ["github.com/nickderobertis/onetaskgraph"],
                    }
                ],
                "labels": [{"id": "L-1", "name": "sdk"}],
                "task_dependencies": [
                    {
                        "from": "T-1",
                        "to": {"id": "elsewhere:P-9", "kind": "project"},
                        "kind": "blocks",
                    }
                ],
                "project_dependencies": [
                    {
                        "from": "P-1",
                        "to": {"id": "elsewhere:T-9", "kind": "task"},
                        "kind": "blocks",
                    }
                ],
            },
        },
        "markdown": {
            "plugin": "local-md",
            "config": {"root": str(tmp_path / "markdown"), "status_mapping": {"todo": "todo"}},
        },
    }
    if failing:
        sources["broken"] = {"plugin": "github-projects", "config": {}}
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    return tmp_path


def test_real_sources_and_typed_partial_failure(binary: Path, tmp_path: Path) -> None:
    """Return validated rows, plans, and a typed failure from actual sources."""
    client = Client(
        binary,
        cwd=configured(tmp_path, failing=True),
        environment={"ONETASKGRAPH_SDK_BINARY": str(tmp_path / "wrong")},
    )
    response = run(client.task_list())
    assert {item.item.title for item in response.items} == {"Markdown task", "Memory task"}
    assert {plan.source.root for plan in response.plan.per_source} == {"memory", "markdown"}
    assert response.errors[0].source.root == "broken"
    assert response.errors[0].error.root.kind == "config"


def test_public_error_contains_exit_status(binary: Path, tmp_path: Path) -> None:
    """Expose a malformed invocation as the documented typed client exception."""
    client = Client(binary, cwd=configured(tmp_path))
    with pytest.raises(OnetaskgraphError) as caught:
        run(client.task_show(id="not-qualified"))
    assert caught.value.exit_code == 1
    assert "qualify the id" in str(caught.value)


def test_real_binary_response_is_rejected_against_the_wrong_contract(
    binary: Path, tmp_path: Path
) -> None:
    """Reject real process JSON when it does not match the selected generated model."""
    client = Client(binary, cwd=configured(tmp_path))
    with pytest.raises(OnetaskgraphError, match="outside its emitted schema"):
        run(client._invoke(["sources", "list"], QueryResponseOfQualifiedTask))


def test_every_generated_method_drives_the_binary(binary: Path, tmp_path: Path) -> None:
    """Exercise every generated command method and representative CLI option shapes."""
    client = Client(binary, cwd=configured(tmp_path))
    assert run(
        client.task_list(
            source=("memory",),
            status=["todo"],
            limit=2,
            explain=True,
            page=None,
            no_project=False,
        )
    ).items
    assert run(client.task_show(id=GlobalId(root="memory:T-1"))).items
    assert run(client.task_deps(id="memory:T-1")).items
    assert run(client.project_list(source=["memory"])).items
    assert run(client.project_show(id="memory:P-1")).items
    assert run(client.project_deps(id="memory:P-1")).items
    assert run(client.document_list(source=["memory"], limit=2, explain=True)).items
    assert run(client.document_show(id=GlobalId(root="memory:D-1"))).items
    assert run(client.label_list(source=["memory"])).items
    assert run(client.search(text="Memory", kind="task")).items
    assert run(client.sources_list())
    assert run(client.config_show()).settings


def test_status_options_method_passes_source_and_apply_to_the_real_binary(
    binary: Path, tmp_path: Path
) -> None:
    """Send the typed source operand and boolean flag through the subprocess boundary."""
    client = Client(binary, cwd=configured(tmp_path))
    with pytest.raises(OnetaskgraphError) as caught:
        run(client.sources_status_options(source=SourceName(root="memory"), apply=True))
    assert caught.value.exit_code == 1
    assert "source memory uses plugin in-memory" in str(caught.value)


def test_status_options_method_decodes_a_real_binary_plan(binary: Path, tmp_path: Path) -> None:
    """Decode the adapter's real GraphQL response through the generated SDK model."""

    class BoardHandler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:  # noqa: N802  # stdlib handler API names the method.
            length = int(self.headers["content-length"])
            request = json.loads(self.rfile.read(length))
            assert "optionId" in request["query"]
            options = [
                {
                    "id": f"OPT-{index}",
                    "name": name,
                    "color": "GRAY",
                    "description": "",
                }
                # Every option the shipped mapping names, terminal ones included: a
                # terminal write refuses without its option, so the plan counts
                # `Done` and `Cancelled` as configured and would report them missing.
                for index, name in enumerate(
                    ["Backlog", "Todo", "Queued", "In Progress", "Done", "Cancelled"],
                    start=1,
                )
            ]
            response = json.dumps(
                {
                    "data": {
                        "owner": {
                            "projectV2": {
                                "id": "PVT-board",
                                "fields": {
                                    "nodes": [
                                        {
                                            "id": "FIELD-status",
                                            "name": "Status",
                                            "options": options,
                                        }
                                    ],
                                    "pageInfo": {"hasNextPage": False},
                                },
                                "items": {
                                    "nodes": [],
                                    "pageInfo": {
                                        "hasNextPage": False,
                                        "endCursor": None,
                                    },
                                },
                            }
                        }
                    }
                }
            ).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)

        def log_message(self, format: str, *args: object) -> None:
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), BoardHandler)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    try:
        config = {
            "sources": {
                "board": {
                    "plugin": "github-projects",
                    "config": {
                        "owner": "fixture-owner",
                        "project_number": 7,
                        "token_env": "TEST_GITHUB_TOKEN",
                        "endpoint": f"http://127.0.0.1:{server.server_port}",
                    },
                }
            }
        }
        (tmp_path / "onetaskgraph.yaml").write_text(json.dumps(config), encoding="utf-8")
        client = Client(
            binary,
            cwd=tmp_path,
            environment={**os.environ, "TEST_GITHUB_TOKEN": "fixture-token"},
        )
        report = run(client.sources_status_options(source=SourceName(root="board")))
        assert report.source.root == "board"
        assert report.missing == []
        assert report.outcome.value == "planned"
        assert [option.id.root for option in report.existing] == [
            "OPT-1",
            "OPT-2",
            "OPT-3",
            "OPT-4",
            "OPT-5",
            "OPT-6",
        ]
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def landed(outcome: CopyOutcome) -> str:
    """The id a copy wrote to, absent only for a dry run that would have created one."""
    reported = outcome.root
    assert reported.destination is not None, "this copy wrote, so it reports where"
    return reported.destination.root


def folders(tmp_path: Path) -> Path:
    """Configure two real Markdown folders, one holding a task and one empty."""
    source = tmp_path / "from" / "tasks"
    source.mkdir(parents=True)
    (source / "T-1.md").write_text(
        "---\ntitle: Alpha engine\nstatus: todo\nmetadata: {caller.count: 3}\n---\nthe core\n",
        encoding="utf-8",
    )
    (tmp_path / "into").mkdir()
    folder = {"status_mapping": {"todo": "todo"}}
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps(
            {
                "sources": {
                    "from": {
                        "plugin": "local-md",
                        "config": {"root": str(tmp_path / "from"), **folder},
                    },
                    "into": {
                        "plugin": "local-md",
                        "config": {"root": str(tmp_path / "into"), **folder},
                    },
                    "sealed": {
                        "plugin": "in-memory",
                        "config": {"capabilities": {"writes": "unsupported"}},
                    },
                }
            }
        ),
        encoding="utf-8",
    )
    return tmp_path


def test_copy_drives_the_binary_and_reports_each_item(binary: Path, tmp_path: Path) -> None:
    """Copy through the real binary: a dry run, a create, an update, and a refusal."""
    client = Client(binary, cwd=folders(tmp_path))

    planned = run(client.task_copy(ids=["from:T-1"], to="into", dry_run=True))
    assert [
        (item.root.source.root, item.root.destination, item.root.action) for item in planned.items
    ] == [("from:T-1", None, "created")]
    assert not (tmp_path / "into" / "tasks").exists()

    created = run(client.task_copy(ids=[GlobalId(root="from:T-1")], to="into"))
    assert [(item.root.source.root, landed(item), item.root.action) for item in created.items] == [
        ("from:T-1", "into:T-1", "created")
    ]
    # The destination really holds it, read back through the same binary.
    assert run(client.task_show(id="into:T-1")).items[0].item.metadata == {
        "caller.count": 3,
        "onetaskgraph.origin": "from:T-1",
    }

    (tmp_path / "into" / "tasks" / "T-1.md").write_text(
        "---\ntitle: Alpha engine, edited\nstatus: todo\n"
        "metadata: {caller.count: 3, onetaskgraph.origin: from:T-1}\n---\nthe core\n",
        encoding="utf-8",
    )
    updated = run(client.task_copy(ids=["into:T-1"], to="from"))
    assert [(landed(item), item.root.action) for item in updated.items] == [("from:T-1", "updated")]
    assert run(client.task_show(id="from:T-1")).items[0].item.title == "Alpha engine, edited"

    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_copy(ids=["from:T-1"], to="sealed"))
    assert refused.value.exit_code == 1
    assert "sealed cannot be written" in str(refused.value)


def test_project_copy_drives_the_binary(binary: Path, tmp_path: Path) -> None:
    """Copy a project and the tasks in it, then copy it again without duplicating them."""
    root = folders(tmp_path)
    (root / "from" / "projects").mkdir()
    (root / "from" / "projects" / "P-1.md").write_text(
        "---\ntitle: Engine\nstatus: todo\n---\nthe engine\n", encoding="utf-8"
    )
    (root / "from" / "tasks" / "T-1.md").write_text(
        "---\ntitle: Alpha engine\nstatus: todo\nproject: P-1\n---\nthe core\n",
        encoding="utf-8",
    )
    client = Client(binary, cwd=root)

    first = run(client.project_copy(id="from:P-1", to="into"))
    assert [(item.root.source.root, item.root.action) for item in first.items] == [
        ("from:P-1", "created"),
        ("from:T-1", "created"),
    ]
    second = run(client.project_copy(id="from:P-1", to="into"))
    assert [item.root.action for item in second.items] == ["unchanged", "unchanged"]
    assert [item.id.root for item in run(client.task_list(source=["into"])).items] == ["into:T-1"]

    # The project and exactly the members named. Two folders of Markdown count nothing they
    # send, so the report carries no `spent` rather than a zero.
    narrowed = run(client.project_copy(id="from:P-1", to="into", member=[GlobalId("from:T-1")]))
    assert [(item.root.source.root, item.root.action) for item in narrowed.items] == [
        ("from:P-1", "unchanged"),
        ("from:T-1", "unchanged"),
    ]
    assert narrowed.spent is None
    with pytest.raises(OnetaskgraphError) as refused:
        run(client.project_copy(id="from:P-1", to="into", member=["from:T-9"]))
    assert refused.value.exit_code == 1
    assert "from:T-9 is not a task of from:P-1" in str(refused.value)


def test_document_copy_drives_the_binary_and_refuses_a_destination_with_no_documents(
    binary: Path, tmp_path: Path
) -> None:
    """Copy a document by id through the real binary, and be refused by a source with none.

    `markdown` is a `local-md` source, whose documents are files that outlive the process,
    so what one SDK call copied the next one reads back — which is what proves the copy
    landed rather than only that a report was printed. `sealed` is an `in-memory` source
    without the `documents` capability, so it holds none: the refusal comes from the
    handshake, before anything is read.
    """
    root = configured(tmp_path)
    document = json.loads((root / "onetaskgraph.yaml").read_text(encoding="utf-8"))
    document["sources"]["sealed"] = {"plugin": "in-memory", "config": {}}
    (root / "onetaskgraph.yaml").write_text(json.dumps(document), encoding="utf-8")
    client = Client(binary, cwd=root)

    planned = run(client.document_copy(ids=["memory:D-1"], to="markdown", dry_run=True))
    assert [
        (item.root.source.root, item.root.destination, item.root.action) for item in planned.items
    ] == [("memory:D-1", None, "created")]
    assert not (root / "markdown" / "documents").exists()

    created = run(client.document_copy(ids=[GlobalId(root="memory:D-1")], to="markdown"))
    assert [(item.root.source.root, landed(item), item.root.action) for item in created.items] == [
        ("memory:D-1", "markdown:D-1", "created")
    ]

    # The destination really holds it, read back through the same binary: every field the
    # copy carried with its JSON types intact, and — the whole model, so what is absent is
    # asserted too — a location that is the destination's own, the path of the file this
    # folder put it in rather than the URL the source reported, and no URL or timestamps,
    # which this destination does not have for a document it just wrote.
    copied = run(client.document_show(id="markdown:D-1")).items[0].item
    dumped = copied.model_dump(mode="json", exclude_none=True)
    # The location is compared by the file it names rather than by how it is spelled: this
    # source canonicalizes, and a canonical path is spelled differently on each platform —
    # macOS resolves the temporary tree's symlink, and Windows answers with an
    # extended-length `\\?\` path that no other language writes. `samefile` asks the
    # operating system the question the assertion is really making, and answers it on all
    # three; it also proves the path names a file that is really there, which comparing two
    # strings does not.
    located = Path(dumped.pop("location")["path"])
    assert located.samefile(root / "markdown" / "documents" / "D-1.md")
    assert dumped == {
        "id": "D-1",
        "title": "Memory document",
        "content": "the engine core, reviewed",
        "project": "P-1",
        "labels": [{"id": "L-1", "name": "sdk"}],
        "metadata": {"caller.note": "a string", "onetaskgraph.origin": "memory:D-1"},
        "repositories": ["github.com/nickderobertis/onetaskgraph"],
    }

    with pytest.raises(OnetaskgraphError) as refused:
        run(client.document_copy(ids=["memory:D-1"], to="sealed"))
    assert refused.value.exit_code == 1
    assert "has no documents" in str(refused.value)


def test_comment_methods_drive_the_binary(binary: Path, tmp_path: Path) -> None:
    """Add, list, edit and delete a task's comments through the real binary.

    `notes` is a folder of Markdown, whose comments are a section of the task's own file, so
    what one call writes the next one reads back. `plain` is an in-memory source that does not
    say its tasks have comments, so every comment call against it is refused before anything
    is read.
    """
    (tmp_path / "notes" / "tasks").mkdir(parents=True)
    (tmp_path / "notes" / "tasks" / "T-1.md").write_text(
        "---\ntitle: Ship the release\nstatus: todo\n---\nLong-form task content.\n",
        encoding="utf-8",
    )
    plain_task = {
        "id": "T-1",
        "title": "Plain",
        "status": {"category": "todo", "name": "Todo"},
        "labels": [],
    }
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps(
            {
                "sources": {
                    "notes": {"plugin": "local-md", "config": {"root": str(tmp_path / "notes")}},
                    "plain": {"plugin": "in-memory", "config": {"tasks": [plain_task]}},
                }
            }
        ),
        encoding="utf-8",
    )
    client = Client(binary, cwd=tmp_path)

    # A body handed over as text reaches the binary on standard input, byte for byte.
    first = run(
        client.task_comment_add(
            id="notes:T-1", body="Seen again on main:\n\n## Evidence\n", author="ada"
        )
    )
    assert (first.body, first.author) == ("Seen again on main:\n\n## Evidence\n", "ada")
    body_file = tmp_path / "second.md"
    # Written as bytes: text mode would translate the newline on Windows, and the
    # binary hands the file back byte for byte.
    body_file.write_bytes(b"from a file\n")
    second = run(client.task_comment_add(id=GlobalId(root="notes:T-1"), body_file=str(body_file)))
    assert second.body == "from a file\n"

    listed = run(client.task_comment_list(id="notes:T-1"))
    assert [comment.id.root for comment in listed.comments] == [first.id.root, second.id.root]

    edited = run(
        client.task_comment_edit(id="notes:T-1", comment_id=first.id.root, body="corrected\n")
    )
    assert (edited.id, edited.author, edited.created_at, edited.body) == (
        first.id,
        first.author,
        first.created_at,
        "corrected\n",
    )

    deleted = run(client.task_comment_delete(id="notes:T-1", comment_id=second.id.root))
    assert deleted.deleted.root == second.id.root
    shown = run(client.task_show(id="notes:T-1"))
    assert shown.comments is not None
    assert [comment.body for comment in shown.comments] == ["corrected\n"]
    assert shown.items[0].item.content == "Long-form task content."

    # A source whose tasks have none carries no comments key, and refuses the verbs.
    assert run(client.task_show(id="plain:T-1")).comments is None
    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_comment_list(id="plain:T-1"))
    assert refused.value.exit_code == 1
    assert "has no comments" in str(refused.value)

    # No body at all is refused rather than waiting on this process's own standard input.
    with pytest.raises(OnetaskgraphError) as empty:
        run(client.task_comment_add(id="notes:T-1"))
    assert empty.value.exit_code == 1
    assert "is empty" in str(empty.value)
    assert [comment.body for comment in run(client.task_comment_list(id="notes:T-1")).comments] == [
        "corrected\n"
    ]
    with pytest.raises(TypeError, match="comment_id"):
        run(client._invoke(["task", "comment", "delete"], object, id="notes:T-1"))


def delivering_folder(tmp_path: Path) -> Path:
    """Configure one real Markdown folder, `work`, holding a plain task and a delivering one.

    A folder of Markdown outlives the invocation, so what one call writes the next one reads
    back. `P` delivers a task of `nowhere`, which no configuration names, so every write of
    its status re-evaluates a delivered task that cannot be read.
    """
    tasks = tmp_path / "work" / "tasks"
    tasks.mkdir(parents=True)
    (tasks / "T-1.md").write_text("---\ntitle: One\nstatus: todo\n---\n", encoding="utf-8")
    (tasks / "P.md").write_text(
        '---\ntitle: Parent\nstatus: todo\ndelivers: ["nowhere:T-9"]\n---\n', encoding="utf-8"
    )
    work = {"plugin": "local-md", "config": {"root": str(tmp_path / "work")}}
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps({"sources": {"work": work}}), encoding="utf-8"
    )
    return tmp_path


def test_task_status_set_drives_the_binary(binary: Path, tmp_path: Path) -> None:
    """Set one task's status through the real binary, read it back, and be refused by name."""
    client = Client(binary, cwd=delivering_folder(tmp_path))

    answer = run(
        client.task_status_set(id="work:T-1", category=StatusCategory.StatusCategoryQueued)
    )
    assert isinstance(answer, TaskStatusSet)
    assert answer.id.root == "work:T-1"
    assert answer.status.category == "queued"
    assert answer.delivered == []

    # The folder really holds it: a later invocation reads the status this one wrote.
    shown = run(client.task_show(id=GlobalId(root="work:T-1"))).items[0].item
    assert (shown.title, shown.status.category) == ("One", StatusCategory.StatusCategoryQueued)

    # The category is accepted as the binary spells it, too.
    moved = run(client.task_status_set(id=GlobalId(root="work:T-1"), category="in-progress"))
    assert moved.status.category == "in-progress"

    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_status_set(id="missing:T-1", category="queued"))
    assert refused.value.exit_code == 1
    assert 'no source named "missing" is configured' in str(refused.value)
    assert run(client.task_show(id="work:T-1")).items[0].item.status.category == "in-progress"


def test_task_status_set_answers_when_a_delivered_task_could_not_be_kept_in_step(
    binary: Path, tmp_path: Path
) -> None:
    """Exit 4 is a write that landed with a delivered task it could not reach, not a failure.

    The client hands back the whole answer the binary wrote — the status set, and the task it
    could not keep in step with why — rather than raising on the exit code as it does for 1.
    """
    client = Client(binary, cwd=delivering_folder(tmp_path))

    # The invocation the client makes, observed at the process boundary: it exits 4.
    arguments = [client.binary, "task", "status", "set", "work:P", "in-progress", "--json"]
    completed = run(client._invoke_process(arguments))
    assert completed.returncode == 4
    assert "nowhere:T-9 could not be kept in step with work:P" in completed.stderr

    answer = run(client.task_status_set(id="work:P", category="queued"))
    assert (answer.id.root, answer.status.category) == ("work:P", "queued")
    [entry] = answer.delivered
    outcome = entry.root
    assert isinstance(outcome, DeliveredFailed)
    assert (outcome.ticket.root, outcome.deliverer.root, outcome.outcome) == (
        "nowhere:T-9",
        "work:P",
        "failed",
    )
    assert outcome.failure.kind == "unknown-source"
    # The entry reads as the published `Delivered` root too, not only as a nested model.
    republished = Delivered.model_validate(entry.model_dump(mode="json", by_alias=True))
    assert republished.root.outcome == "failed"

    shown = run(client.task_show(id="work:P")).items[0].item
    assert shown.status.category == "queued"
    assert [reference.root for reference in shown.delivers or []] == ["nowhere:T-9"]


def metadata_folder(tmp_path: Path) -> Path:
    """Configure one real Markdown folder, `work`, holding a task, a project and a document.

    Each record already carries a metadata key of its own, so a set that disturbed anything
    beside the key it names would show in what a later invocation reads back.
    """
    root = tmp_path / "work"
    for kind in ("tasks", "projects", "documents"):
        (root / kind).mkdir(parents=True)
    kept = 'metadata:\n  "myapp.kept": 1\n'
    (root / "tasks" / "T-1.md").write_text(
        f"---\ntitle: One\nstatus: todo\n{kept}---\nbody\n", encoding="utf-8"
    )
    (root / "projects" / "P-1.md").write_text(
        f"---\ntitle: Plan\nstatus: todo\n{kept}---\n", encoding="utf-8"
    )
    (root / "documents" / "D-1.md").write_text(
        f"---\ntitle: Design\n{kept}---\nprose\n", encoding="utf-8"
    )
    work = {"plugin": "local-md", "config": {"root": str(root)}}
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps({"sources": {"work": work}}), encoding="utf-8"
    )
    return tmp_path


def test_metadata_set_methods_drive_the_binary(binary: Path, tmp_path: Path) -> None:
    """Set one metadata key of a task, a project and a document through the real binary."""
    cwd = metadata_folder(tmp_path)
    client = Client(binary, cwd=cwd)

    task = run(client.task_metadata_set("work:T-1", "myapp.review", '{"approved": true}'))
    assert isinstance(task, MetadataSet)
    assert (task.id.root, task.key.root, task.value) == (
        "work:T-1",
        "myapp.review",
        {"approved": True},
    )
    # Compared by the file it names, as the document copy above explains: a canonical path is
    # spelled differently on each platform.
    assert task.location is not None
    located = task.location.root.model_dump()
    assert list(located) == ["path"]
    assert Path(located["path"]).samefile(cwd / "work" / "tasks" / "T-1.md")

    project = run(
        client.project_metadata_set(id=GlobalId(root="work:P-1"), key="myapp.review", value="3")
    )
    assert isinstance(project, MetadataSet)
    assert (project.id.root, project.value) == ("work:P-1", 3)

    document = run(client.document_metadata_set("work:D-1", "myapp.review", "null"))
    assert isinstance(document, MetadataSet)
    assert (document.id.root, document.value) == ("work:D-1", None)

    # The folder really holds each: a later invocation reads what these wrote, beside the key
    # each record already had.
    shown = run(client.task_show(id="work:T-1")).items[0].item
    assert shown.metadata == {"myapp.kept": 1, "myapp.review": {"approved": True}}
    assert run(client.project_show(id="work:P-1")).items[0].item.metadata == {
        "myapp.kept": 1,
        "myapp.review": 3,
    }
    assert run(client.document_show(id="work:D-1")).items[0].item.metadata == {
        "myapp.kept": 1,
        "myapp.review": None,
    }

    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_metadata_set("work:T-1", "onetaskgraph.origin", '"x"'))
    assert refused.value.exit_code == 1
    assert "which this product owns" in str(refused.value)
    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_metadata_set("work:T-1", "myapp.review", "yes"))
    assert "is not JSON" in str(refused.value)
    assert shown.metadata == run(client.task_show(id="work:T-1")).items[0].item.metadata


def test_binary_resolution_order(binary: Path, tmp_path: Path) -> None:
    """Use environment before the packaged PATH fallback and reject no executable."""
    config = configured(tmp_path)
    relative_binary = Path(os.path.relpath(binary, Path.cwd()))
    from_explicit_relative = Client(relative_binary, cwd=config)
    assert run(from_explicit_relative.task_list()).items
    from_environment = Client(
        cwd=config, environment={"ONETASKGRAPH_SDK_BINARY": str(relative_binary), "PATH": ""}
    )
    assert run(from_environment.task_list()).items
    from_path = Client(cwd=config, environment={"PATH": str(binary.parent)})
    assert run(from_path.task_list()).items
    with pytest.raises(FileNotFoundError, match="binary not found"):
        Client(environment={"PATH": ""})
    with pytest.raises(FileNotFoundError, match="not an executable"):
        Client(tmp_path / "missing")


def test_generator_rejects_drift_and_unmapped_commands(tmp_path: Path) -> None:
    """Name stale output and a newly discovered command with no client method."""
    expected = tmp_path / "expected"
    actual = tmp_path / "actual"
    expected.mkdir()
    actual.mkdir()
    (expected / "effective_config.py").write_text("current", encoding="utf-8")
    (actual / "effective_config.py").write_text("stale", encoding="utf-8")
    stale = subprocess.run(
        [
            sys.executable,
            "-c",
            "from pathlib import Path; import generate; "
            f"generate.check_generated(Path({str(expected)!r}), Path({str(actual)!r}))",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert stale.returncode == 1
    assert "effective_config.py" in stale.stderr
    assert "uv run python generate.py" in stale.stderr
    invalid_bundle = subprocess.run(
        [sys.executable, "-c", "import generate; generate.validate_schema_bundle([])"],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert invalid_bundle.returncode == 1
    assert "invalid schema bundle" in invalid_bundle.stderr
    changed_option = subprocess.run(
        [
            sys.executable,
            "-c",
            "import generate; generate.validate_option_placeholders({'limit':'TEXT'}, ['limit'])",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert changed_option.returncode == 1
    assert "value shape" in changed_option.stderr
    missing_roots = subprocess.run(
        [sys.executable, "-c", "import generate; generate.validate_schema_bundle({'roots':{}})"],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert missing_roots.returncode == 1
    assert "missing roots" in missing_roots.stderr
    missing_choices = subprocess.run(
        [sys.executable, "-c", "import generate; generate.choice_values('', 'kind')"],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert missing_choices.returncode == 1
    assert "did not report option" in missing_choices.stderr
    empty_choices = subprocess.run(
        [
            sys.executable,
            "-c",
            "import generate; generate.choice_values('--kind <KIND>\\n', 'kind')",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert empty_choices.returncode == 1
    assert "possible values" in empty_choices.stderr
    malformed_root = subprocess.run(
        [
            sys.executable,
            "-c",
            "import json, generate; "
            "bundle=json.loads(generate.run_workspace_binary('schema')); "
            "bundle['roots']['QueryPlan']=[]; generate.validate_schema_bundle(bundle)",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert malformed_root.returncode == 1
    assert "non-object roots: QueryPlan" in malformed_root.stderr
    missing_minimum = subprocess.run(
        [
            sys.executable,
            "-c",
            "import generate; generate.documented_minimum({}, 'Capabilities.max_page_size')",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert missing_minimum.returncode == 1
    assert "Capabilities.max_page_size" in missing_minimum.stderr
    unmapped = subprocess.run(
        [
            sys.executable,
            "-c",
            "from pathlib import Path; import generate; "
            f"generate.generate_client([('future',)], Path({str(tmp_path)!r}))",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert unmapped.returncode == 1
    assert "future" in unmapped.stderr


# llmlint: ignore-block[async_typed_clients_at_boundaries] The subprocess boundary is what is
# under test: this asserts that a generation which could not run the binary puts the binary's
# own words on stderr and exits non-zero, which is what a CI log shows and what an in-process
# call could not observe. Every case in this file drives the real generator the same way, and
# the async typed client this rule asks for is the package's own `Client`, not this test.
def test_generator_reports_what_a_failing_binary_said() -> None:
    """Carry the binary's own diagnosis out of a generation that could not run it.

    The generator captures the binary's output because stdout is the schema bundle it
    consumes, so a failure whose reason went to stderr is a failure with no reason in the
    log — which is what one CI run of this target reported and nobody could act on.
    """
    failed = subprocess.run(
        [sys.executable, "-c", "import generate; generate.run_workspace_binary('not-a-command')"],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert failed.returncode == 1
    assert "exited 2" in failed.stderr
    assert "unrecognized subcommand 'not-a-command'" in failed.stderr


# llmlint: ignore-end[async_typed_clients_at_boundaries]


def test_generator_write_mode_uses_real_binary(tmp_path: Path) -> None:
    """Regenerate into a fresh destination from the real schema and command surface."""
    destination = tmp_path / "generated"
    subprocess.run(
        [
            sys.executable,
            "-c",
            "import json; from pathlib import Path; import generate; "
            "bundle=generate.validate_schema_bundle("
            "json.loads(generate.run_workspace_binary('schema'))); "
            f"generate.generate(bundle, check=False, destination=Path({str(destination)!r}))",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        check=True,
    )
    generated_client = (destination / "client.py").read_text(encoding="utf-8")
    generated_report = (destination / "status_options_report.py").read_text(encoding="utf-8")
    assert "async def sources_status_options(" in generated_client
    assert "source: SourceName | str" in generated_client
    assert "apply: bool | None = None" in generated_client
    assert generated_report.count("min_length=1") == 2


def test_distribution_version() -> None:
    """Keep the one public version aligned with the manifest."""
    manifest = tomllib.loads((WORKSPACE / "sdks" / "python" / "pyproject.toml").read_text())
    assert __version__ == manifest["project"]["version"]


def test_metadata_and_repositories_survive_the_generated_models(
    binary: Path, tmp_path: Path
) -> None:
    """Read caller metadata and repository origins back through the validated models."""
    client = Client(binary, cwd=configured(tmp_path))

    origins = ["github.com/nickderobertis/onetaskgraph"]

    task = run(client.task_show(id="memory:T-1")).items[0].item
    assert task.metadata == {
        "onepipeline.turn_budget": 12,
        "caller.flags": [True, None],
        "caller.shape": {"nested": "value"},
    }
    assert [repository.root for repository in task.repositories or []] == origins

    project = run(client.project_show(id="memory:P-1")).items[0].item
    assert project.metadata == {"onepipeline.publication": {"mode": "review"}}
    assert [repository.root for repository in project.repositories or []] == origins

    hit = run(client.search(text="Memory", kind="task")).items[0].root
    assert (hit.item.metadata or {})["onepipeline.turn_budget"] == 12
    assert [repository.root for repository in hit.item.repositories or []] == origins


def test_a_dependency_endpoint_carries_its_kind_and_may_leave_the_source(
    binary: Path, tmp_path: Path
) -> None:
    """Read a typed, qualified endpoint of another source back through the models."""
    client = Client(binary, cwd=configured(tmp_path))

    edge = run(client.task_deps(id="memory:T-1")).items[0]
    assert edge.from_.id.root == "memory:T-1"
    assert edge.from_.kind == "task"
    assert edge.to.id.root == "elsewhere:P-9"
    assert edge.to.kind == "project"
    assert edge.kind == "blocks"

    across_levels = run(client.project_deps(id="memory:P-1")).items[0]
    assert across_levels.from_.kind == "project"
    assert across_levels.to.id.root == "elsewhere:T-9"
    assert across_levels.to.kind == "task"

    # The far source is not configured, so reporting the edge cannot have resolved it.
    with pytest.raises(OnetaskgraphError, match="elsewhere"):
        run(client.project_show(id="elsewhere:P-9"))
