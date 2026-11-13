"""Content-keyed attestations recorded and checked through the SDK, against the real binary.

Each call drives the binary over a real folder of Markdown, and every answer is validated
against the model generated from the schema root the binary emits for it — `Attested`,
`AttestationCheck` and `ProjectAttestationCheck` — so the client, the models and the binary's
own answers are held to one declaration.
"""

from __future__ import annotations

import asyncio
import json
import sys
from collections.abc import Coroutine
from pathlib import Path
from typing import NamedTuple

import pytest

from onetaskgraph_sdk import (
    Attestation,
    AttestationCheck,
    AttestationState,
    Attested,
    Client,
    CopyReport,
    GlobalId,
    OnetaskgraphError,
    ProjectAttestationCheck,
    VoidReason,
)
from onetaskgraph_sdk._generated.copy_report import CopyOutcomeCreatedOrWouldCreate
from onetaskgraph_sdk._generated.project_attestation_check import (
    AttestationState as TaskAttestationState,
)


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one public async SDK call to completion."""
    return asyncio.run(call)


class Plan(NamedTuple):
    """A client over a seeded folder, and the qualified ids of what it holds."""

    client: Client
    project: GlobalId
    task: GlobalId


def plan(binary: Path, tmp_path: Path) -> Plan:
    """A client over a folder of Markdown with one project and one task in it."""
    (tmp_path / "notes").mkdir()
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps({"sources": {"notes": {"plugin": "local-md", "config": {"root": "notes"}}}}),
        encoding="utf-8",
    )
    client = Client(binary, cwd=tmp_path)
    run(client.project_create("notes", "P-1", "Plan", body="The plan.\n"))
    created = run(
        client.task_create("notes", "P-1", "Build", body="Build it.\n", metadata=["myapp.size=1"])
    )
    return Plan(client, GlobalId("notes:P-1"), GlobalId(created.items[0].id.root))


def test_a_record_reads_back_as_its_models_and_goes_void_when_content_moves(
    binary: Path, tmp_path: Path
) -> None:
    """A record, its checks and a project's per-task check validate as the emitted models."""
    seeded = plan(binary, tmp_path)
    client, project, task = seeded.client, seeded.project, seeded.task
    absent = run(client.task_attestation_check(task, "plan-review"))
    assert isinstance(absent, AttestationCheck)
    assert (absent.state, absent.reason, absent.record) == (AttestationState("absent"), None, None)

    attested = run(
        client.task_attest(task, "plan-review", "ada", "bar-1", note="fine", exclude=["myapp.x"])
    )
    assert isinstance(attested, Attested)
    record = attested.record
    note = record.note
    # The record as an item's metadata holds it is the `Attestation` root, field for field.
    assert (
        Attestation.model_validate(record.model_dump(mode="json", exclude_none=True)).bar.root
        == "bar-1"
    )
    assert note is not None
    assert (record.kind.root, record.by.root, record.bar.root, note.root) == (
        "plan-review",
        "ada",
        "bar-1",
        "fine",
    )
    assert [key.root for key in record.excludes or []] == ["myapp.x"]
    assert record.content_digest.root.startswith("sha256:")

    valid = run(client.task_attestation_check(task, "plan-review", bar="bar-1"))
    assert valid.state == AttestationState("valid")
    assert valid.record is not None
    assert valid.record.model_dump(mode="json") == record.model_dump(mode="json")
    moved_bar = run(client.task_attestation_check(task, "plan-review", bar="bar-2"))
    assert (moved_bar.state, moved_bar.reason) == (
        AttestationState("void"),
        VoidReason("bar-moved"),
    )

    run(client.task_metadata_set(task, "myapp.size", "2"))
    moved = run(client.task_attestation_check(task, "plan-review", bar="bar-1"))
    assert (moved.state, moved.reason) == (AttestationState("void"), VoidReason("content-moved"))

    run(client.project_attest(project, "plan-review", "ada", "bar-1"))
    own = run(client.project_attestation_check(project, "plan-review"))
    assert isinstance(own, AttestationCheck)
    assert own.state == AttestationState("valid")
    tasks = run(client.project_attestation_check(project, "plan-review", tasks=True))
    assert isinstance(tasks, ProjectAttestationCheck)
    assert [(entry.id.root, entry.state) for entry in tasks.tasks] == [
        (task.root, AttestationState("void"))
    ]


def test_a_refused_attestation_raises_with_the_failure(binary: Path, tmp_path: Path) -> None:
    """A kind the binary refuses raises carrying the failure document's kind."""
    seeded = plan(binary, tmp_path)
    client, task = seeded.client, seeded.task
    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_attest(task, "plan review", "ada", "bar-1"))
    assert refused.value.exit_code == 1
    assert refused.value.failure is not None
    assert refused.value.failure.kind == "invalid-attestation"


class TaskState(NamedTuple):
    """One task's attestation state, as a project's per-task check answers it."""

    task: GlobalId
    state: TaskAttestationState


class Landed(NamedTuple):
    """Where one item a copy read landed."""

    source: GlobalId
    destination: GlobalId


def test_a_dependent_task_copied_out_of_a_private_folder_keeps_its_review(
    binary: Path, tmp_path: Path
) -> None:
    """B, depending on A, copied from a private folder to a public one checks ``valid`` there.

    A copy out of a source declared private records no origin naming it, so the copy of A
    names no root of its own; the store finds it through the link the private A keeps. The
    copy's edge pointed elsewhere still voids the review.
    """
    for folder in ("authoring", "published"):
        (tmp_path / folder).mkdir()
    passing = [sys.executable, "-c", 'import sys; sys.stdin.read(); print(\'{"verdict": "pass"}\')']
    public = [
        sys.executable,
        "-c",
        'import sys; sys.stdin.read(); print(\'{"visibility": "public"}\')',
    ]
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps(
            {
                "sources": {
                    "authoring": {
                        "plugin": "local-md",
                        "visibility": "private",
                        "config": {"root": "authoring"},
                    },
                    "published": {
                        "plugin": "local-md",
                        "visibility": "public",
                        "config": {"root": "published"},
                    },
                },
                "write_policy": {"check_command": passing, "visibility_command": public},
            }
        ),
        encoding="utf-8",
    )
    client = Client(binary, cwd=tmp_path)
    run(client.project_create("authoring", "launch", "Launch", body="The plan.\n"))
    a = run(client.task_create("authoring", "launch", "A", body="Do A.\n")).items[0].id.root
    b = (
        run(client.task_create("authoring", "launch", "B", body="Do B.\n", depends_on=[a]))
        .items[0]
        .id.root
    )
    c = run(client.task_create("authoring", "launch", "C", body="Do C.\n")).items[0].id.root
    for task in (a, b, c):
        run(client.task_attest(task, "plan-review", "review-plan", "bar-1"))

    def states(project: GlobalId) -> list[TaskState]:
        """Each task's state under `project`, in qualified-id order."""
        checked = run(client.project_attestation_check(project, "plan-review", tasks=True))
        assert isinstance(checked, ProjectAttestationCheck)
        held = [TaskState(GlobalId(entry.id.root), entry.state) for entry in checked.tasks]
        return sorted(held, key=lambda state: state.task.root)

    def all_valid(ids: list[GlobalId]) -> list[TaskState]:
        valid = TaskAttestationState("valid")
        return sorted((TaskState(task, valid) for task in ids), key=lambda state: state.task.root)

    originals = [GlobalId(task) for task in (a, b, c)]
    assert states(GlobalId("authoring:launch")) == all_valid(originals)
    report = run(client.project_copy("authoring:launch", to="published", documents=True))
    assert isinstance(report, CopyReport)
    landed: list[Landed] = []
    for outcome in (item.root for item in report.items):
        assert isinstance(outcome, CopyOutcomeCreatedOrWouldCreate)
        assert outcome.destination is not None
        landed.append(Landed(GlobalId(outcome.source.root), GlobalId(outcome.destination.root)))

    def copy_of(source: GlobalId) -> GlobalId:
        return next(each.destination for each in landed if each.source == source)

    copies = [copy_of(task) for task in originals]
    assert states(copy_of(GlobalId("authoring:launch"))) == all_valid(copies)

    run(client.task_update(copies[1], depends_on=[copies[2]]))
    moved = run(client.task_attestation_check(copies[1], "plan-review", bar="bar-1"))
    assert (moved.state, moved.reason) == (AttestationState("void"), VoidReason("content-moved"))
