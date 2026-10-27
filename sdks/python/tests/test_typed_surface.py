"""Every exported name type-checks as its real class, and every Client verb as its answer.

This module is itself the consumer: it imports each name `onetaskgraph_sdk` and
`onetaskgraph_sdk._generated.models` export, by name, and calls Client methods annotated with
the types they answer with. The `typecheck` target reads it with `ty`, and the last test
reads it with `mypy --strict`, the setting a consumer like ai-orchestrator's runs nearest to,
so a name a checker could no longer resolve to its class fails both. The first tests hold
the two import lists to each module's `__all__`, so a new export cannot be left out of them.
"""

from __future__ import annotations

import ast
import asyncio
import json
import subprocess
import sys
from pathlib import Path
from typing import assert_type

import pytest
from pydantic import BaseModel

import onetaskgraph_sdk
from onetaskgraph_sdk import (
    CacheCleared,
    CacheUse,
    Classification,
    Client,
    CommentList,
    CommentWritten,
    CopyReport,
    DeletedComment,
    Delivered,
    DeliveryOutcome,
    Document,
    DocumentDetail,
    DocumentQuery,
    EffectiveConfig,
    ExpectedWrite,
    FailureDocument,
    FieldsReport,
    GlobalId,
    IssuesCompleted,
    ItemType,
    LinearConfig,
    Location,
    MetadataSet,
    NativeId,
    OnetaskgraphError,
    PageOfDocument,
    Priority,
    ProjectGraph,
    ProjectStatusSet,
    PublicProjections,
    PublicWriteInput,
    QueryPlan,
    QueryResponseOfQualifiedDocument,
    QueryResponseOfQualifiedEdge,
    QueryResponseOfQualifiedLabel,
    QueryResponseOfQualifiedProject,
    QueryResponseOfQualifiedTask,
    QueryResponseOfSearchHit,
    Regenerated,
    RenderedTemplate,
    RepositoryVisibility,
    SourceCacheConfig,
    SourceFailure,
    SourceListing,
    SourceName,
    SourceRoute,
    SourceVisibility,
    StatusCategory,
    StatusesReconciled,
    StatusMapping,
    StatusNamesReport,
    StatusOptionsReport,
    TaskAssetNames,
    TaskContentSet,
    TaskDetail,
    TaskDetails,
    TaskPrioritySet,
    TaskRef,
    TaskStatusSet,
    TaskUpdated,
    TemplateAnswers,
    TemplateProvenance,
    TemplateVariable,
    TemplateVariables,
    UpdatedField,
    VariableType,
    WritePolicyConfig,
    WriteVerdict,
    client,
)
from onetaskgraph_sdk._generated import models
from onetaskgraph_sdk._generated.models import (
    CacheCleared as ModelsCacheCleared,
)
from onetaskgraph_sdk._generated.models import (
    CacheUse as ModelsCacheUse,
)
from onetaskgraph_sdk._generated.models import (
    Classification as ModelsClassification,
)
from onetaskgraph_sdk._generated.models import (
    CommentList as ModelsCommentList,
)
from onetaskgraph_sdk._generated.models import (
    CommentWritten as ModelsCommentWritten,
)
from onetaskgraph_sdk._generated.models import (
    CopyReport as ModelsCopyReport,
)
from onetaskgraph_sdk._generated.models import (
    DeletedComment as ModelsDeletedComment,
)
from onetaskgraph_sdk._generated.models import (
    Delivered as ModelsDelivered,
)
from onetaskgraph_sdk._generated.models import (
    DeliveryOutcome as ModelsDeliveryOutcome,
)
from onetaskgraph_sdk._generated.models import (
    Document as ModelsDocument,
)
from onetaskgraph_sdk._generated.models import (
    DocumentDetail as ModelsDocumentDetail,
)
from onetaskgraph_sdk._generated.models import (
    DocumentQuery as ModelsDocumentQuery,
)
from onetaskgraph_sdk._generated.models import (
    EffectiveConfig as ModelsEffectiveConfig,
)
from onetaskgraph_sdk._generated.models import (
    ExpectedWrite as ModelsExpectedWrite,
)
from onetaskgraph_sdk._generated.models import (
    FailureDocument as ModelsFailureDocument,
)
from onetaskgraph_sdk._generated.models import (
    FieldsReport as ModelsFieldsReport,
)
from onetaskgraph_sdk._generated.models import (
    GlobalId as ModelsGlobalId,
)
from onetaskgraph_sdk._generated.models import (
    IssuesCompleted as ModelsIssuesCompleted,
)
from onetaskgraph_sdk._generated.models import (
    ItemType as ModelsItemType,
)
from onetaskgraph_sdk._generated.models import (
    LinearConfig as ModelsLinearConfig,
)
from onetaskgraph_sdk._generated.models import (
    Location as ModelsLocation,
)
from onetaskgraph_sdk._generated.models import (
    MetadataSet as ModelsMetadataSet,
)
from onetaskgraph_sdk._generated.models import (
    NativeId as ModelsNativeId,
)
from onetaskgraph_sdk._generated.models import (
    PageOfDocument as ModelsPageOfDocument,
)
from onetaskgraph_sdk._generated.models import (
    Priority as ModelsPriority,
)
from onetaskgraph_sdk._generated.models import (
    ProjectGraph as ModelsProjectGraph,
)
from onetaskgraph_sdk._generated.models import (
    ProjectStatusSet as ModelsProjectStatusSet,
)
from onetaskgraph_sdk._generated.models import (
    PublicProjections as ModelsPublicProjections,
)
from onetaskgraph_sdk._generated.models import (
    PublicWriteInput as ModelsPublicWriteInput,
)
from onetaskgraph_sdk._generated.models import (
    QueryPlan as ModelsQueryPlan,
)
from onetaskgraph_sdk._generated.models import (
    QueryResponseOfQualifiedDocument as ModelsQueryResponseOfQualifiedDocument,
)
from onetaskgraph_sdk._generated.models import (
    QueryResponseOfQualifiedEdge as ModelsQueryResponseOfQualifiedEdge,
)
from onetaskgraph_sdk._generated.models import (
    QueryResponseOfQualifiedLabel as ModelsQueryResponseOfQualifiedLabel,
)
from onetaskgraph_sdk._generated.models import (
    QueryResponseOfQualifiedProject as ModelsQueryResponseOfQualifiedProject,
)
from onetaskgraph_sdk._generated.models import (
    QueryResponseOfQualifiedTask as ModelsQueryResponseOfQualifiedTask,
)
from onetaskgraph_sdk._generated.models import (
    QueryResponseOfSearchHit as ModelsQueryResponseOfSearchHit,
)
from onetaskgraph_sdk._generated.models import (
    Regenerated as ModelsRegenerated,
)
from onetaskgraph_sdk._generated.models import (
    RenderedTemplate as ModelsRenderedTemplate,
)
from onetaskgraph_sdk._generated.models import (
    RepositoryVisibility as ModelsRepositoryVisibility,
)
from onetaskgraph_sdk._generated.models import (
    SourceCacheConfig as ModelsSourceCacheConfig,
)
from onetaskgraph_sdk._generated.models import (
    SourceFailure as ModelsSourceFailure,
)
from onetaskgraph_sdk._generated.models import (
    SourceListing as ModelsSourceListing,
)
from onetaskgraph_sdk._generated.models import (
    SourceName as ModelsSourceName,
)
from onetaskgraph_sdk._generated.models import (
    SourceRoute as ModelsSourceRoute,
)
from onetaskgraph_sdk._generated.models import (
    SourceVisibility as ModelsSourceVisibility,
)
from onetaskgraph_sdk._generated.models import (
    StatusCategory as ModelsStatusCategory,
)
from onetaskgraph_sdk._generated.models import (
    StatusesReconciled as ModelsStatusesReconciled,
)
from onetaskgraph_sdk._generated.models import (
    StatusMapping as ModelsStatusMapping,
)
from onetaskgraph_sdk._generated.models import (
    StatusNamesReport as ModelsStatusNamesReport,
)
from onetaskgraph_sdk._generated.models import (
    StatusOptionsReport as ModelsStatusOptionsReport,
)
from onetaskgraph_sdk._generated.models import (
    TaskAssetNames as ModelsTaskAssetNames,
)
from onetaskgraph_sdk._generated.models import (
    TaskContentSet as ModelsTaskContentSet,
)
from onetaskgraph_sdk._generated.models import (
    TaskDetail as ModelsTaskDetail,
)
from onetaskgraph_sdk._generated.models import (
    TaskDetails as ModelsTaskDetails,
)
from onetaskgraph_sdk._generated.models import (
    TaskPrioritySet as ModelsTaskPrioritySet,
)
from onetaskgraph_sdk._generated.models import (
    TaskRef as ModelsTaskRef,
)
from onetaskgraph_sdk._generated.models import (
    TaskStatusSet as ModelsTaskStatusSet,
)
from onetaskgraph_sdk._generated.models import (
    TaskUpdated as ModelsTaskUpdated,
)
from onetaskgraph_sdk._generated.models import (
    TemplateAnswers as ModelsTemplateAnswers,
)
from onetaskgraph_sdk._generated.models import (
    TemplateProvenance as ModelsTemplateProvenance,
)
from onetaskgraph_sdk._generated.models import (
    TemplateVariable as ModelsTemplateVariable,
)
from onetaskgraph_sdk._generated.models import (
    TemplateVariables as ModelsTemplateVariables,
)
from onetaskgraph_sdk._generated.models import (
    UpdatedField as ModelsUpdatedField,
)
from onetaskgraph_sdk._generated.models import (
    VariableType as ModelsVariableType,
)
from onetaskgraph_sdk._generated.models import (
    WritePolicyConfig as ModelsWritePolicyConfig,
)
from onetaskgraph_sdk._generated.models import (
    WriteVerdict as ModelsWriteVerdict,
)

SCRIPTS = Path(sys.executable).parent
SUFFIX = ".exe" if sys.platform == "win32" else ""


def imported_from(module: str) -> list[str]:
    """The names this file imports from `module`, as the checkers read them."""
    tree = ast.parse(Path(__file__).read_text(encoding="utf-8"))
    return sorted(
        alias.name
        for node in tree.body
        if isinstance(node, ast.ImportFrom) and node.module == module
        for alias in node.names
    )


@pytest.mark.parametrize("module", [onetaskgraph_sdk, models])
def test_this_file_imports_every_name_the_module_exports(module: object) -> None:
    """The import lists above are each module's whole `__all__`, so none can be missed."""
    name = getattr(module, "__name__", "")
    assert imported_from(name) == sorted(getattr(module, "__all__", [])), (
        f"import every name {name}.__all__ lists above, and nothing it does not list"
    )


FROM_ROOT: dict[str, object] = {
    "client": client,
    "CacheCleared": CacheCleared,
    "CacheUse": CacheUse,
    "Classification": Classification,
    "Client": Client,
    "CommentList": CommentList,
    "CommentWritten": CommentWritten,
    "CopyReport": CopyReport,
    "DeletedComment": DeletedComment,
    "Delivered": Delivered,
    "DeliveryOutcome": DeliveryOutcome,
    "Document": Document,
    "DocumentDetail": DocumentDetail,
    "DocumentQuery": DocumentQuery,
    "EffectiveConfig": EffectiveConfig,
    "ExpectedWrite": ExpectedWrite,
    "FailureDocument": FailureDocument,
    "FieldsReport": FieldsReport,
    "GlobalId": GlobalId,
    "IssuesCompleted": IssuesCompleted,
    "ItemType": ItemType,
    "LinearConfig": LinearConfig,
    "Location": Location,
    "MetadataSet": MetadataSet,
    "NativeId": NativeId,
    "OnetaskgraphError": OnetaskgraphError,
    "PageOfDocument": PageOfDocument,
    "Priority": Priority,
    "ProjectGraph": ProjectGraph,
    "ProjectStatusSet": ProjectStatusSet,
    "PublicProjections": PublicProjections,
    "PublicWriteInput": PublicWriteInput,
    "QueryPlan": QueryPlan,
    "QueryResponseOfQualifiedDocument": QueryResponseOfQualifiedDocument,
    "QueryResponseOfQualifiedEdge": QueryResponseOfQualifiedEdge,
    "QueryResponseOfQualifiedLabel": QueryResponseOfQualifiedLabel,
    "QueryResponseOfQualifiedProject": QueryResponseOfQualifiedProject,
    "QueryResponseOfQualifiedTask": QueryResponseOfQualifiedTask,
    "QueryResponseOfSearchHit": QueryResponseOfSearchHit,
    "Regenerated": Regenerated,
    "RenderedTemplate": RenderedTemplate,
    "RepositoryVisibility": RepositoryVisibility,
    "SourceCacheConfig": SourceCacheConfig,
    "SourceFailure": SourceFailure,
    "SourceListing": SourceListing,
    "SourceName": SourceName,
    "SourceRoute": SourceRoute,
    "SourceVisibility": SourceVisibility,
    "StatusCategory": StatusCategory,
    "StatusMapping": StatusMapping,
    "StatusNamesReport": StatusNamesReport,
    "StatusOptionsReport": StatusOptionsReport,
    "StatusesReconciled": StatusesReconciled,
    "TaskAssetNames": TaskAssetNames,
    "TaskContentSet": TaskContentSet,
    "TaskDetail": TaskDetail,
    "TaskDetails": TaskDetails,
    "TaskPrioritySet": TaskPrioritySet,
    "TaskRef": TaskRef,
    "TaskStatusSet": TaskStatusSet,
    "TaskUpdated": TaskUpdated,
    "TemplateAnswers": TemplateAnswers,
    "TemplateProvenance": TemplateProvenance,
    "TemplateVariable": TemplateVariable,
    "TemplateVariables": TemplateVariables,
    "UpdatedField": UpdatedField,
    "VariableType": VariableType,
    "WritePolicyConfig": WritePolicyConfig,
    "WriteVerdict": WriteVerdict,
}
"""Every name imported from the package root, by the name it was imported as."""

FROM_MODELS: dict[str, object] = {
    "CacheCleared": ModelsCacheCleared,
    "CacheUse": ModelsCacheUse,
    "Classification": ModelsClassification,
    "CommentList": ModelsCommentList,
    "CommentWritten": ModelsCommentWritten,
    "CopyReport": ModelsCopyReport,
    "DeletedComment": ModelsDeletedComment,
    "Delivered": ModelsDelivered,
    "DeliveryOutcome": ModelsDeliveryOutcome,
    "Document": ModelsDocument,
    "DocumentDetail": ModelsDocumentDetail,
    "DocumentQuery": ModelsDocumentQuery,
    "EffectiveConfig": ModelsEffectiveConfig,
    "ExpectedWrite": ModelsExpectedWrite,
    "FailureDocument": ModelsFailureDocument,
    "FieldsReport": ModelsFieldsReport,
    "GlobalId": ModelsGlobalId,
    "IssuesCompleted": ModelsIssuesCompleted,
    "ItemType": ModelsItemType,
    "LinearConfig": ModelsLinearConfig,
    "Location": ModelsLocation,
    "MetadataSet": ModelsMetadataSet,
    "NativeId": ModelsNativeId,
    "PageOfDocument": ModelsPageOfDocument,
    "Priority": ModelsPriority,
    "ProjectGraph": ModelsProjectGraph,
    "ProjectStatusSet": ModelsProjectStatusSet,
    "PublicProjections": ModelsPublicProjections,
    "PublicWriteInput": ModelsPublicWriteInput,
    "QueryPlan": ModelsQueryPlan,
    "QueryResponseOfQualifiedDocument": ModelsQueryResponseOfQualifiedDocument,
    "QueryResponseOfQualifiedEdge": ModelsQueryResponseOfQualifiedEdge,
    "QueryResponseOfQualifiedLabel": ModelsQueryResponseOfQualifiedLabel,
    "QueryResponseOfQualifiedProject": ModelsQueryResponseOfQualifiedProject,
    "QueryResponseOfQualifiedTask": ModelsQueryResponseOfQualifiedTask,
    "QueryResponseOfSearchHit": ModelsQueryResponseOfSearchHit,
    "Regenerated": ModelsRegenerated,
    "RenderedTemplate": ModelsRenderedTemplate,
    "RepositoryVisibility": ModelsRepositoryVisibility,
    "SourceCacheConfig": ModelsSourceCacheConfig,
    "SourceFailure": ModelsSourceFailure,
    "SourceListing": ModelsSourceListing,
    "SourceName": ModelsSourceName,
    "SourceRoute": ModelsSourceRoute,
    "SourceVisibility": ModelsSourceVisibility,
    "StatusCategory": ModelsStatusCategory,
    "StatusMapping": ModelsStatusMapping,
    "StatusNamesReport": ModelsStatusNamesReport,
    "StatusOptionsReport": ModelsStatusOptionsReport,
    "StatusesReconciled": ModelsStatusesReconciled,
    "TaskAssetNames": ModelsTaskAssetNames,
    "TaskContentSet": ModelsTaskContentSet,
    "TaskDetail": ModelsTaskDetail,
    "TaskDetails": ModelsTaskDetails,
    "TaskPrioritySet": ModelsTaskPrioritySet,
    "TaskRef": ModelsTaskRef,
    "TaskStatusSet": ModelsTaskStatusSet,
    "TaskUpdated": ModelsTaskUpdated,
    "TemplateAnswers": ModelsTemplateAnswers,
    "TemplateProvenance": ModelsTemplateProvenance,
    "TemplateVariable": ModelsTemplateVariable,
    "TemplateVariables": ModelsTemplateVariables,
    "UpdatedField": ModelsUpdatedField,
    "VariableType": ModelsVariableType,
    "WritePolicyConfig": ModelsWritePolicyConfig,
    "WriteVerdict": ModelsWriteVerdict,
}
"""Every name imported from `models`, by the name it was imported as."""


def test_every_spelling_of_an_export_is_one_object() -> None:
    """A name imported, read as an attribute, and read through `models` is the same object."""
    assert sorted(FROM_ROOT) == imported_from("onetaskgraph_sdk")
    assert sorted(FROM_MODELS) == imported_from("onetaskgraph_sdk._generated.models")
    for name, value in FROM_ROOT.items():
        assert getattr(onetaskgraph_sdk, name) is value, name
    assert client is sys.modules["onetaskgraph_sdk.client"]
    for name, value in FROM_MODELS.items():
        assert getattr(models, name) is value is FROM_ROOT[name], name


def plan(tmp_path: Path) -> Path:
    """Configure a Markdown folder holding one task and an empty one to copy into."""
    tasks = tmp_path / "plan" / "tasks"
    tasks.mkdir(parents=True)
    (tasks / "parse.md").write_text(
        "---\ntitle: Parse the feed\nstatus: todo\n---\nRead every row.\n", encoding="utf-8"
    )
    (tmp_path / "into").mkdir()
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps(
            {
                "sources": {
                    name: {"plugin": "local-md", "config": {"root": str(tmp_path / name)}}
                    for name in ("plan", "into")
                }
            }
        ),
        encoding="utf-8",
    )
    return tmp_path


def test_client_methods_answer_with_the_types_they_declare(binary: Path, tmp_path: Path) -> None:
    """Each verb's answer is, to a checker and at run time, the class its signature names."""
    client = Client(binary, cwd=plan(tmp_path))

    effective = asyncio.run(client.config_show())
    assert_type(effective, EffectiveConfig)
    assert isinstance(effective, EffectiveConfig)

    sources = asyncio.run(client.sources_list())
    assert_type(sources, list[SourceListing])
    # Containment: a host's own user configuration may name sources of its own beside these.
    assert {"into", "plan"} <= {source.root.source.root for source in sources}
    assert all(isinstance(source, SourceListing) for source in sources)

    listed = asyncio.run(client.task_list(source=["plan"]))
    assert_type(listed, QueryResponseOfQualifiedTask)
    assert isinstance(listed, QueryResponseOfQualifiedTask)
    assert [task.id.root for task in listed.items] == ["plan:parse"]

    shown = asyncio.run(client.task_show(id=GlobalId(root="plan:parse")))
    assert_type(shown, TaskDetail)
    assert isinstance(shown, TaskDetail)
    assert shown.items[0].item.title == "Parse the feed"

    copied = asyncio.run(client.task_copy(ids=["plan:parse"], to="into"))
    assert_type(copied, CopyReport)
    assert isinstance(copied, CopyReport)
    assert [item.root.action for item in copied.items] == ["created"]

    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(client.task_show(id="nowhere:parse"))
    assert refused.value.exit_code != 0


MODEL_CLASSES: dict[str, type[BaseModel]] = {
    "CacheCleared": CacheCleared,
    "CacheUse": CacheUse,
    "CommentList": CommentList,
    "CommentWritten": CommentWritten,
    "CopyReport": CopyReport,
    "DeletedComment": DeletedComment,
    "Delivered": Delivered,
    "DeliveryOutcome": DeliveryOutcome,
    "Document": Document,
    "DocumentDetail": DocumentDetail,
    "DocumentQuery": DocumentQuery,
    "EffectiveConfig": EffectiveConfig,
    "ExpectedWrite": ExpectedWrite,
    "FailureDocument": FailureDocument,
    "FieldsReport": FieldsReport,
    "GlobalId": GlobalId,
    "IssuesCompleted": IssuesCompleted,
    "LinearConfig": LinearConfig,
    "Location": Location,
    "MetadataSet": MetadataSet,
    "NativeId": NativeId,
    "PageOfDocument": PageOfDocument,
    "ProjectGraph": ProjectGraph,
    "ProjectStatusSet": ProjectStatusSet,
    "PublicProjections": PublicProjections,
    "PublicWriteInput": PublicWriteInput,
    "QueryPlan": QueryPlan,
    "QueryResponseOfQualifiedDocument": QueryResponseOfQualifiedDocument,
    "QueryResponseOfQualifiedEdge": QueryResponseOfQualifiedEdge,
    "QueryResponseOfQualifiedLabel": QueryResponseOfQualifiedLabel,
    "QueryResponseOfQualifiedProject": QueryResponseOfQualifiedProject,
    "QueryResponseOfQualifiedTask": QueryResponseOfQualifiedTask,
    "QueryResponseOfSearchHit": QueryResponseOfSearchHit,
    "Regenerated": Regenerated,
    "RenderedTemplate": RenderedTemplate,
    "SourceCacheConfig": SourceCacheConfig,
    "SourceFailure": SourceFailure,
    "SourceListing": SourceListing,
    "SourceName": SourceName,
    "SourceRoute": SourceRoute,
    "StatusMapping": StatusMapping,
    "StatusNamesReport": StatusNamesReport,
    "StatusOptionsReport": StatusOptionsReport,
    "StatusesReconciled": StatusesReconciled,
    "TaskAssetNames": TaskAssetNames,
    "TaskContentSet": TaskContentSet,
    "TaskDetail": TaskDetail,
    "TaskDetails": TaskDetails,
    "TaskPrioritySet": TaskPrioritySet,
    "TaskRef": TaskRef,
    "TaskStatusSet": TaskStatusSet,
    "TaskUpdated": TaskUpdated,
    "TemplateAnswers": TemplateAnswers,
    "TemplateProvenance": TemplateProvenance,
    "TemplateVariable": TemplateVariable,
    "TemplateVariables": TemplateVariables,
    "WritePolicyConfig": WritePolicyConfig,
    "WriteVerdict": WriteVerdict,
}
"""Every export that is a pydantic model, read from the package root as a model class."""

MODELS_MODEL_CLASSES: dict[str, type[BaseModel]] = {
    "CacheCleared": ModelsCacheCleared,
    "CacheUse": ModelsCacheUse,
    "CommentList": ModelsCommentList,
    "CommentWritten": ModelsCommentWritten,
    "CopyReport": ModelsCopyReport,
    "DeletedComment": ModelsDeletedComment,
    "Delivered": ModelsDelivered,
    "DeliveryOutcome": ModelsDeliveryOutcome,
    "Document": ModelsDocument,
    "DocumentDetail": ModelsDocumentDetail,
    "DocumentQuery": ModelsDocumentQuery,
    "EffectiveConfig": ModelsEffectiveConfig,
    "ExpectedWrite": ModelsExpectedWrite,
    "FailureDocument": ModelsFailureDocument,
    "FieldsReport": ModelsFieldsReport,
    "GlobalId": ModelsGlobalId,
    "IssuesCompleted": ModelsIssuesCompleted,
    "LinearConfig": ModelsLinearConfig,
    "Location": ModelsLocation,
    "MetadataSet": ModelsMetadataSet,
    "NativeId": ModelsNativeId,
    "PageOfDocument": ModelsPageOfDocument,
    "ProjectGraph": ModelsProjectGraph,
    "ProjectStatusSet": ModelsProjectStatusSet,
    "PublicProjections": ModelsPublicProjections,
    "PublicWriteInput": ModelsPublicWriteInput,
    "QueryPlan": ModelsQueryPlan,
    "QueryResponseOfQualifiedDocument": ModelsQueryResponseOfQualifiedDocument,
    "QueryResponseOfQualifiedEdge": ModelsQueryResponseOfQualifiedEdge,
    "QueryResponseOfQualifiedLabel": ModelsQueryResponseOfQualifiedLabel,
    "QueryResponseOfQualifiedProject": ModelsQueryResponseOfQualifiedProject,
    "QueryResponseOfQualifiedTask": ModelsQueryResponseOfQualifiedTask,
    "QueryResponseOfSearchHit": ModelsQueryResponseOfSearchHit,
    "Regenerated": ModelsRegenerated,
    "RenderedTemplate": ModelsRenderedTemplate,
    "SourceCacheConfig": ModelsSourceCacheConfig,
    "SourceFailure": ModelsSourceFailure,
    "SourceListing": ModelsSourceListing,
    "SourceName": ModelsSourceName,
    "SourceRoute": ModelsSourceRoute,
    "StatusMapping": ModelsStatusMapping,
    "StatusNamesReport": ModelsStatusNamesReport,
    "StatusOptionsReport": ModelsStatusOptionsReport,
    "StatusesReconciled": ModelsStatusesReconciled,
    "TaskAssetNames": ModelsTaskAssetNames,
    "TaskContentSet": ModelsTaskContentSet,
    "TaskDetail": ModelsTaskDetail,
    "TaskDetails": ModelsTaskDetails,
    "TaskPrioritySet": ModelsTaskPrioritySet,
    "TaskRef": ModelsTaskRef,
    "TaskStatusSet": ModelsTaskStatusSet,
    "TaskUpdated": ModelsTaskUpdated,
    "TemplateAnswers": ModelsTemplateAnswers,
    "TemplateProvenance": ModelsTemplateProvenance,
    "TemplateVariable": ModelsTemplateVariable,
    "TemplateVariables": ModelsTemplateVariables,
    "WritePolicyConfig": ModelsWritePolicyConfig,
    "WriteVerdict": ModelsWriteVerdict,
}
"""The same, read from `models`."""


def test_every_model_export_is_a_model_class_to_a_checker() -> None:
    """Both tables above name every export that is a model, and each is one to a checker.

    The annotation is what the checkers read: an export a checker could not resolve to a
    pydantic model class would fail `type[BaseModel]` in either table.
    """
    for module, table in ((onetaskgraph_sdk, MODEL_CLASSES), (models, MODELS_MODEL_CLASSES)):
        exported = getattr(module, "__all__", [])
        classes = sorted(
            name
            for name in exported
            if isinstance(value := getattr(module, name), type) and issubclass(value, BaseModel)
        )
        assert sorted(table) == classes, f"name every model {module} exports in the table"
        assert all(table[name] is getattr(module, name) for name in table)


def test_a_strict_mypy_consumer_reads_this_module(tmp_path: Path) -> None:
    """`mypy --strict` resolves every import and every verb's type here, and `ty` does too."""
    executable = SCRIPTS / f"mypy{SUFFIX}"
    if not executable.is_file():
        pytest.fail(f"{executable} is missing; run `uv sync --frozen` from {Path.cwd()}")
    checked = subprocess.run(
        [
            str(executable),
            "--strict",
            "--no-incremental",
            "--cache-dir",
            str(tmp_path / "mypy-cache"),
            str(Path(__file__)),
        ],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    assert checked.returncode == 0, f"mypy --strict refused:\n{checked.stdout}{checked.stderr}"
