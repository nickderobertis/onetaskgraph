"""The built wheel is what a type checker reads, so the typed marker is held there."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

PACKAGE = Path(__file__).parents[1]
SOURCE = PACKAGE / "src" / "onetaskgraph_sdk"
MARKER = "py.typed"
CLASSIFIER = "Classifier: Typing :: Typed"


def test_the_wheel_carries_the_typed_marker_and_the_typed_classifier(tmp_path: Path) -> None:
    """A wheel without `py.typed` is untyped to every consumer, whatever the tree holds.

    `release.yml` builds this distribution with `uv build sdks/python` — the sdist, then the
    wheel from it — so the wheel here is built the same way rather than `--wheel` alone: a
    marker the sdist left out would be missing from what PyPI serves and present in a wheel
    built straight from the tree. The wheel is read as the zip it is; nothing here looks at
    the source tree.
    """
    subprocess.run(["uv", "build", "--out-dir", str(tmp_path)], cwd=PACKAGE, check=True)
    wheel = next(tmp_path.glob("onetaskgraph_sdk-*.whl"))
    with zipfile.ZipFile(wheel) as archive:
        members = archive.namelist()
        assert f"onetaskgraph_sdk/{MARKER}" in members, (
            f"{wheel.name} carries no onetaskgraph_sdk/{MARKER}; add an empty {MARKER} "
            f"beside {SOURCE / '__init__.py'}"
        )
        metadata = next(name for name in members if name.endswith(".dist-info/METADATA"))
        lines = archive.read(metadata).decode("utf-8").splitlines()
    assert CLASSIFIER in lines, (
        f"{metadata} in {wheel.name} declares no `Typing :: Typed`; add it to "
        f"[project].classifiers in {PACKAGE / 'pyproject.toml'}"
    )


def test_regeneration_keeps_the_typed_marker(tmp_path: Path) -> None:
    """The marker sits outside `_generated`, so the generator neither writes nor removes it.

    The real generator runs against the bundle the built binary emits, into a copy of the
    package rather than the tree, so a generator that ever reached past `_generated` would
    fail here without dirtying the checkout. What it wrote *inside* `_generated` is not
    compared with the committed package: that is `generate-check`'s claim, and the justfile
    runs it on Linux alone because the regenerate step is not byte-stable on the Windows
    runner, where this test does run.
    """
    sys.path.insert(0, str(PACKAGE))
    import generate

    package_copy = tmp_path / "onetaskgraph_sdk"
    shutil.copytree(SOURCE, package_copy)
    assert (package_copy / MARKER).is_file()

    bundle = generate.validate_schema_bundle(json.loads(generate.run_workspace_binary("schema")))
    generate.generate(bundle, check=False, destination=package_copy / "_generated")

    assert (package_copy / MARKER).is_file(), f"regeneration removed {MARKER}"
    assert (package_copy / MARKER).read_bytes() == b"", f"regeneration wrote into {MARKER}"
    assert not (package_copy / "_generated" / MARKER).exists(), (
        f"the generator emitted {MARKER} inside _generated, which it does not own"
    )
