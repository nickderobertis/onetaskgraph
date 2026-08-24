"""Generated typed client methods. Do not edit."""

from __future__ import annotations

from .models import (
    EffectiveConfig,
    GlobalId,
    QueryResponseOfQualifiedEdge,
    QueryResponseOfQualifiedLabel,
    QueryResponseOfQualifiedProject,
    QueryResponseOfQualifiedTask,
    QueryResponseOfSearchHit,
    SourceListing,
)


class GeneratedClient:
    """Methods generated from the binary command surface."""

    async def _invoke[T](self, command: list[str], model: object, **options: object) -> T:
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

    async def project_deps(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: str | None = None,
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
        in_: str | None = None,
        label: list[str] | tuple[str, ...] | None = None,
        limit: int | None = None,
        not_label: list[str] | tuple[str, ...] | None = None,
        page: str | None = None,
        page_size: int | None = None,
        search: str | None = None,
        set: list[str] | tuple[str, ...] | None = None,
        source: list[str] | tuple[str, ...] | None = None,
        status: list[str] | tuple[str, ...] | None = None,
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
        in_: str | None = None,
        kind: str | None = None,
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

    async def task_deps(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        direction: str | None = None,
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
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        in_: str | None = None,
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
        status: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedTask:
        """Run ``onetaskgraph task list``."""
        return await self._invoke(
            ["task", "list"],
            QueryResponseOfQualifiedTask,
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
            status=status,
        )

    async def task_show(
        self,
        id: GlobalId | str,
        *,
        allow_partial: bool | None = None,
        default_sources: list[str] | tuple[str, ...] | None = None,
        explain: bool | None = None,
        page_size: int | None = None,
        set: list[str] | tuple[str, ...] | None = None,
    ) -> QueryResponseOfQualifiedTask:
        """Run ``onetaskgraph task show``."""
        return await self._invoke(
            ["task", "show"],
            QueryResponseOfQualifiedTask,
            id=id,
            allow_partial=allow_partial,
            default_sources=default_sources,
            explain=explain,
            page_size=page_size,
            set=set,
        )
