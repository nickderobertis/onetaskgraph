"""The aliased schema roots are only public to a strict type checker if `__all__` names them.

`models.py` re-exports each root under the name the schema gave it, which for every
`QueryResponseOf…` and for `PageOfDocument` is not the name its generated module defines.
`from X import Y as Z` is not a re-export to a checker running mypy's strict setting, so
without `__all__` a consumer importing those names from `onetaskgraph_sdk` type-checks as
importing nothing — which is what sent consumers into `_generated` for their annotations.
These tests drive the two checkers this repository ships over a real consumer file.
"""

from __future__ import annotations

import importlib
import subprocess
import sys
from pathlib import Path

import pytest

import onetaskgraph_sdk
from onetaskgraph_sdk._generated import models

PACKAGE = Path(__file__).parents[1]
SCRIPTS = Path(sys.executable).parent
SUFFIX = ".exe" if sys.platform == "win32" else ""

CONTRACT_NAMES = (
    "QueryResponseOfQualifiedDocument",
    "QueryResponseOfQualifiedEdge",
    "QueryResponseOfQualifiedLabel",
    "QueryResponseOfQualifiedProject",
    "QueryResponseOfQualifiedTask",
    "QueryResponseOfSearchHit",
)
"""The six names a dependent of this release writes down, from issue #2280.

They are not a second spelling of the public set: the consumer below is built from
`models.__all__` itself, and the test that reads this tuple reconciles it against that set,
so a root renamed out from under a dependent fails here rather than at its next upgrade.
"""

PUBLISHED = sorted(name for name in vars(models) if not name.startswith("_"))
"""Every root `models.py` binds, read from the module rather than from its `__all__`.

From the bindings, because `__all__` is what is under test: a consumer built from it could
not fail when a root fell out of it, since the root would leave the consumer too.
"""

CONSUMER = (
    "from onetaskgraph_sdk import (\n"
    + "".join(f"    {name},\n" for name in PUBLISHED)
    + ")\n\n"
    + "".join(f"model_{index}: type[{name}] = {name}\n" for index, name in enumerate(PUBLISHED))
)
"""A consumer that imports every published name from the package root and annotates with it.

Every name rather than the six above, because the aliasing is what this is about and
`PageOfDocument` is aliased by a rule of its own.
"""


def tool(name: str) -> Path:
    """Resolve a checker from the environment running these tests, refusing when it is absent."""
    executable = SCRIPTS / f"{name}{SUFFIX}"
    if not executable.is_file():
        pytest.fail(f"{executable} is missing; run `uv sync --frozen` from {PACKAGE}")
    return executable


def run_mypy(source: Path, cache: Path) -> subprocess.CompletedProcess[str]:
    """Type-check one file under mypy's strict setting, caching outside the checkout.

    `--strict` is what turns implicit re-export off, which is the whole subject here; the
    cache is directed at the temporary directory so a test run leaves no `.mypy_cache`
    behind. The output is decoded as UTF-8 rather than in the platform's code page, which
    on the Windows runner is not the encoding a subprocess writes.
    """
    return subprocess.run(
        [
            str(tool("mypy")),
            "--strict",
            "--no-incremental",
            "--cache-dir",
            str(cache),
            str(source),
        ],
        cwd=source.parent,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )


def run_ty(source: Path) -> subprocess.CompletedProcess[str]:
    """Type-check one file under `ty`, against the interpreter these tests run in."""
    return subprocess.run(
        [str(tool("ty")), "check", "--python", sys.executable, str(source)],
        cwd=source.parent,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )


def test_a_strict_consumer_imports_every_published_name_from_the_package(tmp_path: Path) -> None:
    """Every name the SDK publishes is importable, and annotatable, from `onetaskgraph_sdk`."""
    source = tmp_path / "consumer.py"
    source.write_text(CONSUMER, encoding="utf-8")

    checked = run_mypy(source, tmp_path / "mypy-cache")
    assert checked.returncode == 0, (
        "mypy --strict refused a consumer importing the SDK's own published names:\n"
        f"{checked.stdout}{checked.stderr}"
        f"add every name to __all__ in {models.__file__}, which generate.py writes"
    )

    checked = run_ty(source)
    assert checked.returncode == 0, (
        f"ty refused the same consumer:\n{checked.stdout}{checked.stderr}"
    )


def test_a_name_the_package_does_not_export_is_refused_by_both_checkers(tmp_path: Path) -> None:
    """Both checkers really read this file, so the run above is not passing vacuously."""
    source = tmp_path / "unknown.py"
    source.write_text("from onetaskgraph_sdk import QueryResponseOfNothing\n", encoding="utf-8")

    checked = run_mypy(source, tmp_path / "mypy-cache")
    assert checked.returncode != 0, f"mypy --strict accepted an unexported name:\n{checked.stdout}"

    checked = run_ty(source)
    assert checked.returncode != 0, f"ty accepted an unexported name:\n{checked.stdout}"


def test_mypy_runs_with_implicit_re_export_off(tmp_path: Path) -> None:
    """`--strict` is in force, which is the only setting under which `__all__` is needed.

    `client.py` imports `Path` for its own use and never re-exports it. A checker with
    implicit re-export left on accepts that import, and would have accepted the aliased
    roots without `__all__` too — so the run above would prove nothing.
    """
    source = tmp_path / "implicit.py"
    source.write_text("from onetaskgraph_sdk.client import Path\n", encoding="utf-8")

    checked = run_mypy(source, tmp_path / "mypy-cache")
    assert checked.returncode != 0, (
        f"mypy accepted an implicit re-export, so it did not run strict:\n{checked.stdout}"
    )
    assert "explicitly export" in checked.stdout, (
        f"mypy refused for some other reason than implicit re-export:\n{checked.stdout}"
    )


def test_every_published_name_is_the_class_its_generated_module_defines() -> None:
    """The name the package advertises is the model itself, not a stand-in or a second class.

    The module each root lives in is read from the generator's own `camel_to_snake` rather
    than spelled again here, so this reconciles against `generate.py` instead of restating
    it — the alias a root is published under is exactly what these tests are about.
    """
    sys.path.insert(0, str(PACKAGE))
    import generate

    for name in PUBLISHED:
        module = importlib.import_module(
            f"onetaskgraph_sdk._generated.{generate.camel_to_snake(name)}"
        )
        published = getattr(onetaskgraph_sdk, name)
        assert isinstance(published, type), f"{name} is published as {published!r}, not a class"
        assert published.__module__ == module.__name__, (
            f"onetaskgraph_sdk.{name} is defined in {published.__module__}, "
            f"not in {module.__name__}"
        )


def test_dunder_all_names_every_root_models_binds_and_every_name_a_dependent_writes() -> None:
    """A root cannot fall out of the public set, nor out from under a named dependent."""
    assert sorted(models.__all__) == PUBLISHED, (
        "models.py binds a public name its __all__ omits (or the reverse); "
        "generate.py writes both from the same root list"
    )
    missing = sorted(set(CONTRACT_NAMES) - set(models.__all__))
    assert not missing, f"the SDK stopped publishing {missing}, which a dependent imports by name"
