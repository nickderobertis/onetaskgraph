"""The generated client and models; each model is imported when first used."""

from typing import TYPE_CHECKING

from . import models as _models
from .client import GeneratedClient as GeneratedClient

if TYPE_CHECKING:
    from .models import *  # noqa: F403  # Schema roots define the public set.

__all__ = ["GeneratedClient"]
__all__ += _models.__all__

if not TYPE_CHECKING:

    def __getattr__(name: str) -> object:
        """Resolve a schema root through `models`, which imports its module alone."""
        try:
            value = getattr(_models, name)
        except AttributeError:
            message = f"module {__name__!r} has no attribute {name!r}"
            raise AttributeError(message) from None
        globals()[name] = value
        return value

    def __dir__() -> list[str]:
        """Every name bound here and every root, imported or not."""
        return sorted({*globals(), *__all__})
