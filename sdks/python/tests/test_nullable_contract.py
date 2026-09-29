"""Which members an explicit `null` may be decoded for, as the generated models read them.

A member the wire schema gives a default and no `null` type is one the binary may omit and
never writes as `null`: omitted, it takes its default, which is how a document written before
the member existed is read; an explicit `null` is a malformed document, and a model that
accepted one would hand its consumer `None` where the contract promises a value. A member the
schema declares nullable — an `Option` on the Rust side — keeps accepting `null`.
"""

from __future__ import annotations

import asyncio
import json
import shutil
import sys
from pathlib import Path

import pytest
from pydantic import BaseModel, JsonValue, ValidationError

from onetaskgraph_sdk import Client
from onetaskgraph_sdk._generated import (
    query_response_of_qualified_task,
    query_response_of_search_hit,
    source_listing,
    task_detail,
)

sys.path.insert(0, str(Path(__file__).parents[1]))
import generate  # noqa: E402  # The generator lives beside the package, not inside it.

GENERATED = Path(generate.__file__).parent / "src" / "onetaskgraph_sdk" / "_generated"

# Every generated model a task is decoded through, since each root carries its own copy.
TASK_MODELS: list[type[BaseModel]] = [
    query_response_of_qualified_task.Task,
    query_response_of_search_hit.Task,
    task_detail.Task,
]

# A task as a source wrote one before `delivers` and `delivered_by` existed, with its other
# defaulted collections left out as the binary leaves them out when empty.
TASK: dict[str, JsonValue] = {
    "id": "tasks/migrate.md",
    "title": "Migrate the store",
    "content": None,
    "status": {"category": "todo", "name": "Todo"},
    "labels": [],
    "project": None,
    "url": None,
    "created_at": None,
    "updated_at": None,
}

# Each is left out of the wire when empty, so omitting it is what the binary does every day
# rather than only what an old writer did.
TASK_DEFAULTS: dict[str, JsonValue] = {
    "delivers": [],
    "delivered_by": [],
    "metadata": {},
    "repositories": [],
}

# A capability declaration as a plugin's handshake carried one before documents and comments.
CAPABILITIES: dict[str, JsonValue] = {
    "projects": "native",
    "orphan_tasks": "native",
    "filter_by_label": "unsupported",
    "filter_by_status": "native",
    "search_title": "native",
    "search_content": "unsupported",
    "task_dependencies": "forward-only",
    "project_dependencies": "both-directions",
    "max_page_size": 25,
}

CAPABILITY_MEMBERS = ("documents", "comments")


def test_an_omitted_documents_or_comments_capability_reads_as_unsupported() -> None:
    """A handshake that predates either capability reads it as the source not holding any."""
    capabilities = source_listing.Capabilities.model_validate(CAPABILITIES)
    for member in CAPABILITY_MEMBERS:
        assert getattr(capabilities, member) == source_listing.Support.SupportUnsupported, member


@pytest.mark.parametrize("member", CAPABILITY_MEMBERS)
def test_an_explicit_null_documents_or_comments_capability_is_refused(member: str) -> None:
    """An explicit `null` is no declaration a plugin makes, so it is refused, naming it."""
    with pytest.raises(ValidationError) as refused:
        source_listing.Capabilities.model_validate({**CAPABILITIES, member: None})
    assert refused.value.errors()[0]["loc"] == (member,)


@pytest.mark.parametrize("member", CAPABILITY_MEMBERS)
@pytest.mark.parametrize("support", ["native", "unsupported"])
def test_a_documents_or_comments_capability_round_trips(member: str, support: str) -> None:
    """Both declarations are written back out and read in again as themselves."""
    capabilities = source_listing.Capabilities.model_validate({**CAPABILITIES, member: support})
    dumped = capabilities.model_dump(mode="json")
    assert dumped[member] == support
    assert source_listing.Capabilities.model_validate(dumped) == capabilities


@pytest.mark.parametrize("model", TASK_MODELS)
def test_an_omitted_defaulted_task_member_reads_as_its_default(model: type[BaseModel]) -> None:
    """A task that leaves out its empty collections reads each as empty, in every task model."""
    dumped = model.model_validate(TASK).model_dump(mode="json")
    for member, default in TASK_DEFAULTS.items():
        assert dumped[member] == default, member


@pytest.mark.parametrize("model", TASK_MODELS)
@pytest.mark.parametrize("member", sorted(TASK_DEFAULTS))
def test_an_explicit_null_defaulted_task_member_is_refused(
    model: type[BaseModel], member: str
) -> None:
    """An explicit `null` for a defaulted collection is refused, naming the member."""
    with pytest.raises(ValidationError) as refused:
        model.model_validate({**TASK, member: None})
    assert refused.value.errors()[0]["loc"] == (member,)


@pytest.mark.parametrize("model", TASK_MODELS)
def test_a_defaulted_task_member_round_trips(model: type[BaseModel]) -> None:
    """A task holding a value in each defaulted member writes it out and reads it back."""
    task = model.model_validate(
        {
            **TASK,
            "delivers": ["work:T-2"],
            "delivered_by": ["work:T-3"],
            "metadata": {"team.estimate": 3},
            "repositories": ["github.com/acme/store"],
        }
    )
    dumped = task.model_dump(mode="json")
    assert dumped["delivers"] == ["work:T-2"]
    assert dumped["metadata"] == {"team.estimate": 3}
    assert model.model_validate(dumped) == task


@pytest.mark.parametrize("model", TASK_MODELS)
def test_a_nullable_task_member_still_accepts_null(model: type[BaseModel]) -> None:
    """A member the schema declares nullable is not narrowed with the defaulted ones."""
    dumped = model.model_validate({**TASK, "key": None, "content": None}).model_dump()
    assert (dumped["key"], dumped["content"]) == (None, None)


def emitted_bundle() -> generate.SchemaBundle:
    """The schema bundle the real binary emits, validated as the generator validates it."""
    return generate.validate_schema_bundle(json.loads(generate.run_workspace_binary("schema")))


def test_no_generated_member_accepts_null_that_the_schema_declares_non_nullable() -> None:
    """Over the whole generated package, every model agrees with the schema about `null`.

    This is the decode test for every member the tests above do not name — the document,
    project, delivery, copy-report and update models among them — rather than a structural
    stand-in for one: the guard decodes an explicit `None` through every field of every
    generated model, which is the `model_validate` a consumer's answer takes, and compares
    what each accepts with what the schema the real binary emits declares. One decode test
    per member would restate that rule a few dozen times and miss the next member added.

    And the pairing it rests on is not vacuous: the objects this test is about were found on
    both sides, so an agreement is not two empty sides agreeing.
    """
    bundle = emitted_bundle()
    assert generate.nullability_disagreements(bundle, GENERATED) == []
    capabilities = next(
        key
        for key in generate.model_nullability(GENERATED / "source_listing.py")
        if {"documents", "comments"} <= key
    )
    assert generate.schema_nullability(bundle["roots"]["SourceListing"])[capabilities] == {
        frozenset()
    }


def test_the_guard_refuses_a_generated_member_the_schema_does_not_let_be_null(
    tmp_path: Path,
) -> None:
    """A model widened to `Support | None` fails generation, naming its root and members."""
    package = tmp_path / "_generated"
    shutil.copytree(GENERATED, package)
    module = package / "source_listing.py"
    text = module.read_text(encoding="utf-8")
    widened = text.replace(
        "    documents: Annotated[\n        Support,",
        "    documents: Annotated[\n        Support | None,",
    )
    assert widened != text, "the documents capability is no longer generated as it was"
    module.write_text(widened, encoding="utf-8")

    disagreements = generate.nullability_disagreements(emitted_bundle(), package)
    assert len(disagreements) == 1
    assert disagreements[0].startswith("SourceListing object {comments, documents,")
    assert "the models for [['documents']]" in disagreements[0]


def test_the_guard_refuses_a_schema_member_the_models_do_not_let_be_null() -> None:
    """A schema that admits `null` where the models refuse it disagrees just as loudly."""
    emitted = json.loads(generate.run_workspace_binary("schema"))
    capabilities = emitted["roots"]["SourceListing"]["$defs"]["Capabilities"]["properties"]
    capabilities["comments"] = {"anyOf": [capabilities["comments"], {"type": "null"}]}

    bundle = generate.validate_schema_bundle(emitted)
    disagreements = generate.nullability_disagreements(bundle, GENERATED)
    assert len(disagreements) == 1
    assert "the schema admits null for [['comments']], the models for [[]]" in disagreements[0]


def test_the_guard_refuses_a_schema_reference_it_cannot_follow() -> None:
    """A member referring to a definition its root lacks is refused, not read as admitting null."""
    emitted = json.loads(generate.run_workspace_binary("schema"))
    capabilities = emitted["roots"]["SourceListing"]["$defs"]["Capabilities"]["properties"]
    capabilities["comments"] = {"$ref": "#/$defs/Vanished"}

    bundle = generate.validate_schema_bundle(emitted)
    with pytest.raises(SystemExit) as refused:
        generate.nullability_disagreements(bundle, GENERATED)
    assert "'#/$defs/Vanished' that is not a definition of its own root" in str(refused.value)


def test_the_guard_refuses_a_combinator_that_is_not_a_list() -> None:
    """A `oneOf` the guard cannot read as variants is refused rather than iterated as a map."""
    emitted = json.loads(generate.run_workspace_binary("schema"))
    capabilities = emitted["roots"]["SourceListing"]["$defs"]["Capabilities"]["properties"]
    capabilities["comments"] = {"oneOf": {"type": "null"}}

    bundle = generate.validate_schema_bundle(emitted)
    with pytest.raises(SystemExit) as refused:
        generate.nullability_disagreements(bundle, GENERATED)
    assert "schema whose `oneOf` is not a list" in str(refused.value)


def test_generation_fails_when_its_models_stop_following_the_schema(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """`generate()` refuses models that stop following the schema, before it compares anything.

    The generator runs against the real binary's schema with its code generation step alone
    stood in for: that step stages the committed models with `documents` and `comments`
    widened to `Support | None`, which is what the code generator writes for them without its
    strict nullable option. Everything after it — the client, the formatter, the guard and the
    `--check` comparison — is the generator's own, and the guard refuses before the comparison
    could report the tree merely stale.
    """
    bundle = emitted_bundle()

    def widened_models(_bundle: generate.SchemaBundle, destination: Path) -> None:
        shutil.copytree(GENERATED, destination, dirs_exist_ok=True)
        module = destination / "source_listing.py"
        text = module.read_text(encoding="utf-8")
        widened = text
        for member in CAPABILITY_MEMBERS:
            widened = widened.replace(
                f"    {member}: Annotated[\n        Support,",
                f"    {member}: Annotated[\n        Support | None,",
            )
        assert widened.count("Support | None,") == 2, "the capabilities moved in the models"
        module.write_text(widened, encoding="utf-8")

    monkeypatch.setattr(generate, "generate_models", widened_models)
    with pytest.raises(SystemExit) as refused:
        generate.generate(bundle, check=True)
    message = str(refused.value)
    assert "disagree about which members accept `null`" in message, message
    line = next(
        line
        for line in message.splitlines()
        if line.strip().startswith("SourceListing object {comments, documents,")
    )
    assert "the schema admits null for [[]]" in line
    assert "the models for [['comments', 'documents']]" in line
    assert "generated Python SDK is stale" not in message


def test_the_real_binary_writes_members_that_decode_and_refuses_null(
    binary: Path, tmp_path: Path
) -> None:
    """A real CLI answer decodes through the SDK, and the same answer with `null` does not."""
    tasks = tmp_path / "work" / "tasks"
    tasks.mkdir(parents=True)
    (tasks / "T-1.md").write_text("---\ntitle: Plain\nstatus: todo\n---\nBody.\n")
    (tmp_path / "onetaskgraph.yaml").write_text(
        '{"sources": {"work": {"plugin": "local-md", "config": {"root": "work"}}}}'
    )
    client = Client(binary, cwd=tmp_path)

    shown = asyncio.run(client.task_show(id="work:T-1")).items[0].item
    assert (shown.delivers, shown.delivered_by, shown.repositories) == ([], [], [])
    available = asyncio.run(client.sources_list())[0].root
    assert isinstance(available, source_listing.SourceListingAvailable)
    capabilities = available.capabilities
    assert (capabilities.documents, capabilities.comments) == ("native", "native")

    written = capabilities.model_dump(mode="json")
    for member in CAPABILITY_MEMBERS:
        with pytest.raises(ValidationError) as refused:
            source_listing.Capabilities.model_validate({**written, member: None})
        assert refused.value.errors()[0]["loc"] == (member,)
    task = shown.model_dump(mode="json", by_alias=True)
    with pytest.raises(ValidationError) as refused:
        task_detail.Task.model_validate({**task, "delivers": None})
    assert refused.value.errors()[0]["loc"] == ("delivers",)
