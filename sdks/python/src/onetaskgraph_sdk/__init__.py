"""The generated, typed Python SDK for onetaskgraph."""

from ._generated.models import *  # noqa: F403
from .client import Client as Client
from .client import OnetaskgraphError as OnetaskgraphError

__all__ = ["Client", "OnetaskgraphError", "__version__"]

#: The version this package publishes. `pyproject.toml`, the Cargo workspace and the
#: TypeScript package must all agree; scripts/check-workspace-config.sh reconciles them.
__version__ = "0.1.0"
