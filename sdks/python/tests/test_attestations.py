"""Content-keyed attestations recorded and checked through the SDK, against the real binary.

Each call drives the binary over a real folder of Markdown, and every answer is validated
against the model generated from the schema root the binary emits for it — `Attested`,
`AttestationCheck` and `ProjectAttestationCheck` — so the client, the models and the binary's
own answers are held to one declaration.
"""

from __future__ import annotations

import asyncio
import json
from collections.abc import Coroutine
from pathlib import Path

import pytest

from onetaskgraph_sdk import (
    Attestation,
    AttestationCheck,
    AttestationState,
    Attested,
    Client,
    OnetaskgraphError,
    ProjectAttestationCheck,
    VoidReason,
)


def run[T](call: Coroutine[object, object, T]) -> T:
    """Drive one public async SDK call to completion."""
    return asyncio.run(call)


def plan(binary: Path, tmp_path: Path) -> tuple[Client, str, str]:
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
    return client, "notes:P-1", str(created.items[0].id.root)


def test_a_record_reads_back_as_its_models_and_goes_void_when_content_moves(
    binary: Path, tmp_path: Path
) -> None:
    """A record, its checks and a project's per-task check validate as the emitted models."""
    client, project, task = plan(binary, tmp_path)
    absent = run(client.task_attestation_check(task, "plan-review"))
    assert isinstance(absent, AttestationCheck)
    assert (absent.state, absent.reason, absent.record) == (AttestationState("absent"), None, None)

    attested = run(
        client.task_attest(task, "plan-review", "ada", "bar-1", note="fine", exclude=["myapp.x"])
    )
    assert isinstance(attested, Attested)
    record = attested.record
    # The record as an item's metadata holds it is the `Attestation` root, field for field.
    assert (
        Attestation.model_validate(record.model_dump(mode="json", exclude_none=True)).bar == "bar-1"
    )
    assert (record.kind.root, record.by, record.bar, record.note) == (
        "plan-review",
        "ada",
        "bar-1",
        "fine",
    )
    assert record.excludes == ["myapp.x"]
    assert record.content_digest.startswith("sha256:")

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
        (task, AttestationState("void"))
    ]


def test_a_refused_attestation_raises_with_the_failure(binary: Path, tmp_path: Path) -> None:
    """A kind the binary refuses raises carrying the failure document's kind."""
    client, _, task = plan(binary, tmp_path)
    with pytest.raises(OnetaskgraphError) as refused:
        run(client.task_attest(task, "plan review", "ada", "bar-1"))
    assert refused.value.exit_code == 1
    assert refused.value.failure is not None
    assert refused.value.failure.kind == "invalid-attestation"
