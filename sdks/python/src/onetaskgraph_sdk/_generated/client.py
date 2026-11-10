"""Generated typed client methods. Do not edit."""

from __future__ import annotations

import json
from collections.abc import Mapping
from functools import cache
from typing import TYPE_CHECKING, Literal, overload

from pydantic import JsonValue, TypeAdapter

from . import models as _models

if TYPE_CHECKING:
    from .models import (
        CacheCleared,
        CommentList,
        CommentWritten,
        CopyReport,
        DeletedComment,
        DocumentDetail,
        EffectiveConfig,
        FieldsReport,
        GlobalId,
        HumanTaskProjected,
        IssuesCompleted,
        LocateReport,
        MetadataSet,
        NativeId,
        Priority,
        ProjectGraph,
        ProjectStatusSet,
        QueryResponseOfQualifiedDocument,
        QueryResponseOfQualifiedEdge,
        QueryResponseOfQualifiedLabel,
        QueryResponseOfQualifiedProject,
        QueryResponseOfQualifiedTask,
        QueryResponseOfSearchHit,
        Regenerated,
        RenderedTemplate,
        SanitizeDryRun,
        SourceListing,
        SourceName,
        SourceRoute,
        StatusCategory,
        StatusesReconciled,
        StatusNamesReport,
        StatusOptionsReport,
        SweepReport,
        TaskAssetNames,
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
    ("cache", "clear"): ("source",),
    ("document", "answers"): ("id",),
    ("document", "copy"): ("ids",),
    ("document", "create"): ("source",),
    ("document", "metadata", "set"): ("id", "key", "value"),
    ("document", "render"): ("id",),
    ("document", "show"): ("id",),
    ("project", "answers"): ("id",),
    ("project", "copy"): ("id",),
    ("project", "create"): ("source",),
    ("project", "deps"): ("id",),
    ("project", "graph"): ("id",),
    ("project", "metadata", "set"): ("id", "key", "value"),
    ("project", "render"): ("id",),
    ("project", "show"): ("id",),
    ("project", "status", "set"): ("id", "category"),
    ("search",): ("text",),
    ("sources", "fields"): ("source",),
    ("sources", "route"): ("source",),
    ("sources", "sanitize"): ("source",),
    ("sources", "status-options"): ("source",),
    ("task", "answers"): ("id",),
    ("task", "assets"): ("id",),
    ("task", "comment", "add"): ("id",),
    ("task", "comment", "delete"): ("id", "comment_id"),
    ("task", "comment", "edit"): ("id", "comment_id"),
    ("task", "comment", "list"): ("id",),
    ("task", "content", "set"): ("id",),
    ("task", "copy"): ("ids",),
    ("task", "create"): ("source",),
    ("task", "deps"): ("id",),
    ("task", "locate"): ("ids",),
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

type VersionToken = str
"""A task's change token, its `version`, exactly as a read of it reported it.

Opaque: only the source that spelled one reads inside it. A read pinned to one, and a
write expecting one, hand it back to the binary as it came.
"""


@cache
def _answers() -> TypeAdapter[dict[str, JsonValue]]:
    """The validator of an answers mapping, built the first time a call passes one."""
    return TypeAdapter(dict[str, JsonValue])


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
        checked = _answers().validate_python(dict(answers), strict=True)
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

    @overload
    async def cache_clear(
        self,
        source: SourceName | str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> CacheCleared: ...

    @overload
    async def cache_clear(
        self,
        source: SourceName | str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> CacheCleared | SanitizeDryRun: ...

    @overload
    async def cache_clear(
        self,
        source: SourceName | str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> CacheCleared | SanitizeDryRun: ...

    async def cache_clear(
        self,
        source: SourceName | str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> CacheCleared | SanitizeDryRun:
        """Run ``onetaskgraph cache clear``."""
        return await self._invoke(
            ["cache", "clear"],
            _models.CacheCleared | _models.SanitizeDryRun,
            source=source,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def config_show(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> EffectiveConfig: ...

    @overload
    async def config_show(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> EffectiveConfig | SanitizeDryRun: ...

    @overload
    async def config_show(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> EffectiveConfig | SanitizeDryRun: ...

    async def config_show(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> EffectiveConfig | SanitizeDryRun:
        """Run ``onetaskgraph config show``."""
        return await self._invoke(
            ["config", "show"],
            _models.EffectiveConfig | _models.SanitizeDryRun,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def document_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers: ...

    @overload
    async def document_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers | SanitizeDryRun: ...

    @overload
    async def document_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers | SanitizeDryRun: ...

    async def document_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers | SanitizeDryRun:
        """Run ``onetaskgraph document answers``."""
        return await self._invoke(
            ["document", "answers"],
            _models.TemplateAnswers | _models.SanitizeDryRun,
            id=id,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def document_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        no_link: bool | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport: ...

    @overload
    async def document_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        no_link: bool | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport | SanitizeDryRun: ...

    @overload
    async def document_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        no_link: bool | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport | SanitizeDryRun: ...

    async def document_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        no_link: bool | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport | SanitizeDryRun:
        """Run ``onetaskgraph document copy``."""
        return await self._invoke(
            ["document", "copy"],
            _models.CopyReport | _models.SanitizeDryRun,
            ids=ids,
            default_sources=default_sources,
            dry_run=dry_run,
            match_by=match_by,
            no_link=no_link,
            page_size=page_size,
            recreate=recreate,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("document_copy", "term_scope", term_scope),
            to=to,
        )

    @overload
    async def document_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        id: str | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> DocumentDetail: ...

    @overload
    async def document_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        id: str | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> DocumentDetail | SanitizeDryRun: ...

    @overload
    async def document_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        id: str | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> DocumentDetail | SanitizeDryRun: ...

    async def document_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        id: str | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> DocumentDetail | SanitizeDryRun:
        """Run ``onetaskgraph document create``."""
        return await self._invoke(
            ["document", "create"],
            _models.DocumentDetail | _models.SanitizeDryRun,
            source=source,
            project=project,
            title=title,
            asset=asset,
            body_file=body_file,
            classification=classification,
            default_sources=default_sources,
            id=id,
            label=label,
            metadata=_strings("document_create", "metadata", metadata),
            page_size=page_size,
            reject=reject,
            replace=replace,
            repository=_strings("document_create", "repository", repository),
            sanitize_dry_run=sanitize_dry_run,
            search_path=_strings("document_create", "search_path", search_path),
            set=set,
            template=template,
            template_loader=template_loader,
            term_scope=_strings("document_create", "term_scope", term_scope),
            var=_strings("document_create", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin(
                "document_create",
                body,
                answers,
                template is not None or template_loader is not None or body_file is not None,
            ),
        )

    @overload
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        search: str | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedDocument: ...

    @overload
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        search: str | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedDocument | SanitizeDryRun: ...

    @overload
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search: str | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedDocument | SanitizeDryRun: ...

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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search: str | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedDocument | SanitizeDryRun:
        """Run ``onetaskgraph document list``."""
        return await self._invoke(
            ["document", "list"],
            _models.QueryResponseOfQualifiedDocument | _models.SanitizeDryRun,
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
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            search=search,
            set=set,
            source=source,
        )

    @overload
    async def document_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet: ...

    @overload
    async def document_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet | SanitizeDryRun: ...

    @overload
    async def document_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet | SanitizeDryRun: ...

    async def document_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet | SanitizeDryRun:
        """Run ``onetaskgraph document metadata set``."""
        return await self._invoke(
            ["document", "metadata", "set"],
            _models.MetadataSet | _models.SanitizeDryRun,
            id=id,
            key=key,
            value=value,
            default_sources=default_sources,
            expect=expect,
            if_version=if_version,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("document_metadata_set", "term_scope", term_scope),
        )

    @overload
    async def document_render(
        self,
        id: GlobalId | str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated: ...

    @overload
    async def document_render(
        self,
        id: GlobalId | str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated | SanitizeDryRun: ...

    @overload
    async def document_render(
        self,
        id: GlobalId | str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated | SanitizeDryRun: ...

    async def document_render(
        self,
        id: GlobalId | str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated | SanitizeDryRun:
        """Run ``onetaskgraph document render``."""
        return await self._invoke(
            ["document", "render"],
            _models.Regenerated | _models.SanitizeDryRun,
            id=id,
            asset=asset,
            default_sources=default_sources,
            dry_run=dry_run,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            search_path=_strings("document_render", "search_path", search_path),
            set=set,
            template=template,
            template_loader=template_loader,
            term_scope=_strings("document_render", "term_scope", term_scope),
            unset=_strings("document_render", "unset", unset),
            var=_strings("document_render", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin("document_render", None, answers),
        )

    @overload
    async def document_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> DocumentDetail: ...

    @overload
    async def document_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> DocumentDetail | SanitizeDryRun: ...

    @overload
    async def document_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> DocumentDetail | SanitizeDryRun: ...

    async def document_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> DocumentDetail | SanitizeDryRun:
        """Run ``onetaskgraph document show``."""
        return await self._invoke(
            ["document", "show"],
            _models.DocumentDetail | _models.SanitizeDryRun,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            page_size=page_size,
            pin=pin,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def label_list(
        self,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedLabel: ...

    @overload
    async def label_list(
        self,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedLabel | SanitizeDryRun: ...

    @overload
    async def label_list(
        self,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedLabel | SanitizeDryRun: ...

    async def label_list(
        self,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedLabel | SanitizeDryRun:
        """Run ``onetaskgraph label list``."""
        return await self._invoke(
            ["label", "list"],
            _models.QueryResponseOfQualifiedLabel | _models.SanitizeDryRun,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            limit=limit,
            page=page,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            source=source,
        )

    @overload
    async def project_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers: ...

    @overload
    async def project_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers | SanitizeDryRun: ...

    @overload
    async def project_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers | SanitizeDryRun: ...

    async def project_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers | SanitizeDryRun:
        """Run ``onetaskgraph project answers``."""
        return await self._invoke(
            ["project", "answers"],
            _models.TemplateAnswers | _models.SanitizeDryRun,
            id=id,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def project_copy(
        self,
        id: GlobalId | str,
        *,
        comments: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        member: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        members_only: bool | None = None,
        no_link: bool | None = None,
        no_tasks: bool | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status_policy: Literal["source", "preserve-destination"] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport: ...

    @overload
    async def project_copy(
        self,
        id: GlobalId | str,
        *,
        comments: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        member: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        members_only: bool | None = None,
        no_link: bool | None = None,
        no_tasks: bool | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        status_policy: Literal["source", "preserve-destination"] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport | SanitizeDryRun: ...

    @overload
    async def project_copy(
        self,
        id: GlobalId | str,
        *,
        comments: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        member: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        members_only: bool | None = None,
        no_link: bool | None = None,
        no_tasks: bool | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status_policy: Literal["source", "preserve-destination"] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport | SanitizeDryRun: ...

    async def project_copy(
        self,
        id: GlobalId | str,
        *,
        comments: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        match_by: str | None = None,
        member: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        members_only: bool | None = None,
        no_link: bool | None = None,
        no_tasks: bool | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status_policy: Literal["source", "preserve-destination"] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
    ) -> CopyReport | SanitizeDryRun:
        """Run ``onetaskgraph project copy``."""
        return await self._invoke(
            ["project", "copy"],
            _models.CopyReport | _models.SanitizeDryRun,
            id=id,
            comments=comments,
            default_sources=default_sources,
            dry_run=dry_run,
            match_by=match_by,
            member=member,
            members_only=members_only,
            no_link=no_link,
            no_tasks=no_tasks,
            page_size=page_size,
            recreate=recreate,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            status_policy=status_policy,
            term_scope=_strings("project_copy", "term_scope", term_scope),
            to=to,
        )

    @overload
    async def project_create(
        self,
        source: SourceName | str,
        id: NativeId | str,
        title: str,
        *,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> QueryResponseOfQualifiedProject: ...

    @overload
    async def project_create(
        self,
        source: SourceName | str,
        id: NativeId | str,
        title: str,
        *,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> QueryResponseOfQualifiedProject | SanitizeDryRun: ...

    @overload
    async def project_create(
        self,
        source: SourceName | str,
        id: NativeId | str,
        title: str,
        *,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> QueryResponseOfQualifiedProject | SanitizeDryRun: ...

    async def project_create(
        self,
        source: SourceName | str,
        id: NativeId | str,
        title: str,
        *,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> QueryResponseOfQualifiedProject | SanitizeDryRun:
        """Run ``onetaskgraph project create``."""
        return await self._invoke(
            ["project", "create"],
            _models.QueryResponseOfQualifiedProject | _models.SanitizeDryRun,
            source=source,
            id=id,
            title=title,
            body_file=body_file,
            classification=classification,
            default_sources=default_sources,
            label=label,
            metadata=_strings("project_create", "metadata", metadata),
            page_size=page_size,
            reject=reject,
            replace=replace,
            repository=_strings("project_create", "repository", repository),
            sanitize_dry_run=sanitize_dry_run,
            search_path=_strings("project_create", "search_path", search_path),
            set=set,
            status=status,
            template=template,
            template_loader=template_loader,
            term_scope=_strings("project_create", "term_scope", term_scope),
            var=_strings("project_create", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin(
                "project_create",
                body,
                answers,
                template is not None or template_loader is not None or body_file is not None,
            ),
        )

    @overload
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
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge: ...

    @overload
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
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge | SanitizeDryRun: ...

    @overload
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
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge | SanitizeDryRun: ...

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
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge | SanitizeDryRun:
        """Run ``onetaskgraph project deps``."""
        return await self._invoke(
            ["project", "deps"],
            _models.QueryResponseOfQualifiedEdge | _models.SanitizeDryRun,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            direction=direction,
            explain=explain,
            limit=limit,
            page=page,
            page_size=page_size,
            pin=pin,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def project_graph(
        self,
        id: GlobalId | str,
        *,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["auto", "td", "lr"] | None = None,
        group_by: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> ProjectGraph: ...

    @overload
    async def project_graph(
        self,
        id: GlobalId | str,
        *,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["auto", "td", "lr"] | None = None,
        group_by: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> ProjectGraph | SanitizeDryRun: ...

    @overload
    async def project_graph(
        self,
        id: GlobalId | str,
        *,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["auto", "td", "lr"] | None = None,
        group_by: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> ProjectGraph | SanitizeDryRun: ...

    async def project_graph(
        self,
        id: GlobalId | str,
        *,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["auto", "td", "lr"] | None = None,
        group_by: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> ProjectGraph | SanitizeDryRun:
        """Run ``onetaskgraph project graph``."""
        return await self._invoke(
            ["project", "graph"],
            _models.ProjectGraph | _models.SanitizeDryRun,
            id=id,
            complete=complete,
            default_sources=default_sources,
            direction=direction,
            group_by=group_by,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            format="json",
        )

    @overload
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
        origin: GlobalId | str | None = None,
        page: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
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
    ) -> QueryResponseOfQualifiedProject: ...

    @overload
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
        origin: GlobalId | str | None = None,
        page: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
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
    ) -> QueryResponseOfQualifiedProject | SanitizeDryRun: ...

    @overload
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
        origin: GlobalId | str | None = None,
        page: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
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
    ) -> QueryResponseOfQualifiedProject | SanitizeDryRun: ...

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
        origin: GlobalId | str | None = None,
        page: str | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
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
    ) -> QueryResponseOfQualifiedProject | SanitizeDryRun:
        """Run ``onetaskgraph project list``."""
        return await self._invoke(
            ["project", "list"],
            _models.QueryResponseOfQualifiedProject | _models.SanitizeDryRun,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            in_=in_,
            label=label,
            limit=limit,
            not_label=not_label,
            origin=origin,
            page=page,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            search=search,
            set=set,
            source=source,
            status=status,
        )

    @overload
    async def project_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet: ...

    @overload
    async def project_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet | SanitizeDryRun: ...

    @overload
    async def project_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet | SanitizeDryRun: ...

    async def project_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet | SanitizeDryRun:
        """Run ``onetaskgraph project metadata set``."""
        return await self._invoke(
            ["project", "metadata", "set"],
            _models.MetadataSet | _models.SanitizeDryRun,
            id=id,
            key=key,
            value=value,
            default_sources=default_sources,
            expect=expect,
            if_version=if_version,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("project_metadata_set", "term_scope", term_scope),
        )

    @overload
    async def project_render(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated: ...

    @overload
    async def project_render(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated | SanitizeDryRun: ...

    @overload
    async def project_render(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated | SanitizeDryRun: ...

    async def project_render(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated | SanitizeDryRun:
        """Run ``onetaskgraph project render``."""
        return await self._invoke(
            ["project", "render"],
            _models.Regenerated | _models.SanitizeDryRun,
            id=id,
            default_sources=default_sources,
            dry_run=dry_run,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            search_path=_strings("project_render", "search_path", search_path),
            set=set,
            template=template,
            template_loader=template_loader,
            term_scope=_strings("project_render", "term_scope", term_scope),
            unset=_strings("project_render", "unset", unset),
            var=_strings("project_render", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin("project_render", None, answers),
        )

    @overload
    async def project_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedProject: ...

    @overload
    async def project_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedProject | SanitizeDryRun: ...

    @overload
    async def project_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedProject | SanitizeDryRun: ...

    async def project_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedProject | SanitizeDryRun:
        """Run ``onetaskgraph project show``."""
        return await self._invoke(
            ["project", "show"],
            _models.QueryResponseOfQualifiedProject | _models.SanitizeDryRun,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            page_size=page_size,
            pin=pin,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def project_status_set(
        self,
        id: GlobalId | str,
        category: StatusCategory | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> ProjectStatusSet: ...

    @overload
    async def project_status_set(
        self,
        id: GlobalId | str,
        category: StatusCategory | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> ProjectStatusSet | SanitizeDryRun: ...

    @overload
    async def project_status_set(
        self,
        id: GlobalId | str,
        category: StatusCategory | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> ProjectStatusSet | SanitizeDryRun: ...

    async def project_status_set(
        self,
        id: GlobalId | str,
        category: StatusCategory | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> ProjectStatusSet | SanitizeDryRun:
        """Run ``onetaskgraph project status set``."""
        return await self._invoke(
            ["project", "status", "set"],
            _models.ProjectStatusSet | _models.SanitizeDryRun,
            id=id,
            category=category,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("project_status_set", "term_scope", term_scope),
        )

    @overload
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfSearchHit: ...

    @overload
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfSearchHit | SanitizeDryRun: ...

    @overload
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfSearchHit | SanitizeDryRun: ...

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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfSearchHit | SanitizeDryRun:
        """Run ``onetaskgraph search``."""
        return await self._invoke(
            ["search"],
            _models.QueryResponseOfSearchHit | _models.SanitizeDryRun,
            text=text,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            in_=in_,
            kind=kind,
            limit=limit,
            page=page,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            source=source,
        )

    @overload
    async def sources_fields(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> FieldsReport | StatusNamesReport: ...

    @overload
    async def sources_fields(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> FieldsReport | StatusNamesReport | SanitizeDryRun: ...

    @overload
    async def sources_fields(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> FieldsReport | StatusNamesReport | SanitizeDryRun: ...

    async def sources_fields(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> FieldsReport | StatusNamesReport | SanitizeDryRun:
        """Run ``onetaskgraph sources fields``."""
        return await self._invoke(
            ["sources", "fields"],
            _models.FieldsReport | _models.StatusNamesReport | _models.SanitizeDryRun,
            source=source,
            apply=apply,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def sources_list(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> list[SourceListing]: ...

    @overload
    async def sources_list(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> list[SourceListing] | SanitizeDryRun: ...

    @overload
    async def sources_list(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> list[SourceListing] | SanitizeDryRun: ...

    async def sources_list(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> list[SourceListing] | SanitizeDryRun:
        """Run ``onetaskgraph sources list``."""
        return await self._invoke(
            ["sources", "list"],
            list[_models.SourceListing] | _models.SanitizeDryRun,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def sources_route(
        self,
        source: SourceName | str,
        *,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> SourceRoute: ...

    @overload
    async def sources_route(
        self,
        source: SourceName | str,
        *,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> SourceRoute | SanitizeDryRun: ...

    @overload
    async def sources_route(
        self,
        source: SourceName | str,
        *,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> SourceRoute | SanitizeDryRun: ...

    async def sources_route(
        self,
        source: SourceName | str,
        *,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> SourceRoute | SanitizeDryRun:
        """Run ``onetaskgraph sources route``."""
        return await self._invoke(
            ["sources", "route"],
            _models.SourceRoute | _models.SanitizeDryRun,
            source=source,
            classification=classification,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            repository=_strings("sources_route", "repository", repository),
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def sources_sanitize(
        self,
        source: SourceName | str,
        *,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> SweepReport: ...

    @overload
    async def sources_sanitize(
        self,
        source: SourceName | str,
        *,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> SweepReport | SanitizeDryRun: ...

    @overload
    async def sources_sanitize(
        self,
        source: SourceName | str,
        *,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> SweepReport | SanitizeDryRun: ...

    async def sources_sanitize(
        self,
        source: SourceName | str,
        *,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> SweepReport | SanitizeDryRun:
        """Run ``onetaskgraph sources sanitize``."""
        return await self._invoke(
            ["sources", "sanitize"],
            _models.SweepReport | _models.SanitizeDryRun,
            source=source,
            complete=complete,
            default_sources=default_sources,
            dry_run=dry_run,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def sources_status_options(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> StatusOptionsReport: ...

    @overload
    async def sources_status_options(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> StatusOptionsReport | SanitizeDryRun: ...

    @overload
    async def sources_status_options(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> StatusOptionsReport | SanitizeDryRun: ...

    async def sources_status_options(
        self,
        source: SourceName | str,
        *,
        apply: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> StatusOptionsReport | SanitizeDryRun:
        """Run ``onetaskgraph sources status-options``."""
        return await self._invoke(
            ["sources", "status-options"],
            _models.StatusOptionsReport | _models.SanitizeDryRun,
            source=source,
            apply=apply,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def task_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers: ...

    @overload
    async def task_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers | SanitizeDryRun: ...

    @overload
    async def task_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers | SanitizeDryRun: ...

    async def task_answers(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TemplateAnswers | SanitizeDryRun:
        """Run ``onetaskgraph task answers``."""
        return await self._invoke(
            ["task", "answers"],
            _models.TemplateAnswers | _models.SanitizeDryRun,
            id=id,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def task_assets(
        self,
        id: GlobalId | str,
        output_dir: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskAssetNames: ...

    @overload
    async def task_assets(
        self,
        id: GlobalId | str,
        output_dir: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskAssetNames | SanitizeDryRun: ...

    @overload
    async def task_assets(
        self,
        id: GlobalId | str,
        output_dir: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskAssetNames | SanitizeDryRun: ...

    async def task_assets(
        self,
        id: GlobalId | str,
        output_dir: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskAssetNames | SanitizeDryRun:
        """Run ``onetaskgraph task assets``."""
        return await self._invoke(
            ["task", "assets"],
            _models.TaskAssetNames | _models.SanitizeDryRun,
            id=id,
            output_dir=output_dir,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def task_comment_add(
        self,
        id: GlobalId | str,
        *,
        author: str | None = None,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> CommentWritten: ...

    @overload
    async def task_comment_add(
        self,
        id: GlobalId | str,
        *,
        author: str | None = None,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> CommentWritten | SanitizeDryRun: ...

    @overload
    async def task_comment_add(
        self,
        id: GlobalId | str,
        *,
        author: str | None = None,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> CommentWritten | SanitizeDryRun: ...

    async def task_comment_add(
        self,
        id: GlobalId | str,
        *,
        author: str | None = None,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> CommentWritten | SanitizeDryRun:
        """Run ``onetaskgraph task comment add``."""
        return await self._invoke(
            ["task", "comment", "add"],
            _models.CommentWritten | _models.SanitizeDryRun,
            id=id,
            author=author,
            body_file=body_file,
            default_sources=default_sources,
            expect=expect,
            if_version=if_version,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("task_comment_add", "term_scope", term_scope),
            stdin=body,
        )

    @overload
    async def task_comment_delete(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> DeletedComment: ...

    @overload
    async def task_comment_delete(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> DeletedComment | SanitizeDryRun: ...

    @overload
    async def task_comment_delete(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> DeletedComment | SanitizeDryRun: ...

    async def task_comment_delete(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> DeletedComment | SanitizeDryRun:
        """Run ``onetaskgraph task comment delete``."""
        return await self._invoke(
            ["task", "comment", "delete"],
            _models.DeletedComment | _models.SanitizeDryRun,
            id=id,
            comment_id=comment_id,
            default_sources=default_sources,
            expect=expect,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def task_comment_edit(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> CommentWritten: ...

    @overload
    async def task_comment_edit(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> CommentWritten | SanitizeDryRun: ...

    @overload
    async def task_comment_edit(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> CommentWritten | SanitizeDryRun: ...

    async def task_comment_edit(
        self,
        id: GlobalId | str,
        comment_id: str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
    ) -> CommentWritten | SanitizeDryRun:
        """Run ``onetaskgraph task comment edit``."""
        return await self._invoke(
            ["task", "comment", "edit"],
            _models.CommentWritten | _models.SanitizeDryRun,
            id=id,
            comment_id=comment_id,
            body_file=body_file,
            default_sources=default_sources,
            expect=expect,
            if_version=if_version,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("task_comment_edit", "term_scope", term_scope),
            stdin=body,
        )

    @overload
    async def task_comment_list(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> CommentList: ...

    @overload
    async def task_comment_list(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> CommentList | SanitizeDryRun: ...

    @overload
    async def task_comment_list(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> CommentList | SanitizeDryRun: ...

    async def task_comment_list(
        self,
        id: GlobalId | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> CommentList | SanitizeDryRun:
        """Run ``onetaskgraph task comment list``."""
        return await self._invoke(
            ["task", "comment", "list"],
            _models.CommentList | _models.SanitizeDryRun,
            id=id,
            default_sources=default_sources,
            page_size=page_size,
            pin=pin,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def task_content_set(
        self,
        id: GlobalId | str,
        file: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> TaskContentSet: ...

    @overload
    async def task_content_set(
        self,
        id: GlobalId | str,
        file: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> TaskContentSet | SanitizeDryRun: ...

    @overload
    async def task_content_set(
        self,
        id: GlobalId | str,
        file: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> TaskContentSet | SanitizeDryRun: ...

    async def task_content_set(
        self,
        id: GlobalId | str,
        file: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> TaskContentSet | SanitizeDryRun:
        """Run ``onetaskgraph task content set``."""
        return await self._invoke(
            ["task", "content", "set"],
            _models.TaskContentSet | _models.SanitizeDryRun,
            id=id,
            file=file,
            default_sources=default_sources,
            expect=expect,
            if_version=if_version,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("task_content_set", "term_scope", term_scope),
        )

    @overload
    async def task_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        commented_since: str | None = None,
        comments: bool | None = None,
        create: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        match_by: str | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_link: bool | None = None,
        otherwise: Literal["refuse", "preserve-destination"] | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
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
        status_policy: Literal["source", "preserve-destination"] | None = None,
        target_task: GlobalId | str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
        when_destination_priority: list[Literal["none", "urgent", "high", "medium", "low"]]
        | tuple[Literal["none", "urgent", "high", "medium", "low"], ...]
        | None = None,
        when_destination_status: list[
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
    ) -> CopyReport: ...

    @overload
    async def task_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        commented_since: str | None = None,
        comments: bool | None = None,
        create: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        match_by: str | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_link: bool | None = None,
        otherwise: Literal["refuse", "preserve-destination"] | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
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
        status_policy: Literal["source", "preserve-destination"] | None = None,
        target_task: GlobalId | str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
        when_destination_priority: list[Literal["none", "urgent", "high", "medium", "low"]]
        | tuple[Literal["none", "urgent", "high", "medium", "low"], ...]
        | None = None,
        when_destination_status: list[
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
    ) -> CopyReport | SanitizeDryRun: ...

    @overload
    async def task_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        commented_since: str | None = None,
        comments: bool | None = None,
        create: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        match_by: str | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_link: bool | None = None,
        otherwise: Literal["refuse", "preserve-destination"] | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
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
        status_policy: Literal["source", "preserve-destination"] | None = None,
        target_task: GlobalId | str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
        when_destination_priority: list[Literal["none", "urgent", "high", "medium", "low"]]
        | tuple[Literal["none", "urgent", "high", "medium", "low"], ...]
        | None = None,
        when_destination_status: list[
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
    ) -> CopyReport | SanitizeDryRun: ...

    async def task_copy(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        commented_since: str | None = None,
        comments: bool | None = None,
        create: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        match_by: str | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_link: bool | None = None,
        otherwise: Literal["refuse", "preserve-destination"] | None = None,
        page_size: int | None = None,
        recreate: bool | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
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
        status_policy: Literal["source", "preserve-destination"] | None = None,
        target_task: GlobalId | str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        to: str | None = None,
        when_destination_priority: list[Literal["none", "urgent", "high", "medium", "low"]]
        | tuple[Literal["none", "urgent", "high", "medium", "low"], ...]
        | None = None,
        when_destination_status: list[
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
    ) -> CopyReport | SanitizeDryRun:
        """Run ``onetaskgraph task copy``."""
        return await self._invoke(
            ["task", "copy"],
            _models.CopyReport | _models.SanitizeDryRun,
            ids=ids,
            commented_since=commented_since,
            comments=comments,
            create=create,
            default_sources=default_sources,
            dry_run=dry_run,
            expect=expect,
            if_version=if_version,
            match_by=match_by,
            metadata=_strings("task_copy", "metadata", metadata),
            no_link=no_link,
            otherwise=otherwise,
            page_size=page_size,
            recreate=recreate,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            source=source,
            status=status,
            status_policy=status_policy,
            target_task=target_task,
            term_scope=_strings("task_copy", "term_scope", term_scope),
            to=to,
            when_destination_priority=when_destination_priority,
            when_destination_status=when_destination_status,
        )

    @overload
    async def task_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> TaskDetail: ...

    @overload
    async def task_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> TaskDetail | SanitizeDryRun: ...

    @overload
    async def task_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> TaskDetail | SanitizeDryRun: ...

    async def task_create(
        self,
        source: SourceName | str,
        project: str,
        title: str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        body_file: str | None = None,
        classification: Literal["public", "private"] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        body: str | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> TaskDetail | SanitizeDryRun:
        """Run ``onetaskgraph task create``."""
        return await self._invoke(
            ["task", "create"],
            _models.TaskDetail | _models.SanitizeDryRun,
            source=source,
            project=project,
            title=title,
            asset=asset,
            body_file=body_file,
            classification=classification,
            default_sources=default_sources,
            delivers=delivers,
            depends_on=depends_on,
            label=label,
            metadata=_strings("task_create", "metadata", metadata),
            page_size=page_size,
            reject=reject,
            replace=replace,
            repository=_strings("task_create", "repository", repository),
            sanitize_dry_run=sanitize_dry_run,
            search_path=_strings("task_create", "search_path", search_path),
            set=set,
            status=status,
            template=template,
            template_loader=template_loader,
            term_scope=_strings("task_create", "term_scope", term_scope),
            var=_strings("task_create", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin(
                "task_create",
                body,
                answers,
                template is not None or template_loader is not None or body_file is not None,
            ),
        )

    @overload
    async def task_deps(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["depends-on", "depended-on-by"] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge: ...

    @overload
    async def task_deps(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["depends-on", "depended-on-by"] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge | SanitizeDryRun: ...

    @overload
    async def task_deps(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["depends-on", "depended-on-by"] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge | SanitizeDryRun: ...

    async def task_deps(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        complete: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: Literal["depends-on", "depended-on-by"] | None = None,
        explain: bool | None = None,
        limit: int | None = None,
        page: str | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedEdge | SanitizeDryRun:
        """Run ``onetaskgraph task deps``."""
        return await self._invoke(
            ["task", "deps"],
            _models.QueryResponseOfQualifiedEdge | _models.SanitizeDryRun,
            id=id,
            allow_partial=allow_partial,
            complete=complete,
            default_sources=default_sources,
            direction=direction,
            explain=explain,
            limit=limit,
            page=page,
            page_size=page_size,
            pin=pin,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
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
        members: bool | None = None,
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
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
    ) -> QueryResponseOfQualifiedTask: ...

    @overload
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
        members: bool | None = None,
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
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
    ) -> QueryResponseOfQualifiedTask | SanitizeDryRun: ...

    @overload
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
        members: bool | None = None,
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
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
    ) -> QueryResponseOfQualifiedTask | SanitizeDryRun: ...

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
        members: bool | None = None,
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
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
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
    ) -> QueryResponseOfQualifiedTask | SanitizeDryRun:
        """Run ``onetaskgraph task list``."""
        return await self._invoke(
            ["task", "list"],
            _models.QueryResponseOfQualifiedTask | _models.SanitizeDryRun,
            allow_partial=allow_partial,
            commented_since=commented_since,
            default_sources=default_sources,
            explain=explain,
            in_=in_,
            label=label,
            limit=limit,
            members=members,
            metadata=_strings("task_list", "metadata", metadata),
            no_project=no_project,
            not_label=not_label,
            origin=origin,
            page=page,
            page_size=page_size,
            priority=priority,
            project=project,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            search=search,
            set=set,
            source=source,
            status=status,
        )

    @overload
    async def task_locate(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        to: str,
        default_sources: list[str] | tuple[str, ...] | None = None,
        no_counterpart: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        target_native: str | None = None,
    ) -> LocateReport: ...

    @overload
    async def task_locate(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        to: str,
        default_sources: list[str] | tuple[str, ...] | None = None,
        no_counterpart: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        target_native: str | None = None,
    ) -> LocateReport | SanitizeDryRun: ...

    @overload
    async def task_locate(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        to: str,
        default_sources: list[str] | tuple[str, ...] | None = None,
        no_counterpart: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        target_native: str | None = None,
    ) -> LocateReport | SanitizeDryRun: ...

    async def task_locate(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        to: str,
        default_sources: list[str] | tuple[str, ...] | None = None,
        no_counterpart: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        target_native: str | None = None,
    ) -> LocateReport | SanitizeDryRun:
        """Run ``onetaskgraph task locate``."""
        return await self._invoke(
            ["task", "locate"],
            _models.LocateReport | _models.SanitizeDryRun,
            ids=ids,
            to=to,
            default_sources=default_sources,
            no_counterpart=no_counterpart,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            target_native=target_native,
        )

    @overload
    async def task_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet: ...

    @overload
    async def task_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet | SanitizeDryRun: ...

    @overload
    async def task_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet | SanitizeDryRun: ...

    async def task_metadata_set(
        self,
        id: GlobalId | str,
        key: str,
        value: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> MetadataSet | SanitizeDryRun:
        """Run ``onetaskgraph task metadata set``."""
        return await self._invoke(
            ["task", "metadata", "set"],
            _models.MetadataSet | _models.SanitizeDryRun,
            id=id,
            key=key,
            value=value,
            default_sources=default_sources,
            expect=expect,
            if_version=if_version,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("task_metadata_set", "term_scope", term_scope),
        )

    @overload
    async def task_priority_set(
        self,
        id: GlobalId | str,
        priority: Priority | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        when_priority: list[Literal["none", "urgent", "high", "medium", "low"]]
        | tuple[Literal["none", "urgent", "high", "medium", "low"], ...]
        | None = None,
    ) -> TaskPrioritySet: ...

    @overload
    async def task_priority_set(
        self,
        id: GlobalId | str,
        priority: Priority | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        when_priority: list[Literal["none", "urgent", "high", "medium", "low"]]
        | tuple[Literal["none", "urgent", "high", "medium", "low"], ...]
        | None = None,
    ) -> TaskPrioritySet | SanitizeDryRun: ...

    @overload
    async def task_priority_set(
        self,
        id: GlobalId | str,
        priority: Priority | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        when_priority: list[Literal["none", "urgent", "high", "medium", "low"]]
        | tuple[Literal["none", "urgent", "high", "medium", "low"], ...]
        | None = None,
    ) -> TaskPrioritySet | SanitizeDryRun: ...

    async def task_priority_set(
        self,
        id: GlobalId | str,
        priority: Priority | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        when_priority: list[Literal["none", "urgent", "high", "medium", "low"]]
        | tuple[Literal["none", "urgent", "high", "medium", "low"], ...]
        | None = None,
    ) -> TaskPrioritySet | SanitizeDryRun:
        """Run ``onetaskgraph task priority set``."""
        return await self._invoke(
            ["task", "priority", "set"],
            _models.TaskPrioritySet | _models.SanitizeDryRun,
            id=id,
            priority=priority,
            default_sources=default_sources,
            expect=expect,
            if_version=if_version,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("task_priority_set", "term_scope", term_scope),
            when_priority=when_priority,
        )

    @overload
    async def task_project_human(
        self,
        input: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> HumanTaskProjected: ...

    @overload
    async def task_project_human(
        self,
        input: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> HumanTaskProjected | SanitizeDryRun: ...

    @overload
    async def task_project_human(
        self,
        input: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> HumanTaskProjected | SanitizeDryRun: ...

    async def task_project_human(
        self,
        input: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> HumanTaskProjected | SanitizeDryRun:
        """Run ``onetaskgraph task project-human``."""
        return await self._invoke(
            ["task", "project-human"],
            _models.HumanTaskProjected | _models.SanitizeDryRun,
            input=input,
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def task_render(
        self,
        id: GlobalId | str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated: ...

    @overload
    async def task_render(
        self,
        id: GlobalId | str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated | SanitizeDryRun: ...

    @overload
    async def task_render(
        self,
        id: GlobalId | str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated | SanitizeDryRun: ...

    async def task_render(
        self,
        id: GlobalId | str,
        *,
        asset: list[str] | tuple[str, ...] | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template: str | None = None,
        template_loader: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        unset: list[str] | tuple[str, ...] | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> Regenerated | SanitizeDryRun:
        """Run ``onetaskgraph task render``."""
        return await self._invoke(
            ["task", "render"],
            _models.Regenerated | _models.SanitizeDryRun,
            id=id,
            asset=asset,
            default_sources=default_sources,
            dry_run=dry_run,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            search_path=_strings("task_render", "search_path", search_path),
            set=set,
            template=template,
            template_loader=template_loader,
            term_scope=_strings("task_render", "term_scope", term_scope),
            unset=_strings("task_render", "unset", unset),
            var=_strings("task_render", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin("task_render", None, answers),
        )

    @overload
    async def task_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetail: ...

    @overload
    async def task_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetail | SanitizeDryRun: ...

    @overload
    async def task_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetail | SanitizeDryRun: ...

    async def task_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        pin: VersionToken | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetail | SanitizeDryRun:
        """Run ``onetaskgraph task show``."""
        return await self._invoke(
            ["task", "show"],
            _models.TaskDetail | _models.SanitizeDryRun,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            no_comments=no_comments,
            page_size=page_size,
            pin=pin,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def task_show_many(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        pin: list[str] | tuple[str, ...] | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetails: ...

    @overload
    async def task_show_many(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        pin: list[str] | tuple[str, ...] | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetails | SanitizeDryRun: ...

    @overload
    async def task_show_many(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        pin: list[str] | tuple[str, ...] | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetails | SanitizeDryRun: ...

    async def task_show_many(
        self,
        ids: list[GlobalId | str] | tuple[GlobalId | str, ...],
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        no_comments: bool | None = None,
        page_size: int | None = None,
        pin: list[str] | tuple[str, ...] | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> TaskDetails | SanitizeDryRun:
        """Run ``onetaskgraph task show-many``."""
        return await self._invoke(
            ["task", "show-many"],
            _models.TaskDetails | _models.SanitizeDryRun,
            ids=ids,
            default_sources=default_sources,
            no_comments=no_comments,
            page_size=page_size,
            pin=pin,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
        )

    @overload
    async def task_status_complete(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        issue: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> IssuesCompleted: ...

    @overload
    async def task_status_complete(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        issue: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> IssuesCompleted | SanitizeDryRun: ...

    @overload
    async def task_status_complete(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        issue: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> IssuesCompleted | SanitizeDryRun: ...

    async def task_status_complete(
        self,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        issue: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> IssuesCompleted | SanitizeDryRun:
        """Run ``onetaskgraph task status complete``."""
        return await self._invoke(
            ["task", "status", "complete"],
            _models.IssuesCompleted | _models.SanitizeDryRun,
            default_sources=default_sources,
            dry_run=dry_run,
            issue=issue,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            source=source,
            term_scope=_strings("task_status_complete", "term_scope", term_scope),
        )

    @overload
    async def task_status_reconcile(
        self,
        source: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> StatusesReconciled: ...

    @overload
    async def task_status_reconcile(
        self,
        source: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> StatusesReconciled | SanitizeDryRun: ...

    @overload
    async def task_status_reconcile(
        self,
        source: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> StatusesReconciled | SanitizeDryRun: ...

    async def task_status_reconcile(
        self,
        source: str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        dry_run: bool | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> StatusesReconciled | SanitizeDryRun:
        """Run ``onetaskgraph task status reconcile``."""
        return await self._invoke(
            ["task", "status", "reconcile"],
            _models.StatusesReconciled | _models.SanitizeDryRun,
            source=source,
            default_sources=default_sources,
            dry_run=dry_run,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("task_status_reconcile", "term_scope", term_scope),
        )

    @overload
    async def task_status_set(
        self,
        id: GlobalId | str,
        category: StatusCategory | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> TaskStatusSet: ...

    @overload
    async def task_status_set(
        self,
        id: GlobalId | str,
        category: StatusCategory | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> TaskStatusSet | SanitizeDryRun: ...

    @overload
    async def task_status_set(
        self,
        id: GlobalId | str,
        category: StatusCategory | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> TaskStatusSet | SanitizeDryRun: ...

    async def task_status_set(
        self,
        id: GlobalId | str,
        category: StatusCategory | str,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
    ) -> TaskStatusSet | SanitizeDryRun:
        """Run ``onetaskgraph task status set``."""
        return await self._invoke(
            ["task", "status", "set"],
            _models.TaskStatusSet | _models.SanitizeDryRun,
            id=id,
            category=category,
            default_sources=default_sources,
            expect=expect,
            if_version=if_version,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            term_scope=_strings("task_status_set", "term_scope", term_scope),
        )

    @overload
    async def task_update(
        self,
        id: GlobalId | str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_delivers: bool | None = None,
        no_depends_on: bool | None = None,
        no_repositories: bool | None = None,
        page_size: int | None = None,
        priority: Literal["none", "urgent", "high", "medium", "low"] | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        remove_metadata: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        status_name: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        title: str | None = None,
    ) -> TaskUpdated: ...

    @overload
    async def task_update(
        self,
        id: GlobalId | str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_delivers: bool | None = None,
        no_depends_on: bool | None = None,
        no_repositories: bool | None = None,
        page_size: int | None = None,
        priority: Literal["none", "urgent", "high", "medium", "low"] | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        remove_metadata: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        status_name: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        title: str | None = None,
    ) -> TaskUpdated | SanitizeDryRun: ...

    @overload
    async def task_update(
        self,
        id: GlobalId | str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_delivers: bool | None = None,
        no_depends_on: bool | None = None,
        no_repositories: bool | None = None,
        page_size: int | None = None,
        priority: Literal["none", "urgent", "high", "medium", "low"] | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        remove_metadata: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        status_name: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        title: str | None = None,
    ) -> TaskUpdated | SanitizeDryRun: ...

    async def task_update(
        self,
        id: GlobalId | str,
        *,
        body_file: str | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        delivers: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        depends_on: list[GlobalId | str] | tuple[GlobalId | str, ...] | None = None,
        expect: VersionToken | None = None,
        if_version: VersionToken | None = None,
        metadata: list[str] | tuple[str, ...] | None = None,
        no_delivers: bool | None = None,
        no_depends_on: bool | None = None,
        no_repositories: bool | None = None,
        page_size: int | None = None,
        priority: Literal["none", "urgent", "high", "medium", "low"] | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        remove_metadata: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        repository: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        status: Literal[
            "draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"
        ]
        | None = None,
        status_name: str | None = None,
        term_scope: list[str] | tuple[str, ...] | None = None,
        title: str | None = None,
    ) -> TaskUpdated | SanitizeDryRun:
        """Run ``onetaskgraph task update``."""
        return await self._invoke(
            ["task", "update"],
            _models.TaskUpdated | _models.SanitizeDryRun,
            id=id,
            body_file=body_file,
            default_sources=default_sources,
            delivers=delivers,
            depends_on=depends_on,
            expect=expect,
            if_version=if_version,
            metadata=_strings("task_update", "metadata", metadata),
            no_delivers=no_delivers,
            no_depends_on=no_depends_on,
            no_repositories=no_repositories,
            page_size=page_size,
            priority=priority,
            reject=reject,
            remove_metadata=_strings("task_update", "remove_metadata", remove_metadata),
            replace=replace,
            repository=_strings("task_update", "repository", repository),
            sanitize_dry_run=sanitize_dry_run,
            set=set,
            status=status,
            status_name=status_name,
            term_scope=_strings("task_update", "term_scope", term_scope),
            title=title,
        )

    @overload
    async def template_render(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> RenderedTemplate: ...

    @overload
    async def template_render(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> RenderedTemplate | SanitizeDryRun: ...

    @overload
    async def template_render(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> RenderedTemplate | SanitizeDryRun: ...

    async def template_render(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
        var: list[str] | tuple[str, ...] | None = None,
        answers: Mapping[str, JsonValue] | None = None,
    ) -> RenderedTemplate | SanitizeDryRun:
        """Run ``onetaskgraph template render``."""
        return await self._invoke(
            ["template", "render"],
            _models.RenderedTemplate | _models.SanitizeDryRun,
            file=_template_file("template_render", file),
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            search_path=_strings("template_render", "search_path", search_path),
            set=set,
            template_loader=template_loader,
            var=_strings("template_render", "var", var),
            answers=None if answers is None else "-",
            stdin=_stdin("template_render", None, answers),
        )

    @overload
    async def template_variables(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[False] | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
    ) -> TemplateVariables: ...

    @overload
    async def template_variables(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: Literal[True],
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
    ) -> TemplateVariables | SanitizeDryRun: ...

    @overload
    async def template_variables(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
    ) -> TemplateVariables | SanitizeDryRun: ...

    async def template_variables(
        self,
        file: str | None = None,
        *,
        default_sources: list[str] | tuple[str, ...] | None = None,
        page_size: int | None = None,
        reject: list[str] | tuple[str, ...] | None = None,
        replace: list[str] | tuple[str, ...] | None = None,
        sanitize_dry_run: bool | None = None,
        search_path: list[str] | tuple[str, ...] | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        template_loader: str | None = None,
    ) -> TemplateVariables | SanitizeDryRun:
        """Run ``onetaskgraph template variables``."""
        return await self._invoke(
            ["template", "variables"],
            _models.TemplateVariables | _models.SanitizeDryRun,
            file=_template_file("template_variables", file),
            default_sources=default_sources,
            page_size=page_size,
            reject=reject,
            replace=replace,
            sanitize_dry_run=sanitize_dry_run,
            search_path=_strings("template_variables", "search_path", search_path),
            set=set,
            template_loader=template_loader,
        )
