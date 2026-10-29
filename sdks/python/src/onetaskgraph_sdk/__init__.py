"""The generated, typed Python SDK for onetaskgraph.

Every response model is importable from here, and each is built only when a caller first
touches it: a process that runs two verbs builds the models of those two, not the roughly
700 classes of every root, which once cost most of a short-lived caller's start-up and exit.
"""

from typing import TYPE_CHECKING

from . import client as client
from ._generated import models as _models
from .client import Client as Client
from .client import OnetaskgraphError as OnetaskgraphError

if TYPE_CHECKING:
    from ._generated.models import *  # noqa: F403  # Schema roots define the public set.

# Not `import *` at run time: it reads every name `__all__` lists, which would build every
# model. `__all__ += <module>.__all__` is the spelling a type checker follows, so a strict
# one reads every root as re-exported from here.
__all__ = ["Client", "OnetaskgraphError", "client"]
__all__ += _models.__all__

#: The version this package publishes. `pyproject.toml`, the Cargo workspace and the
#: TypeScript package must all agree; scripts/check-workspace-config.sh reconciles them.
__version__ = "0.12.10"

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
