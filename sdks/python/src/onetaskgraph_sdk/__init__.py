"""The Python SDK for onetaskgraph.

The client surface is generated from the schema bundle `onetaskgraph schema` emits and
lands with a later change. What is here now is the version this package publishes, stated
once so the generated surface and the distribution metadata cannot disagree.
"""

__all__ = ["__version__"]

#: The version this package publishes. `pyproject.toml`, the Cargo workspace and the
#: TypeScript package must all agree; scripts/check-workspace-config.sh reconciles them.
__version__ = "0.1.0"
