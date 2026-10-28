"""Every schema root, each imported from its generated module when first used."""

# ruff: noqa: F401, I001  # Generated public re-exports are used by consumers.
import importlib
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .cache_cleared import CacheCleared as CacheCleared
    from .cache_use import CacheUse as CacheUse
    from .classification import Classification as Classification
    from .comment_list import CommentList as CommentList
    from .comment_written import CommentWritten as CommentWritten
    from .copy_report import CopyReport as CopyReport
    from .deleted_comment import DeletedComment as DeletedComment
    from .delivered import Delivered as Delivered
    from .delivery_outcome import DeliveryOutcome as DeliveryOutcome
    from .document import Document as Document
    from .document_detail import DocumentDetail as DocumentDetail
    from .document_query import DocumentQuery as DocumentQuery
    from .effective_config import EffectiveConfig as EffectiveConfig
    from .expected_write import ExpectedWrite as ExpectedWrite
    from .failure_document import FailureDocument as FailureDocument
    from .fields_report import FieldsReport as FieldsReport
    from .global_id import GlobalId as GlobalId
    from .issues_completed import IssuesCompleted as IssuesCompleted
    from .item_type import ItemType as ItemType
    from .linear_config import LinearConfig as LinearConfig
    from .locate_report import LocateReport as LocateReport
    from .location import Location as Location
    from .metadata_set import MetadataSet as MetadataSet
    from .native_id import NativeId as NativeId
    from .page_of_document import Page as PageOfDocument
    from .priority import Priority as Priority
    from .project_graph import ProjectGraph as ProjectGraph
    from .project_status_set import ProjectStatusSet as ProjectStatusSet
    from .public_projections import PublicProjections as PublicProjections
    from .public_write_input import PublicWriteInput as PublicWriteInput
    from .query_plan import QueryPlan as QueryPlan
    from .query_response_of_qualified_document import (
        QueryResponse as QueryResponseOfQualifiedDocument,
    )
    from .query_response_of_qualified_edge import QueryResponse as QueryResponseOfQualifiedEdge
    from .query_response_of_qualified_label import QueryResponse as QueryResponseOfQualifiedLabel
    from .query_response_of_qualified_project import (
        QueryResponse as QueryResponseOfQualifiedProject,
    )
    from .query_response_of_qualified_task import QueryResponse as QueryResponseOfQualifiedTask
    from .query_response_of_search_hit import QueryResponse as QueryResponseOfSearchHit
    from .regenerated import Regenerated as Regenerated
    from .rendered_template import RenderedTemplate as RenderedTemplate
    from .repository_visibility import RepositoryVisibility as RepositoryVisibility
    from .source_cache_config import SourceCacheConfig as SourceCacheConfig
    from .source_failure import SourceFailure as SourceFailure
    from .source_listing import SourceListing as SourceListing
    from .source_name import SourceName as SourceName
    from .source_route import SourceRoute as SourceRoute
    from .source_visibility import SourceVisibility as SourceVisibility
    from .status_category import StatusCategory as StatusCategory
    from .status_mapping import StatusMapping as StatusMapping
    from .status_names_report import StatusNamesReport as StatusNamesReport
    from .status_options_report import StatusOptionsReport as StatusOptionsReport
    from .statuses_reconciled import StatusesReconciled as StatusesReconciled
    from .task_asset_names import TaskAssetNames as TaskAssetNames
    from .task_content_set import TaskContentSet as TaskContentSet
    from .task_detail import TaskDetail as TaskDetail
    from .task_details import TaskDetails as TaskDetails
    from .task_priority_set import TaskPrioritySet as TaskPrioritySet
    from .task_ref import TaskRef as TaskRef
    from .task_status_set import TaskStatusSet as TaskStatusSet
    from .task_updated import TaskUpdated as TaskUpdated
    from .template_answers import TemplateAnswers as TemplateAnswers
    from .template_provenance import TemplateProvenance as TemplateProvenance
    from .template_variable import TemplateVariable as TemplateVariable
    from .template_variables import TemplateVariables as TemplateVariables
    from .updated_field import UpdatedField as UpdatedField
    from .variable_type import VariableType as VariableType
    from .write_policy_config import WritePolicyConfig as WritePolicyConfig
    from .write_verdict import WriteVerdict as WriteVerdict

# Every root is named here rather than left to the `import X as X` form alone: a
# root whose generated class carries another name — every `QueryResponseOf…`, and
# `PageOfDocument` — is aliased, and a strict type checker reads an aliased name as
# private to this module rather than as a re-export. This list is what makes the
# whole set public to one, here and through the package root's own `__all__`.
__all__ = [
    "CacheCleared",
    "CacheUse",
    "Classification",
    "CommentList",
    "CommentWritten",
    "CopyReport",
    "DeletedComment",
    "Delivered",
    "DeliveryOutcome",
    "Document",
    "DocumentDetail",
    "DocumentQuery",
    "EffectiveConfig",
    "ExpectedWrite",
    "FailureDocument",
    "FieldsReport",
    "GlobalId",
    "IssuesCompleted",
    "ItemType",
    "LinearConfig",
    "LocateReport",
    "Location",
    "MetadataSet",
    "NativeId",
    "PageOfDocument",
    "Priority",
    "ProjectGraph",
    "ProjectStatusSet",
    "PublicProjections",
    "PublicWriteInput",
    "QueryPlan",
    "QueryResponseOfQualifiedDocument",
    "QueryResponseOfQualifiedEdge",
    "QueryResponseOfQualifiedLabel",
    "QueryResponseOfQualifiedProject",
    "QueryResponseOfQualifiedTask",
    "QueryResponseOfSearchHit",
    "Regenerated",
    "RenderedTemplate",
    "RepositoryVisibility",
    "SourceCacheConfig",
    "SourceFailure",
    "SourceListing",
    "SourceName",
    "SourceRoute",
    "SourceVisibility",
    "StatusCategory",
    "StatusMapping",
    "StatusNamesReport",
    "StatusOptionsReport",
    "StatusesReconciled",
    "TaskAssetNames",
    "TaskContentSet",
    "TaskDetail",
    "TaskDetails",
    "TaskPrioritySet",
    "TaskRef",
    "TaskStatusSet",
    "TaskUpdated",
    "TemplateAnswers",
    "TemplateProvenance",
    "TemplateVariable",
    "TemplateVariables",
    "UpdatedField",
    "VariableType",
    "WritePolicyConfig",
    "WriteVerdict",
]

_MODULES: dict[str, tuple[str, str]] = {
    "CacheCleared": ("cache_cleared", "CacheCleared"),
    "CacheUse": ("cache_use", "CacheUse"),
    "Classification": ("classification", "Classification"),
    "CommentList": ("comment_list", "CommentList"),
    "CommentWritten": ("comment_written", "CommentWritten"),
    "CopyReport": ("copy_report", "CopyReport"),
    "DeletedComment": ("deleted_comment", "DeletedComment"),
    "Delivered": ("delivered", "Delivered"),
    "DeliveryOutcome": ("delivery_outcome", "DeliveryOutcome"),
    "Document": ("document", "Document"),
    "DocumentDetail": ("document_detail", "DocumentDetail"),
    "DocumentQuery": ("document_query", "DocumentQuery"),
    "EffectiveConfig": ("effective_config", "EffectiveConfig"),
    "ExpectedWrite": ("expected_write", "ExpectedWrite"),
    "FailureDocument": ("failure_document", "FailureDocument"),
    "FieldsReport": ("fields_report", "FieldsReport"),
    "GlobalId": ("global_id", "GlobalId"),
    "IssuesCompleted": ("issues_completed", "IssuesCompleted"),
    "ItemType": ("item_type", "ItemType"),
    "LinearConfig": ("linear_config", "LinearConfig"),
    "LocateReport": ("locate_report", "LocateReport"),
    "Location": ("location", "Location"),
    "MetadataSet": ("metadata_set", "MetadataSet"),
    "NativeId": ("native_id", "NativeId"),
    "PageOfDocument": ("page_of_document", "Page"),
    "Priority": ("priority", "Priority"),
    "ProjectGraph": ("project_graph", "ProjectGraph"),
    "ProjectStatusSet": ("project_status_set", "ProjectStatusSet"),
    "PublicProjections": ("public_projections", "PublicProjections"),
    "PublicWriteInput": ("public_write_input", "PublicWriteInput"),
    "QueryPlan": ("query_plan", "QueryPlan"),
    "QueryResponseOfQualifiedDocument": ("query_response_of_qualified_document", "QueryResponse"),
    "QueryResponseOfQualifiedEdge": ("query_response_of_qualified_edge", "QueryResponse"),
    "QueryResponseOfQualifiedLabel": ("query_response_of_qualified_label", "QueryResponse"),
    "QueryResponseOfQualifiedProject": ("query_response_of_qualified_project", "QueryResponse"),
    "QueryResponseOfQualifiedTask": ("query_response_of_qualified_task", "QueryResponse"),
    "QueryResponseOfSearchHit": ("query_response_of_search_hit", "QueryResponse"),
    "Regenerated": ("regenerated", "Regenerated"),
    "RenderedTemplate": ("rendered_template", "RenderedTemplate"),
    "RepositoryVisibility": ("repository_visibility", "RepositoryVisibility"),
    "SourceCacheConfig": ("source_cache_config", "SourceCacheConfig"),
    "SourceFailure": ("source_failure", "SourceFailure"),
    "SourceListing": ("source_listing", "SourceListing"),
    "SourceName": ("source_name", "SourceName"),
    "SourceRoute": ("source_route", "SourceRoute"),
    "SourceVisibility": ("source_visibility", "SourceVisibility"),
    "StatusCategory": ("status_category", "StatusCategory"),
    "StatusMapping": ("status_mapping", "StatusMapping"),
    "StatusNamesReport": ("status_names_report", "StatusNamesReport"),
    "StatusOptionsReport": ("status_options_report", "StatusOptionsReport"),
    "StatusesReconciled": ("statuses_reconciled", "StatusesReconciled"),
    "TaskAssetNames": ("task_asset_names", "TaskAssetNames"),
    "TaskContentSet": ("task_content_set", "TaskContentSet"),
    "TaskDetail": ("task_detail", "TaskDetail"),
    "TaskDetails": ("task_details", "TaskDetails"),
    "TaskPrioritySet": ("task_priority_set", "TaskPrioritySet"),
    "TaskRef": ("task_ref", "TaskRef"),
    "TaskStatusSet": ("task_status_set", "TaskStatusSet"),
    "TaskUpdated": ("task_updated", "TaskUpdated"),
    "TemplateAnswers": ("template_answers", "TemplateAnswers"),
    "TemplateProvenance": ("template_provenance", "TemplateProvenance"),
    "TemplateVariable": ("template_variable", "TemplateVariable"),
    "TemplateVariables": ("template_variables", "TemplateVariables"),
    "UpdatedField": ("updated_field", "UpdatedField"),
    "VariableType": ("variable_type", "VariableType"),
    "WritePolicyConfig": ("write_policy_config", "WritePolicyConfig"),
    "WriteVerdict": ("write_verdict", "WriteVerdict"),
}
"""Each root's module, and its class's name there, by its published name."""

if not TYPE_CHECKING:

    def __getattr__(name: str) -> object:
        """Import the module defining `name`, alone, and bind the root here."""
        try:
            module, attribute = _MODULES[name]
        except KeyError:
            message = f"module {__name__!r} has no attribute {name!r}"
            raise AttributeError(message) from None
        value = getattr(importlib.import_module(f".{module}", __package__), attribute)
        globals()[name] = value
        return value

    def __dir__() -> list[str]:
        """Every name bound here and every root, imported or not."""
        return sorted({*globals(), *__all__})
