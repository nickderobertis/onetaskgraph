"""Installed-wheel journey."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


def test_wheel_installs_and_queries_through_public_import(tmp_path: Path) -> None:
    """Install a wheel cleanly and drive a real configured query through it."""
    package = Path(__file__).parents[1]
    workspace = package.parents[1]
    subprocess.run(
        ["cargo", "build", "--quiet", "-p", "onetaskgraph", "--bin", "onetaskgraph"],
        cwd=workspace,
        check=True,
    )
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
    suffix = ".exe" if os.name == "nt" else ""
    binary = (workspace / "target" / "debug" / f"onetaskgraph{suffix}").resolve()
    script = (
        "import asyncio; from onetaskgraph_sdk import Client; "
        f"r=asyncio.run(Client(cwd={str(config)!r}).task_list()); "
        "assert r.items[0].item.title == 'Installed'"
    )
    child_environment = dict(os.environ)
    child_environment["ONETASKGRAPH_SDK_BINARY"] = str(binary)
    subprocess.run([str(python), "-c", script], env=child_environment, check=True)


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


def test_the_generated_package_is_built_from_the_schema_bundle_this_sdk_expects() -> None:
    """The bundle version is what lets an SDK refuse a bundle it was not generated for.

    Version 8 is the one that published the documents contract's four roots, so this
    asserts the version and the roots together — a version bumped without them, or them
    without the bump, is the drift the number exists to make visible.
    """
    import json
    import subprocess
    import sys
    from pathlib import Path

    sys.path.insert(0, str(Path(__file__).parents[1]))
    import generate

    workspace = Path(__file__).parents[3]
    emitted = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "onetaskgraph", "--bin", "onetaskgraph", "--", "schema"],
        cwd=workspace,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    emitted_bundle = json.loads(emitted)
    # `validate_schema_bundle` narrows to the part generation consumes, so the version is
    # read from the raw document and the roots from the validated one.
    bundle = generate.validate_schema_bundle(emitted_bundle)

    assert emitted_bundle["version"] == 8
    for root in ("Document", "DocumentQuery", "Location", "PageOfDocument"):
        assert root in bundle["roots"], root
        assert root in generate.CONTRACT_ROOTS, root
