"""Generated typed client methods. Do not edit."""

from __future__ import annotations

from .models import (
    EffectiveConfig,
    QueryResponseOfQualifiedEdge,
    QueryResponseOfQualifiedLabel,
    QueryResponseOfQualifiedProject,
    QueryResponseOfQualifiedTask,
    QueryResponseOfSearchHit,
    SourceListing,
)

type Option = str | int | bool | list[str] | tuple[str, ...] | None


class GeneratedClient:
    """Methods generated from the binary command surface."""

    async def _invoke[T](self, command: list[str], model: object, **options: object) -> T:
        raise NotImplementedError

    async def config_show(
        self, *, default_sources: Option = None, page_size: Option = None, set: Option = None
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
        allow_partial: Option = None,
        default_sources: Option = None,
        explain: Option = None,
        limit: Option = None,
        page: Option = None,
        page_size: Option = None,
        set: Option = None,
        source: Option = None,
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
        id: str,
        *,
        allow_partial: Option = None,
        default_sources: Option = None,
        direction: Option = None,
        explain: Option = None,
        limit: Option = None,
        page: Option = None,
        page_size: Option = None,
        set: Option = None,
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
        allow_partial: Option = None,
        default_sources: Option = None,
        explain: Option = None,
        in_: Option = None,
        label: Option = None,
        limit: Option = None,
        not_label: Option = None,
        page: Option = None,
        page_size: Option = None,
        search: Option = None,
        set: Option = None,
        source: Option = None,
        status: Option = None,
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
        id: str,
        *,
        allow_partial: Option = None,
        default_sources: Option = None,
        explain: Option = None,
        page_size: Option = None,
        set: Option = None,
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
        allow_partial: Option = None,
        default_sources: Option = None,
        explain: Option = None,
        in_: Option = None,
        kind: Option = None,
        limit: Option = None,
        page: Option = None,
        page_size: Option = None,
        set: Option = None,
        source: Option = None,
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
        self, *, default_sources: Option = None, page_size: Option = None, set: Option = None
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
        id: str,
        *,
        allow_partial: Option = None,
        default_sources: Option = None,
        direction: Option = None,
        explain: Option = None,
        limit: Option = None,
        page: Option = None,
        page_size: Option = None,
        set: Option = None,
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
        allow_partial: Option = None,
        default_sources: Option = None,
        explain: Option = None,
        in_: Option = None,
        label: Option = None,
        limit: Option = None,
        no_project: Option = None,
        not_label: Option = None,
        page: Option = None,
        page_size: Option = None,
        project: Option = None,
        search: Option = None,
        set: Option = None,
        source: Option = None,
        status: Option = None,
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
        id: str,
        *,
        allow_partial: Option = None,
        default_sources: Option = None,
        explain: Option = None,
        page_size: Option = None,
        set: Option = None,
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
