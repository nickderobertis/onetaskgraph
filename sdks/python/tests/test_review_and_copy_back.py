"""The plan-review and copy-back additions, through the SDK against the real binary.

`project_attestation_check(tasks=True)` narrowed by `status`, `not_status` and `state`,
`task_list(not_status=...)`, `task_digest` and `project_copy(update_fields=..., metadata=...)`
each answer what the command line answers, validated as the models the binary's schema roots
generate.
"""

from __future__ import annotations

import asyncio
import json
import sys
from collections.abc import Coroutine
from pathlib import Path

from onetaskgraph_sdk import Client, CopyReport, GlobalId, ProjectAttestationCheck
from onetaskgraph_sdk._generated.copy_report import CopyOutcomeCreatedOrWouldCreate
from onetaskgraph_sdk._generated.task_content_digest import TaskContentDigest


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one public async SDK call to completion."""
    return asyncio.run(call)


def folders(tmp_path: Path) -> None:
    """Configure `authoring`, declared private, and `board`, declared public."""
    for name in ("authoring", "board"):
        (tmp_path / name).mkdir()
    passing = 'import sys; sys.stdin.read(); print(\'{"verdict": "pass"}\')'
    public = 'import sys; sys.stdin.read(); print(\'{"visibility": "public"}\')'
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps(
            {
                "sources": {
                    "authoring": {
                        "plugin": "local-md",
                        "visibility": "private",
                        "config": {"root": "authoring"},
                    },
                    "board": {
                        "plugin": "local-md",
                        "visibility": "public",
                        "config": {"root": "board"},
                    },
                },
                "write_policy": {
                    "check_command": [sys.executable, "-c", passing],
                    "visibility_command": [sys.executable, "-c", public],
                },
            }
        ),
        encoding="utf-8",
    )


def created(report: CopyReport, source: str) -> str:
    """The id the copy `report` created for `source`."""
    outcome = next(item.root for item in report.items if item.root.source.root == source)
    assert isinstance(outcome, CopyOutcomeCreatedOrWouldCreate)
    assert outcome.destination is not None
    return outcome.destination.root


def test_a_review_s_open_tasks_and_a_task_s_digest(binary: Path, tmp_path: Path) -> None:
    """The filters keep exactly the open tasks owing a review; the digest is the record's."""
    (tmp_path / "notes").mkdir()
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps({"sources": {"notes": {"plugin": "local-md", "config": {"root": "notes"}}}}),
        encoding="utf-8",
    )
    client = Client(binary, cwd=tmp_path)
    run(client.project_create("notes", "P-1", "Plan", body="The plan.\n"))
    ids = {}
    for title, status in (("open", "todo"), ("reviewed", "todo"), ("shipped", "done")):
        created = run(client.task_create("notes", "P-1", title, body=f"{title}\n", status=status))
        ids[title] = created.items[0].id.root
    record = run(
        client.task_attest(ids["reviewed"], "plan-review", "ada", "bar-1", exclude=["myapp.x"])
    ).record

    owed = run(
        client.project_attestation_check(
            GlobalId("notes:P-1"),
            "plan-review",
            tasks=True,
            not_status=["done", "cancelled"],
            state=["void", "absent"],
        )
    )
    assert isinstance(owed, ProjectAttestationCheck)
    assert [entry.id.root for entry in owed.tasks] == [ids["open"]]
    valid = run(
        client.project_attestation_check(
            GlobalId("notes:P-1"), "plan-review", tasks=True, status=["todo"], state=["valid"]
        )
    )
    assert isinstance(valid, ProjectAttestationCheck)
    assert [entry.id.root for entry in valid.tasks] == [ids["reviewed"]]

    listed = run(client.task_list(project="notes:P-1", not_status=["done"]))
    assert sorted(task.id.root for task in listed.items) == sorted([ids["open"], ids["reviewed"]])

    digest = run(client.task_digest(ids["reviewed"], exclude=["myapp.x"]))
    assert isinstance(digest, TaskContentDigest)
    assert digest.content_digest.root == record.content_digest.root


def test_a_copy_back_writes_the_status_alone_and_overrides_a_key(
    binary: Path, tmp_path: Path
) -> None:
    """A board's copy of a private plan copies back in place, its status and keys alone."""
    folders(tmp_path)
    client = Client(binary, cwd=tmp_path)
    run(client.project_create("authoring", "plan-1", "Plan", body="The plan.\n"))
    task = run(client.task_create("authoring", "plan-1", "Build", body="Build it.\n"))
    authored = task.items[0].id.root
    onto = run(client.project_copy("authoring:plan-1", to="board"))
    board_project = created(onto, "authoring:plan-1")
    board_task = created(onto, authored)
    run(client.task_status_set(board_task, "done"))

    back = run(
        client.project_copy(
            board_project,
            to="authoring",
            update_fields="status",
            metadata=['host.m={"a":1}'],
        )
    )
    assert isinstance(back, CopyReport)
    assert sorted(item.root.action for item in back.items) == ["updated", "updated"]
    shown = run(client.task_show(authored, no_comments=True)).items[0].item
    assert shown.status.category.value == "done"
    assert shown.content == "Build it.\n"
    assert shown.metadata["host.m"] == {"a": 1}
