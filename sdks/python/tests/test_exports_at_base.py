"""Every name the SDK exported before its models became lazy is still exported the same way.

`exports_at_base.json` beside this file was written from the commit it names, before the
change: for `onetaskgraph_sdk._generated.models` its `__all__`, and for `onetaskgraph_sdk`,
which had no `__all__` then, every public name `from onetaskgraph_sdk import *` bound — its
star-import set, less the `client` submodule, which is a module rather than an export and
stays importable as one. `models` lists the names that were pydantic model classes there.
"""

from __future__ import annotations

import importlib
import json
from pathlib import Path

import pytest
from pydantic import BaseModel

BASE = json.loads(Path(__file__).with_name("exports_at_base.json").read_text(encoding="utf-8"))
MODULES = ["onetaskgraph_sdk", "onetaskgraph_sdk._generated.models"]


def test_the_committed_list_is_the_whole_base_inventory() -> None:
    """The list names real identifiers for both modules, so no entry stands in for one."""
    assert sorted(key for key in BASE if key != "base") == MODULES
    for module in MODULES:
        exports, models = BASE[module]["exports"], BASE[module]["models"]
        assert len(exports) >= 66, f"{module} lists only {len(exports)} names"
        assert all(name.isidentifier() and name[0].isupper() for name in exports), exports
        assert exports == sorted(set(exports))
        assert set(models) < set(exports)


@pytest.mark.parametrize("module", MODULES)
def test_every_base_export_is_still_exported_from_the_same_module(module: str) -> None:
    """Importable by name, listed in `__all__` and `dir()`, one class, a model where it was."""
    imported = importlib.import_module(module)
    for name in BASE[module]["exports"]:
        namespace: dict[str, object] = {}
        # The import statement itself is under test, not only attribute access.
        exec(f"from {module} import {name}", namespace)
        value = namespace[name]
        assert getattr(imported, name) is value, f"{module}.{name} is a second object"
        assert name in imported.__all__, f"{module}.__all__ lost {name}"
        assert name in dir(imported), f"dir({module}) lost {name}"
        if name in BASE[module]["models"]:
            assert isinstance(value, type) and issubclass(value, BaseModel), (
                f"{module}.{name} was a pydantic model and is now {value!r}"
            )
