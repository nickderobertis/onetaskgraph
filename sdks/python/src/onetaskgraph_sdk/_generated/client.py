"""Generated typed client methods. Do not edit."""

from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Literal

from pydantic import JsonValue, TypeAdapter

from .models import (
    Comment,
    CommentList,
    CopyReport,
    DeletedComment,
    EffectiveConfig,
    FieldsReport,
    GlobalId,
    MetadataSet,
    Priority,
    QueryResponseOfQualifiedDocument,
    QueryResponseOfQualifiedEdge,
    QueryResponseOfQualifiedLabel,
    QueryResponseOfQualifiedProject,
    QueryResponseOfQualifiedTask,
    QueryResponseOfSearchHit,
    Regenerated,
    RenderedTemplate,
    SourceListing,
    SourceName,
    StatusCategory,
    StatusOptionsReport,
    TaskContentSet,
    TaskDetail,
    TaskDetails,
    TaskPrioritySet,
    TaskStatusSet,
    TaskUpdated,
    TemplateAnswers,
    TemplateVariables,
)

POSITIONALS: dict[tuple[str, ...], tuple[str, ...]] = {
    ("document", "answers"): ("id",),
    ("document", "copy"): ("ids",),
    ("document", "create"): ("source",),
    ("document", "metadata", "set"): ("id", "key", "value"),
    ("document", "render"): ("id",),
    ("document", "show"): ("id",),
    ("project", "copy"): ("id",),
    ("project", "deps"): ("id",),
    ("project", "metadata", "set"): ("id", "key", "value"),
    ("project", "show"): ("id",),
    ("search",): ("text",),
    ("sources", "fields"): ("source",),
    ("sources", "status-options"): ("source",),
    ("task", "answers"): ("id",),
    ("task", "comment", "add"): ("id",),
    ("task", "comment", "delete"): ("id", "comment_id"),
    ("task", "comment", "edit"): ("id", "comment_id"),
    ("task", "comment", "list"): ("id",),
    ("task", "content", "set"): ("id",),
    ("task", "copy"): ("ids",),
    ("task", "create"): ("source",),
    ("task", "deps"): ("id",),
    ("task", "metadata", "set"): ("id", "key", "value"),
    ("task", "priority", "set"): ("id", "priority"),
    ("task", "render"): ("id",),
    ("task", "show"): ("id",),
    ("task", "show-many"): ("ids",),
    ("task", "status", "set"): ("id", "category"),
    ("task", "update"): ("id",),
    ("template", "render"): ("file",),
    ("template", "variables"): ("file",),
}
"""The operands each command takes ahead of its options, in order, by command.

The runtime client builds the argument vector from this rather than from a second
table of its own: a verb whose operand was named in one place and forgotten in the
other generates a method that cannot do what it is named for, and nothing would say
so until the binary refused the invocation.
"""

_ANSWERS = TypeAdapter(dict[str, JsonValue])


def _answers_document(method: str, answers: Mapping[str, JsonValue]) -> str:
    """The answers as the JSON document the binary reads on standard input.

    Validated strictly first, so a key that is not a string or a value JSON cannot carry
    is refused here rather than coerced into something the caller did not pass; and
    serialised with `allow_nan=False`, because `json.dumps` would otherwise write a
    non-finite float as a bare `NaN` that the binary reads as text.
    """
    if not isinstance(answers, Mapping):
        kind = type(answers).__name__
        message = f"{method}: answers are a {kind}, not a mapping"
        raise TypeError(message)
    try:
        checked = _ANSWERS.validate_python(dict(answers), strict=True)
        return json.dumps(checked, allow_nan=False)
    except ValueError as error:
        message = f"{method}: answers are not a JSON mapping: {error}"
        raise TypeError(message) from error


def _template_file(method: str, file: object) -> str | None:
    """The template path, refused unless it is one — or `None`, for a loader document.

    Anything but a string would reach the binary as its string form, an empty one names
    no file, and one opening with `-` would be read as an option rather than as the file
    the caller named.
    """
    if file is None:
        return None
    if not isinstance(file, str) or not file or file.startswith("-"):
        message = (
            f"{method}: file is not a template path; next: pass the template's "
            "path as a non-empty string, spelling one that starts with `-` as `./-…`"
        )
        raise TypeError(message)
    return file


def _strings(method: str, option: str, values: object) -> list[str] | tuple[str, ...] | None:
    """The values of a repeated option, refused unless a list or tuple of strings.

    Checked rather than passed on whatever they are: each becomes one process argument,
    and anything but a string would reach the binary as its string form — a bare string
    as one argument per character.
    """
    if values is None:
        return None
    if not isinstance(values, (list, tuple)):
        kind = type(values).__name__
        message = f"{method}: {option} is a {kind}, not a list; next: pass a list of strings"
        raise TypeError(message)
    for index, value in enumerate(values):
        if not isinstance(value, str):
            kind = type(value).__name__
            message = (
                f"{method}: {option}[{index}] is a {kind}, not a string; next: pass "
                "each entry as a string"
            )
            raise TypeError(message)
    return values


def _stdin(
    method: str,
    body: str | None,
    answers: Mapping[str, JsonValue] | None,
    named: bool = False,
) -> str | None:
    """What a create or a render writes to the binary's standard input.

    The answers as JSON, or a create's plain body — never both, because standard input
    holds one document; and never a body beside a template, a loader document or a body
    file (`named`), which the binary reads instead of it.
    """
    if body is not None and answers is not None:
        message = (
            f"{method}: body and answers both go to standard input; next: pass a body "
            "without a template, or answers with one"
        )
        raise TypeError(message)
    if body is not None and named:
        message = (
            f"{method}: body is read only when no template, template_loader or "
            "body_file names the body; next: pass one of them, not both"
        )
        raise TypeError(message)
    if body is not None and not isinstance(body, str):
        kind = type(body).__name__
        message = f"{method}: body is a {kind}, not a string"
        raise TypeError(message)
    if answers is not None:
        return _answers_document(method, answers)
    return body


class GeneratedClient:
    """Methods generated from the binary command surface."""

    async def _invoke[T](
        self,
        command: list[str],
        model: object,
        *,
        stdin: str | None = None,
        **options: object,
    ) -> T:
        raise NotImplementedError

    async def config_show(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> EffectiveConfig:
        """Run ``onetaskgraph config show``."""
        return await self._invoke(
            ["config", "show"],
            EffectiveConfig,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def document_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers:
        """Run ``onetaskgraph document answers``."""
        return await self._invoke(
            ["document", "answers"],
            TemplateAnswers,
            id=id,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def document_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport:
        """Run ``onetaskgraph document copy``."""
        return await self._invoke(
            ["document", "copy"],
            CopyReport,
            ids=ids,
            default_sources=default_sources,
            dry_run=dry_run,
            match_by=match_by,
            page_size=page_size,
            recreate=recreate,
            set=set,
            to=to,
        )

    async def document_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        id: str | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> QueryResponseOfQualifiedDocument:
        """Run ``onetaskgraph document create``."""
        return await self._invoke(
            ["document", "create"],
            QueryResponseOfQualifiedDocument,
            source=source,
            project=project,
            title=title,
            body_file=body_file,
            default_sources=default_sources,
            id=id,
            label=label,
            metadata=_strings("document_create", "metadata", metadata),
            page_size=page_size,
            repository=_strings("document_create", "repository", repository),
            search_path=_strings("document_create", "search_path", search_path),
            set=set,
            template=template,
            template_loader=template_loader,
            var=_strings("document_create", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin(
                "document_create",
                body,
                answers,
                template is not None or template_loader is not None or body_file is not None,
            ),
        )

    async def document_list(
        self,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        in_: Literal["title", "content", "both"] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        limit: int | None = None,
        no_project: bool | None = None,
        not_label: list[str] | tuple[str, ...] | None = None,
        page: str | None = None,
        page_size: int | None = None,
        project: str | None = None,
        search: str | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedDocument:
        """Run ``onetaskgraph document list``."""
        return await self._invoke(
            ["document", "list"],
            QueryResponseOfQualifiedDocument,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            in_=in_,
            label=label,
            limit=limit,
            no_project=no_project,
            not_label=not_label,
            page=page,
            page_size=page_size,
            project=project,
            search=search,
            set=set,
            source=source,
        )

    async def document_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet:
        """Run ``onetaskgraph document metadata set``."""
        return await self._invoke(
            ["document", "metadata", "set"],
            MetadataSet,
            id=id,
            key=key,
            value=value,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def document_render(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated:
        """Run ``onetaskgraph document render``."""
        return await self._invoke(
            ["document", "render"],
            Regenerated,
            id=id,
            default_sources=default_sources,
            dry_run=dry_run,
            page_size=page_size,
            search_path=_strings("document_render", "search_path", search_path),
            set=set,
            template=template,
            template_loader=template_loader,
            unset=_strings("document_render", "unset", unset),
            var=_strings("document_render", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin("document_render", None, answers),
        )

    async def document_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedDocument:
        """Run ``onetaskgraph document show``."""
        return await self._invoke(
            ["document", "show"],
            QueryResponseOfQualifiedDocument,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            page_size=page_size,
            set=set,
        )

    async def label_list(
        self,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedLabel:
        """Run ``onetaskgraph label list``."""
        return await self._invoke(
            ["label", "list"],
            QueryResponseOfQualifiedLabel,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            limit=limit,
            page=page,
            page_size=page_size,
            set=set,
            source=source,
        )

    async def project_copy(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        member: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        no_tasks: bool | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport:
        """Run ``onetaskgraph project copy``."""
        return await self._invoke(
            ["project", "copy"],
            CopyReport,
            id=id,
            default_sources=default_sources,
            dry_run=dry_run,
            match_by=match_by,
            member=member,
            no_tasks=no_tasks,
            page_size=page_size,
            recreate=recreate,
            set=set,
            to=to,
        )

    async def project_deps(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["depends-on", "depended-on-by"] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge:
        """Run ``onetaskgraph project deps``."""
        return await self._invoke(
            ["project", "deps"],
            QueryResponseOfQualifiedEdge,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            direction=direction,
            explain=explain,
            limit=limit,
            page=page,
            page_size=page_size,
            set=set,
        )

    async def project_list(
        self,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        in_: Literal["title", "content", "both"] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        limit: int | None = None,
        not_label: list[str] | tuple[str, ...] | None = None,
        page: str | None = None,
        page_size: int | None = None,
        search: str | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
        status: list[
            Literal[
                "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
            ]
        ]
        | tuple[
            Literal[
                "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
            ],
            ...,
        ]
        | None = None,
    ) -> QueryResponseOfQualifiedProject:
        """Run ``onetaskgraph project list``."""
        return await self._invoke(
            ["project", "list"],
            QueryResponseOfQualifiedProject,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            in_=in_,
            label=label,
            limit=limit,
            not_label=not_label,
            page=page,
            page_size=page_size,
            search=search,
            set=set,
            source=source,
            status=status,
        )

    async def project_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet:
        """Run ``onetaskgraph project metadata set``."""
        return await self._invoke(
            ["project", "metadata", "set"],
            MetadataSet,
            id=id,
            key=key,
            value=value,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def project_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedProject:
        """Run ``onetaskgraph project show``."""
        return await self._invoke(
            ["project", "show"],
            QueryResponseOfQualifiedProject,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            page_size=page_size,
            set=set,
        )

    async def search(
        self,
        text: str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        in_: Literal["title", "content", "both"] | None = None,
        kind: Literal["task", "project", "both"] | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfSearchHit:
        """Run ``onetaskgraph search``."""
        return await self._invoke(
            ["search"],
            QueryResponseOfSearchHit,
            text=text,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            in_=in_,
            kind=kind,
            limit=limit,
            page=page,
            page_size=page_size,
            set=set,
            source=source,
        )

    async def sources_fields(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> FieldsReport:
        """Run ``onetaskgraph sources fields``."""
        return await self._invoke(
            ["sources", "fields"],
            FieldsReport,
            source=source,
            apply=apply,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def sources_list(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> list[SourceListing]:
        """Run ``onetaskgraph sources list``."""
        return await self._invoke(
            ["sources", "list"],
            list[SourceListing],
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def sources_status_options(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> StatusOptionsReport:
        """Run ``onetaskgraph sources status-options``."""
        return await self._invoke(
            ["sources", "status-options"],
            StatusOptionsReport,
            source=source,
            apply=apply,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def task_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers:
        """Run ``onetaskgraph task answers``."""
        return await self._invoke(
            ["task", "answers"],
            TemplateAnswers,
            id=id,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def task_comment_add(
        self,
        id: GlobalId | str,
        *,
        author: str | None = None,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> Comment:
        """Run ``onetaskgraph task comment add``."""
        return await self._invoke(
            ["task", "comment", "add"],
            Comment,
            id=id,
            author=author,
            body_file=body_file,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
            stdin=body,
        )

    async def task_comment_delete(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> DeletedComment:
        """Run ``onetaskgraph task comment delete``."""
        return await self._invoke(
            ["task", "comment", "delete"],
            DeletedComment,
            id=id,
            comment_id=comment_id,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def task_comment_edit(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> Comment:
        """Run ``onetaskgraph task comment edit``."""
        return await self._invoke(
            ["task", "comment", "edit"],
            Comment,
            id=id,
            comment_id=comment_id,
            body_file=body_file,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
            stdin=body,
        )

    async def task_comment_list(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> CommentList:
        """Run ``onetaskgraph task comment list``."""
        return await self._invoke(
            ["task", "comment", "list"],
            CommentList,
            id=id,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def task_content_set(
        self,
        id: GlobalId | str,
        file: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskContentSet:
        """Run ``onetaskgraph task content set``."""
        return await self._invoke(
            ["task", "content", "set"],
            TaskContentSet,
            id=id,
            file=file,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def task_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        create: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport:
        """Run ``onetaskgraph task copy``."""
        return await self._invoke(
            ["task", "copy"],
            CopyReport,
            ids=ids,
            create=create,
            default_sources=default_sources,
            dry_run=dry_run,
            match_by=match_by,
            page_size=page_size,
            recreate=recreate,
            set=set,
            to=to,
        )

    async def task_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> TaskDetail:
        """Run ``onetaskgraph task create``."""
        return await self._invoke(
            ["task", "create"],
            TaskDetail,
            source=source,
            project=project,
            title=title,
            body_file=body_file,
            default_sources=default_sources,
            delivers=delivers,
            depends_on=depends_on,
            label=label,
            metadata=_strings("task_create", "metadata", metadata),
            page_size=page_size,
            repository=_strings("task_create", "repository", repository),
            search_path=_strings("task_create", "search_path", search_path),
            set=set,
            status=status,
            template=template,
            template_loader=template_loader,
            var=_strings("task_create", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin(
                "task_create",
                body,
                answers,
                template is not None or template_loader is not None or body_file is not None,
            ),
        )

    async def task_deps(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["depends-on", "depended-on-by"] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge:
        """Run ``onetaskgraph task deps``."""
        return await self._invoke(
            ["task", "deps"],
            QueryResponseOfQualifiedEdge,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            direction=direction,
            explain=explain,
            limit=limit,
            page=page,
            page_size=page_size,
            set=set,
        )

    async def task_list(
        self,
        *,
        allow_partial: bool | None = None,
        commented_since: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        in_: Literal["title", "content", "both"] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        limit: int | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_project: bool | None = None,
        not_label: list[str] | tuple[str, ...] | None = None,
        origin: GlobalId | str | None = None,
        page: str | None = None,
        page_size: int | None = None,
        priority: list[Literal["none", "urgent", "high", "medium", "low"]]
        | tuple[Literal["none", "urgent", "high", "medium", "low"], ...]
        | None = None,
        project: str | None = None,
        search: str | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
        status: list[
            Literal[
                "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
            ]
        ]
        | tuple[
            Literal[
                "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
            ],
            ...,
        ]
        | None = None,
    ) -> QueryResponseOfQualifiedTask:
        """Run ``onetaskgraph task list``."""
        return await self._invoke(
            ["task", "list"],
            QueryResponseOfQualifiedTask,
            allow_partial=allow_partial,
            commented_since=commented_since,
            default_sources=default_sources,
            explain=explain,
            in_=in_,
            label=label,
            limit=limit,
            metadata=_strings("task_list", "metadata", metadata),
            no_project=no_project,
            not_label=not_label,
            origin=origin,
            page=page,
            page_size=page_size,
            priority=priority,
            project=project,
            search=search,
            set=set,
            source=source,
            status=status,
        )

    async def task_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet:
        """Run ``onetaskgraph task metadata set``."""
        return await self._invoke(
            ["task", "metadata", "set"],
            MetadataSet,
            id=id,
            key=key,
            value=value,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def task_priority_set(
        self,
        id: GlobalId | str,
        priority: Priority | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskPrioritySet:
        """Run ``onetaskgraph task priority set``."""
        return await self._invoke(
            ["task", "priority", "set"],
            TaskPrioritySet,
            id=id,
            priority=priority,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def task_render(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated:
        """Run ``onetaskgraph task render``."""
        return await self._invoke(
            ["task", "render"],
            Regenerated,
            id=id,
            default_sources=default_sources,
            dry_run=dry_run,
            page_size=page_size,
            search_path=_strings("task_render", "search_path", search_path),
            set=set,
            template=template,
            template_loader=template_loader,
            unset=_strings("task_render", "unset", unset),
            var=_strings("task_render", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin("task_render", None, answers),
        )

    async def task_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetail:
        """Run ``onetaskgraph task show``."""
        return await self._invoke(
            ["task", "show"],
            TaskDetail,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            no_comments=no_comments,
            page_size=page_size,
            set=set,
        )

    async def task_show_many(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetails:
        """Run ``onetaskgraph task show-many``."""
        return await self._invoke(
            ["task", "show-many"],
            TaskDetails,
            ids=ids,
            default_sources=default_sources,
            no_comments=no_comments,
            page_size=page_size,
            set=set,
        )

    async def task_status_set(
        self,
        id: GlobalId | str,
        category: StatusCategory | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskStatusSet:
        """Run ``onetaskgraph task status set``."""
        return await self._invoke(
            ["task", "status", "set"],
            TaskStatusSet,
            id=id,
            category=category,
            default_sources=default_sources,
            page_size=page_size,
            set=set,
        )

    async def task_update(
        self,
        id: GlobalId | str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_delivers: bool | None = None,
        no_depends_on: bool | None = None,
        page_size: int | None = None,
        priority: Literal["none", "urgent", "high", "medium", "low"] | None = None,
        remove_metadata: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        status_name: str | None = None,
        title: str | None = None,
    ) -> TaskUpdated:
        """Run ``onetaskgraph task update``."""
        return await self._invoke(
            ["task", "update"],
            TaskUpdated,
            id=id,
            body_file=body_file,
            default_sources=default_sources,
            delivers=delivers,
            depends_on=depends_on,
            metadata=_strings("task_update", "metadata", metadata),
            no_delivers=no_delivers,
            no_depends_on=no_depends_on,
            page_size=page_size,
            priority=priority,
            remove_metadata=_strings("task_update", "remove_metadata", remove_metadata),
            set=set,
            status=status,
            status_name=status_name,
            title=title,
        )

    async def template_render(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> RenderedTemplate:
        """Run ``onetaskgraph template render``."""
        return await self._invoke(
            ["template", "render"],
            RenderedTemplate,
            file=_template_file("template_render", file),
            default_sources=default_sources,
            page_size=page_size,
            search_path=_strings("template_render", "search_path", search_path),
            set=set,
            template_loader=template_loader,
            var=_strings("template_render", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin("template_render", None, answers),
        )

    async def template_variables(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
    ) -> TemplateVariables:
        """Run ``onetaskgraph template variables``."""
        return await self._invoke(
            ["template", "variables"],
            TemplateVariables,
            file=_template_file("template_variables", file),
            default_sources=default_sources,
            page_size=page_size,
            search_path=_strings("template_variables", "search_path", search_path),
            set=set,
            template_loader=template_loader,
        )
