"""Every schema root, each imported from its generated module when first used."""

# ruff: noqa: F401, I001  # Generated public re-exports are used by consumers.
import importlib
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .attestation import Attestation as Attestation
    from .attestation_check import AttestationCheck as AttestationCheck
    from .attestation_state import AttestationState as AttestationState
    from .attested import Attested as Attested
    from .author_kind import AuthorKind as AuthorKind
    from .batch_outcome import BatchOutcome as BatchOutcome
    from .cache_cleared import CacheCleared as CacheCleared
    from .cache_use import CacheUse as CacheUse
    from .classification import Classification as Classification
    from .comment import Comment as Comment
    from .comment_activity import CommentActivity as CommentActivity
    from .comment_batch import CommentBatch as CommentBatch
    from .comment_list import CommentList as CommentList
    from .comment_written import CommentWritten as CommentWritten
    from .comments_added import CommentsAdded as CommentsAdded
    from .comments_since import CommentsSince as CommentsSince
    from .copy_action import CopyAction as CopyAction
    from .copy_link import CopyLink as CopyLink
    from .copy_outcome import CopyOutcome as CopyOutcome
    from .copy_report import CopyReport as CopyReport
    from .copy_via import CopyVia as CopyVia
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
    from .find_request import FindRequest as FindRequest
    from .found_tasks import FoundTasks as FoundTasks
    from .global_id import GlobalId as GlobalId
    from .human_task_projected import HumanTaskProjected as HumanTaskProjected
    from .human_task_projection import HumanTaskProjection as HumanTaskProjection
    from .issues_completed import IssuesCompleted as IssuesCompleted
    from .item_type import ItemType as ItemType
    from .linear_config import LinearConfig as LinearConfig
    from .link_batch import LinkBatch as LinkBatch
    from .link_outcome import LinkOutcome as LinkOutcome
    from .links_written import LinksWritten as LinksWritten
    from .locate_report import LocateReport as LocateReport
    from .located import Located as Located
    from .location import Location as Location
    from .member_declaration import MemberDeclaration as MemberDeclaration
    from .metadata_set import MetadataSet as MetadataSet
    from .native_id import NativeId as NativeId
    from .other_end import OtherEnd as OtherEnd
    from .page_of_document import Page as PageOfDocument
    from .parent_batch import ParentBatch as ParentBatch
    from .parent_outcome import ParentOutcome as ParentOutcome
    from .parents_set import ParentsSet as ParentsSet
    from .planned_project import PlannedProject as PlannedProject
    from .priority import Priority as Priority
    from .project_attestation_check import ProjectAttestationCheck as ProjectAttestationCheck
    from .project_graph import ProjectGraph as ProjectGraph
    from .project_status_set import ProjectStatusSet as ProjectStatusSet
    from .public_projections import PublicProjections as PublicProjections
    from .public_write_input import PublicWriteInput as PublicWriteInput
    from .qualified_task import Qualified as QualifiedTask
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
    from .sanitize_dry_run import SanitizeDryRun as SanitizeDryRun
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
    from .sweep_report import SweepReport as SweepReport
    from .task import Task as Task
    from .task_asset_names import TaskAssetNames as TaskAssetNames
    from .task_assignee_set import TaskAssigneeSet as TaskAssigneeSet
    from .task_content_set import TaskContentSet as TaskContentSet
    from .task_detail import TaskDetail as TaskDetail
    from .task_details import TaskDetails as TaskDetails
    from .task_priority_set import TaskPrioritySet as TaskPrioritySet
    from .task_ref import TaskRef as TaskRef
    from .task_relations import TaskRelations as TaskRelations
    from .task_status_set import TaskStatusSet as TaskStatusSet
    from .task_updated import TaskUpdated as TaskUpdated
    from .tasks_updated import TasksUpdated as TasksUpdated
    from .template_answers import TemplateAnswers as TemplateAnswers
    from .template_provenance import TemplateProvenance as TemplateProvenance
    from .template_variable import TemplateVariable as TemplateVariable
    from .template_variables import TemplateVariables as TemplateVariables
    from .update_batch import UpdateBatch as UpdateBatch
    from .updated_field import UpdatedField as UpdatedField
    from .variable_type import VariableType as VariableType
    from .void_reason import VoidReason as VoidReason
    from .write_policy_config import WritePolicyConfig as WritePolicyConfig
    from .write_verdict import WriteVerdict as WriteVerdict

# Every root is named here rather than left to the `import X as X` form alone: a
# root whose generated class carries another name — every `QueryResponseOf…`, and
# `PageOfDocument` — is aliased, and a strict type checker reads an aliased name as
# private to this module rather than as a re-export. This list is what makes the
# whole set public to one, here and through the package root's own `__all__`.
__all__ = [
    "Attestation",
    "AttestationCheck",
    "AttestationState",
    "Attested",
    "AuthorKind",
    "BatchOutcome",
    "CacheCleared",
    "CacheUse",
    "Classification",
    "Comment",
    "CommentActivity",
    "CommentBatch",
    "CommentList",
    "CommentWritten",
    "CommentsAdded",
    "CommentsSince",
    "CopyAction",
    "CopyLink",
    "CopyOutcome",
    "CopyReport",
    "CopyVia",
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
    "FindRequest",
    "FoundTasks",
    "GlobalId",
    "HumanTaskProjected",
    "HumanTaskProjection",
    "IssuesCompleted",
    "ItemType",
    "LinearConfig",
    "LinkBatch",
    "LinkOutcome",
    "LinksWritten",
    "LocateReport",
    "Located",
    "Location",
    "MemberDeclaration",
    "MetadataSet",
    "NativeId",
    "OtherEnd",
    "PageOfDocument",
    "ParentBatch",
    "ParentOutcome",
    "ParentsSet",
    "PlannedProject",
    "Priority",
    "ProjectAttestationCheck",
    "ProjectGraph",
    "ProjectStatusSet",
    "PublicProjections",
    "PublicWriteInput",
    "QualifiedTask",
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
    "SanitizeDryRun",
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
    "SweepReport",
    "Task",
    "TaskAssetNames",
    "TaskAssigneeSet",
    "TaskContentSet",
    "TaskDetail",
    "TaskDetails",
    "TaskPrioritySet",
    "TaskRef",
    "TaskRelations",
    "TaskStatusSet",
    "TaskUpdated",
    "TasksUpdated",
    "TemplateAnswers",
    "TemplateProvenance",
    "TemplateVariable",
    "TemplateVariables",
    "UpdateBatch",
    "UpdatedField",
    "VariableType",
    "VoidReason",
    "WritePolicyConfig",
    "WriteVerdict",
]

_MODULES: dict[str, tuple[str, str]] = {
    "Attestation": ("attestation", "Attestation"),
    "AttestationCheck": ("attestation_check", "AttestationCheck"),
    "AttestationState": ("attestation_state", "AttestationState"),
    "Attested": ("attested", "Attested"),
    "AuthorKind": ("author_kind", "AuthorKind"),
    "BatchOutcome": ("batch_outcome", "BatchOutcome"),
    "CacheCleared": ("cache_cleared", "CacheCleared"),
    "CacheUse": ("cache_use", "CacheUse"),
    "Classification": ("classification", "Classification"),
    "Comment": ("comment", "Comment"),
    "CommentActivity": ("comment_activity", "CommentActivity"),
    "CommentBatch": ("comment_batch", "CommentBatch"),
    "CommentList": ("comment_list", "CommentList"),
    "CommentWritten": ("comment_written", "CommentWritten"),
    "CommentsAdded": ("comments_added", "CommentsAdded"),
    "CommentsSince": ("comments_since", "CommentsSince"),
    "CopyAction": ("copy_action", "CopyAction"),
    "CopyLink": ("copy_link", "CopyLink"),
    "CopyOutcome": ("copy_outcome", "CopyOutcome"),
    "CopyReport": ("copy_report", "CopyReport"),
    "CopyVia": ("copy_via", "CopyVia"),
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
    "FindRequest": ("find_request", "FindRequest"),
    "FoundTasks": ("found_tasks", "FoundTasks"),
    "GlobalId": ("global_id", "GlobalId"),
    "HumanTaskProjected": ("human_task_projected", "HumanTaskProjected"),
    "HumanTaskProjection": ("human_task_projection", "HumanTaskProjection"),
    "IssuesCompleted": ("issues_completed", "IssuesCompleted"),
    "ItemType": ("item_type", "ItemType"),
    "LinearConfig": ("linear_config", "LinearConfig"),
    "LinkBatch": ("link_batch", "LinkBatch"),
    "LinkOutcome": ("link_outcome", "LinkOutcome"),
    "LinksWritten": ("links_written", "LinksWritten"),
    "LocateReport": ("locate_report", "LocateReport"),
    "Located": ("located", "Located"),
    "Location": ("location", "Location"),
    "MemberDeclaration": ("member_declaration", "MemberDeclaration"),
    "MetadataSet": ("metadata_set", "MetadataSet"),
    "NativeId": ("native_id", "NativeId"),
    "OtherEnd": ("other_end", "OtherEnd"),
    "PageOfDocument": ("page_of_document", "Page"),
    "ParentBatch": ("parent_batch", "ParentBatch"),
    "ParentOutcome": ("parent_outcome", "ParentOutcome"),
    "ParentsSet": ("parents_set", "ParentsSet"),
    "PlannedProject": ("planned_project", "PlannedProject"),
    "Priority": ("priority", "Priority"),
    "ProjectAttestationCheck": ("project_attestation_check", "ProjectAttestationCheck"),
    "ProjectGraph": ("project_graph", "ProjectGraph"),
    "ProjectStatusSet": ("project_status_set", "ProjectStatusSet"),
    "PublicProjections": ("public_projections", "PublicProjections"),
    "PublicWriteInput": ("public_write_input", "PublicWriteInput"),
    "QualifiedTask": ("qualified_task", "Qualified"),
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
    "SanitizeDryRun": ("sanitize_dry_run", "SanitizeDryRun"),
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
    "SweepReport": ("sweep_report", "SweepReport"),
    "Task": ("task", "Task"),
    "TaskAssetNames": ("task_asset_names", "TaskAssetNames"),
    "TaskAssigneeSet": ("task_assignee_set", "TaskAssigneeSet"),
    "TaskContentSet": ("task_content_set", "TaskContentSet"),
    "TaskDetail": ("task_detail", "TaskDetail"),
    "TaskDetails": ("task_details", "TaskDetails"),
    "TaskPrioritySet": ("task_priority_set", "TaskPrioritySet"),
    "TaskRef": ("task_ref", "TaskRef"),
    "TaskRelations": ("task_relations", "TaskRelations"),
    "TaskStatusSet": ("task_status_set", "TaskStatusSet"),
    "TaskUpdated": ("task_updated", "TaskUpdated"),
    "TasksUpdated": ("tasks_updated", "TasksUpdated"),
    "TemplateAnswers": ("template_answers", "TemplateAnswers"),
    "TemplateProvenance": ("template_provenance", "TemplateProvenance"),
    "TemplateVariable": ("template_variable", "TemplateVariable"),
    "TemplateVariables": ("template_variables", "TemplateVariables"),
    "UpdateBatch": ("update_batch", "UpdateBatch"),
    "UpdatedField": ("updated_field", "UpdatedField"),
    "VariableType": ("variable_type", "VariableType"),
    "VoidReason": ("void_reason", "VoidReason"),
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
