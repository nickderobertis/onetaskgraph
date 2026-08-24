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


class GeneratedClient:
    """Methods generated from the binary command surface."""

    def _invoke[T](self, command: list[str], model: object, **options: object) -> T:
        raise NotImplementedError

    def config_show(self, **options: object) -> EffectiveConfig:
        """Run ``onetaskgraph config show``."""
        return self._invoke(["config", "show"], EffectiveConfig, **options)

    def label_list(self, **options: object) -> QueryResponseOfQualifiedLabel:
        """Run ``onetaskgraph label list``."""
        return self._invoke(["label", "list"], QueryResponseOfQualifiedLabel, **options)

    def project_deps(self, **options: object) -> QueryResponseOfQualifiedEdge:
        """Run ``onetaskgraph project deps``."""
        return self._invoke(["project", "deps"], QueryResponseOfQualifiedEdge, **options)

    def project_list(self, **options: object) -> QueryResponseOfQualifiedProject:
        """Run ``onetaskgraph project list``."""
        return self._invoke(["project", "list"], QueryResponseOfQualifiedProject, **options)

    def project_show(self, **options: object) -> QueryResponseOfQualifiedProject:
        """Run ``onetaskgraph project show``."""
        return self._invoke(["project", "show"], QueryResponseOfQualifiedProject, **options)

    def search(self, **options: object) -> QueryResponseOfSearchHit:
        """Run ``onetaskgraph search``."""
        return self._invoke(["search"], QueryResponseOfSearchHit, **options)

    def sources_list(self, **options: object) -> list[SourceListing]:
        """Run ``onetaskgraph sources list``."""
        return self._invoke(["sources", "list"], list[SourceListing], **options)

    def task_deps(self, **options: object) -> QueryResponseOfQualifiedEdge:
        """Run ``onetaskgraph task deps``."""
        return self._invoke(["task", "deps"], QueryResponseOfQualifiedEdge, **options)

    def task_list(self, **options: object) -> QueryResponseOfQualifiedTask:
        """Run ``onetaskgraph task list``."""
        return self._invoke(["task", "list"], QueryResponseOfQualifiedTask, **options)

    def task_show(self, **options: object) -> QueryResponseOfQualifiedTask:
        """Run ``onetaskgraph task show``."""
        return self._invoke(["task", "show"], QueryResponseOfQualifiedTask, **options)
