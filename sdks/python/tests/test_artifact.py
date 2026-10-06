"""Installed-wheel journey."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import pydantic

if TYPE_CHECKING:
    from pydantic import JsonValue


# llmlint: ignore-block[test_tiers_split_by_project_not_by_marker] This is the sdk-python
# project's own installed-wheel journey: it builds this package's wheel and installs it into
# a scratch environment, in seconds, from the project's own lockfile. A project of its own
# would carry exactly this project's inputs and so be selected in exactly the same cases;
# the split would add an Nx project and buy no edge.
def test_wheel_installs_and_queries_through_public_import(tmp_path: Path, binary: Path) -> None:
    """Install a wheel cleanly and drive a real configured query through it."""
    package = Path(__file__).parents[1]
    subprocess.run(["uv", "build", "--wheel", "--out-dir", str(tmp_path)], cwd=package, check=True)
    venv = tmp_path / "venv"
    subprocess.run(["uv", "venv", "--python", sys.executable, str(venv)], check=True)
    python = venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    wheel = next(tmp_path.glob("onetaskgraph_sdk-*.whl"))
    requirements = tmp_path / "requirements.txt"
    subprocess.run(
        [
            "uv",
            "export",
            "--frozen",
            "--no-dev",
            "--no-emit-project",
            "--output-file",
            str(requirements),
        ],
        cwd=package,
        check=True,
    )
    subprocess.run(
        ["uv", "pip", "install", "--python", str(python), "--requirement", str(requirements)],
        check=True,
    )
    subprocess.run(
        [
            "uv",
            "pip",
            "install",
            "--offline",
            "--no-deps",
            "--python",
            str(python),
            str(wheel),
        ],
        check=True,
    )
    config = tmp_path / "query"
    config.mkdir()
    (config / "onetaskgraph.yaml").write_text(
        '{"sources":{"work":{"plugin":"in-memory","config":{"tasks":['
        '{"id":"T-1","title":"Installed","status":{"category":"todo","name":"Todo"},'
        '"labels":[]}]}}}}',
        encoding="utf-8",
    )
    script = (
        "import asyncio; from onetaskgraph_sdk import Client; "
        f"r=asyncio.run(Client(cwd={str(config)!r}).task_list()); "
        "assert r.items[0].item.title == 'Installed'"
    )
    child_environment = dict(os.environ)
    child_environment["ONETASKGRAPH_SDK_BINARY"] = str(binary)
    subprocess.run([str(python), "-c", script], env=child_environment, check=True)


# llmlint: ignore-end[test_tiers_split_by_project_not_by_marker]


def test_the_generated_package_carries_every_type_of_the_documents_contract() -> None:
    """The SDK owes a caller a model per contract type, verb or no verb.

    No command returns a document, so nothing in `RESPONSE_ROOTS` would reach these four.
    They are generated from the bundle deliberately: a generated surface that waited for a
    verb would leave this SDK describing a different contract from the TypeScript one and
    from the schema both are generated against.
    """
    from onetaskgraph_sdk._generated.models import (
        Document,
        DocumentQuery,
        Location,
        PageOfDocument,
    )

    # A document is not work: no status, and no dependency key.
    assert "status" not in Document.model_fields
    assert "depends_on" not in Document.model_fields
    for field in ("id", "title", "content", "project", "labels", "url", "location"):
        assert field in Document.model_fields, field

    # A `DocumentQuery` carries no statuses, for the same reason.
    assert "statuses" not in DocumentQuery.model_fields
    for field in ("text", "labels", "project"):
        assert field in DocumentQuery.model_fields, field

    filed = {
        "id": "D-1",
        "title": "Why the store holds a document",
        "content": "A person cannot review a plan node by node.",
        "project": "P-1",
        "labels": [{"id": "L-1", "name": "design", "color": None}],
        "url": "https://example.invalid/D-1",
        "location": {"path": "/home/someone/notes/design.md"},
        "created_at": None,
        "updated_at": None,
    }
    document = Document.model_validate(filed)
    assert document.model_dump(mode="json", exclude_none=True)["location"] == {
        "path": "/home/someone/notes/design.md"
    }
    # Round trip, so the location survives being written back out and read again.
    assert Document.model_validate(document.model_dump(mode="json")) == document

    # A consumer tells the two location variants apart by which key is present.
    linked = Location.model_validate({"url": "https://example.invalid/D-1"})
    assert linked.model_dump(mode="json") == {"url": "https://example.invalid/D-1"}
    on_disk = Location.model_validate({"path": "/home/someone/notes/design.md"})
    assert on_disk.model_dump(mode="json") == {"path": "/home/someone/notes/design.md"}

    page = PageOfDocument.model_validate({"items": [filed], "next": "b2Zmc2V0PTE"})
    # `id` is a `NativeId`, which is a root model over the source's own opaque string.
    assert [item.id.root for item in page.items] == ["D-1"]
    assert page.next is not None


def test_the_reference_figures_round_trip_absent_zero_and_nonzero() -> None:
    """The three figures are additive, and this is what that has to mean to a caller.

    The binary omits a figure of zero rather than writing a nought, so the document a copy
    that recognised nothing emits is byte-for-byte what one emitted before these figures
    existed. This model has to read that as zeroes rather than as nulls, put a figure back
    on the wire the way the binary would, and carry a figure that has something to say.
    """
    from onetaskgraph_sdk._generated.models import CopyReport

    # Absent reads back as zero, which is what the schema's `"default": 0` buys a generated
    # consumer: without it this field would model as null and a caller would branch on it.
    absent = CopyReport.model_validate(
        {"items": [{"source": "work:D-1", "action": "created", "destination": "notes:D-1"}]}
    )
    assert (
        absent.references_rewritten,
        absent.references_unresolved,
        absent.references_ambiguous,
    ) == (0, 0, 0)

    assert absent.model_dump(mode="json", exclude_defaults=True) == {
        "items": [{"source": "work:D-1", "action": "created", "destination": "notes:D-1"}]
    }

    # A figure with something to say survives both directions, the ambiguous one being a
    # sub-count of the unresolved one rather than a second total.
    reported = CopyReport.model_validate(
        {
            "items": [],
            "references_rewritten": 3,
            "references_unresolved": 2,
            "references_ambiguous": 1,
        }
    )
    assert reported.references_ambiguous is not None
    assert reported.references_unresolved is not None
    assert reported.references_ambiguous <= reported.references_unresolved
    dumped = reported.model_dump(mode="json", exclude_defaults=True)
    assert dumped == {
        "items": [],
        "references_rewritten": 3,
        "references_unresolved": 2,
        "references_ambiguous": 1,
    }
    assert CopyReport.model_validate(dumped) == reported

    partial = CopyReport.model_validate({"items": [], "references_rewritten": 2})
    assert (partial.references_unresolved, partial.references_ambiguous) == (0, 0)
    assert partial.model_dump(mode="json", exclude_defaults=True) == {
        "items": [],
        "references_rewritten": 2,
    }


def test_an_omitted_location_and_an_omitted_documents_capability_read_as_their_defaults() -> None:
    """Both members this contract added are optional, and both defaults are documented.

    `location` absent means *the source did not say where this is* — not that it is
    nowhere. `documents` absent means the plugin predates documents and is read as the
    document-free source it is. Neither may become a decode failure, because that is what
    would make this addition a breaking one.
    """
    from onetaskgraph_sdk._generated.models import Document, SourceListing

    bare = Document.model_validate(
        {
            "id": "D-2",
            "title": "A source that did not say",
            "content": None,
            "project": None,
            "labels": [],
            "url": None,
            "created_at": None,
            "updated_at": None,
        }
    )
    assert bare.location is None

    # A handshake written before there were documents omits the member entirely.
    listing = SourceListing.model_validate(
        {
            "kind": "in-memory",
            "source": "work",
            "state": "available",
            "capabilities": {
                "projects": "native",
                "orphan_tasks": "native",
                "filter_by_label": "native",
                "filter_by_status": "native",
                "search_title": "native",
                "search_content": "native",
                "task_dependencies": "both-directions",
                "project_dependencies": "both-directions",
                "max_page_size": 50,
            },
        }
    )
    # `SourceListing` is a root model over the available/unavailable pair.
    assert listing.root.state == "available"
    assert listing.root.capabilities.documents == "unsupported"


# llmlint: ignore-block[async_typed_clients_at_boundaries] The one call here reads the schema
# bundle the binary emits, exactly as generate.py does under its own directive: a build-time
# artifact read once, with no service on the other end and nothing to overlap. The async typed
# client this rule asks for is the package's own `Client`, which the rest of this suite drives.
def test_the_generated_package_is_built_from_the_schema_bundle_this_sdk_expects(
    binary: Path,
) -> None:
    """The bundle version is what lets an SDK refuse a bundle it was not generated for.

    Version 8 published the documents contract's four types; version 9 published the two
    roots a document *read* answers with, which is what made those types reachable. So this
    asserts the version and the roots together — a version bumped without them, or them
    without the bump, is the drift the number exists to make visible.
    """
    import json
    import subprocess
    import sys
    from pathlib import Path

    sys.path.insert(0, str(Path(__file__).parents[1]))
    import generate

    emitted = subprocess.run(
        [str(binary), "schema"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    emitted_bundle = json.loads(emitted)
    # `validate_schema_bundle` narrows to the part generation consumes, so the version is
    # read from the raw document and the roots from the validated one.
    bundle = generate.validate_schema_bundle(emitted_bundle)

    assert emitted_bundle["version"] == 31
    # Version 31 published projects among the items rendered from a template: what `project
    # create`, `project render` and `project answers` answer with, and the provenance a
    # rendered project records, described as the task and document ones are.
    for verb, root in (
        ("project_create", "QueryResponseOfQualifiedProject"),
        ("project_render", "Regenerated"),
        ("project_answers", "TemplateAnswers"),
    ):
        assert generate.RESPONSE_ROOTS[verb] == root, verb
        assert root in bundle["roots"], root
    assert "project" in json.dumps(emitted_bundle["roots"]["TemplateProvenance"])
    # And a source's own id for one item, which `project create --id` takes, by name.
    assert "NativeId" in bundle["roots"]
    assert "NativeId" in generate.CONTRACT_ROOTS
    from onetaskgraph_sdk import NativeId

    assert NativeId.model_validate("plan:with:colons").root == "plan:with:colons"
    # Version 30 published the one `status_mapping` grammar as a root of its own, and the
    # generated package models it: a name for every kind, `null`, or a name per kind.
    assert "StatusMapping" in bundle["roots"]
    from onetaskgraph_sdk import StatusMapping

    mapping = StatusMapping.model_validate(
        {"todo": "Todo", "draft": None, "done": {"task": "Done", "project": "Completed"}}
    )
    assert mapping.model_dump(mode="json") == {
        "todo": "Todo",
        "draft": None,
        "done": {"task": "Done", "project": "Completed"},
    }
    for refused in ({"done": {}}, {"done": {"task": None}}, {"done": {"epic": "Done"}}):
        try:
            StatusMapping.model_validate(refused)
        except pydantic.ValidationError:
            pass
        else:
            raise AssertionError(f"{refused} was accepted")
    # Version 29 published routing: what `sources route` answers with, and the placement it and
    # every outcome of a routed copy's report name.
    assert generate.RESPONSE_ROOTS["sources_route"] == "SourceRoute"
    for root in ("SourceRoute", "Placement"):
        assert root in bundle["roots"], root
    assert '"placed"' in json.dumps(emitted_bundle["roots"]["CopyOutcome"])
    # Version 28 published a `linear` source's configuration as a root of its own, carrying the
    # `status_mapping` and the `project` the host's Linear work is written with — and the
    # generated package models it.
    linear_config = bundle["roots"]["LinearConfig"]
    assert isinstance(linear_config, dict)
    for member in ("status_mapping", "project"):
        assert member in linear_config["properties"], member
        assert member not in linear_config.get("required", []), member
    from onetaskgraph_sdk import LinearConfig

    configured = LinearConfig.model_validate(
        {
            "team": "ENG",
            "project": "P-1",
            "status_mapping": {"queued": "Queued", "draft": None, "todo": {"project": "Planned"}},
        }
    )
    # Keyed by the status category itself, valued by the shared grammar.
    assert configured.model_dump(mode="json")["status_mapping"] == {
        "queued": "Queued",
        "draft": None,
        "todo": {"project": "Planned"},
    }
    assert configured.project is not None and configured.project.root == "P-1"
    # A key that names no status category is refused, as the binary refuses it.
    try:
        LinearConfig.model_validate({"status_mapping": {"shipped": "Done"}})
    except pydantic.ValidationError:
        pass
    else:
        raise AssertionError("a status_mapping key naming no category was accepted")
    # Version 27 published what `task show-many` answers with: one `TaskDetail` per id.
    task_details = bundle["roots"]["TaskDetails"]
    assert isinstance(task_details, dict)
    assert task_details["required"] == ["details"]
    # Version 26 published the metadata values and the copy origin a task list may be narrowed
    # by: the query's `metadata` and `origin`, the `MetadataMatch` one of the first is, and the
    # two capabilities a source declares them with.
    task_query = bundle["roots"]["TaskQuery"]
    assert isinstance(task_query, dict)
    for member in ("metadata", "origin"):
        assert member in task_query["properties"], member
        assert member not in task_query.get("required", []), member
    listing = json.dumps(bundle["roots"]["SourceListing"])
    assert "filter_by_metadata" in listing
    assert "filter_by_origin" in listing
    # Version 25 published what a copy says about each item it landed beyond its action:
    # which rule found where it landed, and what became of the link it records at
    # `onetaskgraph.copies` — on the three actions that land an item, and not on an orphan.
    for root, words in (
        ("CopyVia", {"link", "origin", "scan", "match"}),
        ("CopyLink", {"recorded", "unchanged", "unrecorded"}),
    ):
        assert root in bundle["roots"], root
        spelled = {entry["const"] for entry in emitted_bundle["roots"][root]["oneOf"]}
        assert spelled == words, root
    copy_action = json.dumps(emitted_bundle["roots"]["CopyAction"])
    assert '"via"' in copy_action and '"link"' in copy_action
    assert '"created"' in copy_action
    # Version 24 published the comment activity a task list may be narrowed by: the query's
    # `commented_since`, the capability a source declares it with, and the plan's predicate.
    task_query = bundle["roots"]["TaskQuery"]
    assert isinstance(task_query, dict)
    assert "commented_since" in task_query["properties"]
    assert "commented_since" not in task_query.get("required", [])
    # Version 23 published what `task update` is given and answers with, the field vocabulary
    # it reports what it wrote in, and the outcome a plugin answers a targeted update with.
    assert generate.RESPONSE_ROOTS["task_update"] == "TaskUpdated"
    for root in ("TaskUpdate", "TaskUpdated", "UpdatedField", "TaskUpdateOutcome"):
        assert root in bundle["roots"], root
    assert "UpdatedField" in generate.CONTRACT_ROOTS
    # Version 22 published what the create, render and answers verbs answer with, and the
    # `onetaskgraph.template` provenance entry a rendered item's metadata holds.
    for verb, root in (
        ("task_create", "TaskDetail"),
        ("task_render", "Regenerated"),
        ("task_answers", "TemplateAnswers"),
        ("document_create", "QueryResponseOfQualifiedDocument"),
        ("document_render", "Regenerated"),
        ("document_answers", "TemplateAnswers"),
    ):
        assert generate.RESPONSE_ROOTS[verb] == root, verb
        assert root in bundle["roots"], root
    assert "TemplateProvenance" in bundle["roots"]
    assert "TemplateProvenance" in generate.CONTRACT_ROOTS
    # Version 21 published what `template variables` and `template render` answer with, and
    # the declaration and two vocabularies inside the first.
    for verb, root in (
        ("template_variables", "TemplateVariables"),
        ("template_render", "RenderedTemplate"),
    ):
        assert generate.RESPONSE_ROOTS[verb] == root, verb
        assert root in bundle["roots"], root
    for root in ("TemplateVariable", "VariableType", "ItemType"):
        assert root in bundle["roots"], root
        assert root in generate.CONTRACT_ROOTS, root
    # Version 20 published a task's `priority` and what `task priority set`, `task content
    # set` and `sources fields` answer with.
    for root in ("QueryResponseOfQualifiedTask", "TaskDetail"):
        assert "priority" in _schema_path(bundle["roots"][root], "$defs", "Task", "properties"), (
            root
        )
    assert "priority" in _generated_task_fields()
    for verb, root in (
        ("task_priority_set", "TaskPrioritySet"),
        ("task_content_set", "TaskContentSet"),
        ("sources_fields", "FieldsReport"),
    ):
        assert generate.RESPONSE_ROOTS[verb] == root, verb
        assert root in bundle["roots"], root
    assert "Priority" in bundle["roots"]
    # Version 19 published a task's `key`. A property is asserted on both sides, because
    # the bundle carrying one the generated model does not is exactly the drift the version
    # exists to make visible.
    for root in ("QueryResponseOfQualifiedTask", "TaskDetail"):
        assert "key" in _schema_path(bundle["roots"][root], "$defs", "Task", "properties"), root
    assert "key" in _generated_task_fields()
    # Version 15 published what the three `metadata set` verbs answer with.
    assert "MetadataSet" in bundle["roots"]
    for verb in ("task_metadata_set", "project_metadata_set", "document_metadata_set"):
        assert generate.RESPONSE_ROOTS[verb] == "MetadataSet", verb
    for root in ("Document", "DocumentQuery", "Location", "PageOfDocument"):
        assert root in bundle["roots"], root
        assert root in generate.CONTRACT_ROOTS, root
    # The two the `document` verb group returns, which is what an SDK generates a model
    # for and what a caller of `document_list` or `document_show` is handed.
    for root in ("QualifiedDocument", "QueryResponseOfQualifiedDocument"):
        assert root in bundle["roots"], root
    assert generate.RESPONSE_ROOTS["document_list"] == "QueryResponseOfQualifiedDocument"
    assert generate.RESPONSE_ROOTS["document_show"] == "QueryResponseOfQualifiedDocument"
    assert generate.RESPONSE_ROOTS["document_copy"] == "CopyReport"


# llmlint: ignore-end[async_typed_clients_at_boundaries]


def _schema_path(schema: JsonValue, *keys: str) -> dict[str, JsonValue]:
    for key in keys:
        assert isinstance(schema, dict), key
        schema = schema[key]
    assert isinstance(schema, dict), keys
    return schema


def _generated_task_fields() -> set[str]:
    from onetaskgraph_sdk._generated.task_detail import Task

    return set(Task.model_fields)
