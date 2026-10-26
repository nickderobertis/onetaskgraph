"""A process builds the generated models it uses, and no others.

Each journey here runs in a fresh interpreter, because the property is about what an import
leaves in `sys.modules` and this test process has imported the whole package long before.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

GENERATED = "onetaskgraph_sdk._generated."

JOURNEY = """
import asyncio, json, subprocess, sys
from pathlib import Path

binary, folder = sys.argv[1], Path(sys.argv[2])

def generated():
    return sorted(name for name in sys.modules if name.startswith("onetaskgraph_sdk._generated."))

import onetaskgraph_sdk
from onetaskgraph_sdk import Client
at_import = generated()

listed = asyncio.run(Client(binary, cwd=folder).task_list(source=["plan"]))
after_verb = generated()

# A model the verb never answered with, validated from what the binary itself printed.
printed = subprocess.run(
    [binary, "task", "copy", "plan:parse", "--to", "into", "--json"],
    cwd=folder, capture_output=True, text=True, check=True,
).stdout
from onetaskgraph_sdk import CopyReport
after_import = generated()
copied = CopyReport.model_validate_json(printed)
from onetaskgraph_sdk import CopyReport as again
from onetaskgraph_sdk._generated.models import CopyReport as through_models

print(json.dumps({
    "at_import": at_import,
    "after_verb": after_verb,
    "after_import": after_import,
    "listed_type": type(listed).__module__ + "." + type(listed).__qualname__,
    "listed_is_public": type(listed) is onetaskgraph_sdk.QueryResponseOfQualifiedTask,
    "tasks": [
        [task.id.root, task.item.title, task.item.status.category.value]
        for task in listed.items
    ],
    "copied": [
        [item.root.source.root, item.root.action, item.root.destination.root]
        for item in copied.items
    ],
    "same_class": again is CopyReport and through_models is CopyReport,
    "copy_report_is_public": CopyReport is onetaskgraph_sdk.CopyReport,
}))
"""


def plan(tmp_path: Path) -> Path:
    """Configure a Markdown folder holding two tasks and an empty one to copy into."""
    tasks = tmp_path / "plan" / "tasks"
    tasks.mkdir(parents=True)
    (tasks / "parse.md").write_text(
        "---\ntitle: Parse the feed\nstatus: todo\n---\nRead every row.\n", encoding="utf-8"
    )
    (tasks / "load.md").write_text(
        "---\ntitle: Load the rows\nstatus: in-progress\n---\nWrite them.\n", encoding="utf-8"
    )
    (tmp_path / "into").mkdir()
    (tmp_path / "onetaskgraph.yaml").write_text(
        json.dumps(
            {
                "sources": {
                    name: {"plugin": "local-md", "config": {"root": str(tmp_path / name)}}
                    for name in ("plan", "into")
                }
            }
        ),
        encoding="utf-8",
    )
    return tmp_path


def fresh(program: str, *arguments: str) -> subprocess.CompletedProcess[str]:
    """Run `program` in a fresh interpreter of this environment."""
    return subprocess.run(
        [sys.executable, "-c", program, *arguments],
        capture_output=True,
        text=True,
        encoding="utf-8",
        env=os.environ.copy(),
        check=False,
    )


def test_a_verb_builds_its_own_models_and_an_unused_export_builds_on_first_import(
    binary: Path, tmp_path: Path
) -> None:
    """Import, run one verb, then import a model it never used: each loads only its module."""
    folder = plan(tmp_path)
    ran = fresh(JOURNEY, str(binary), str(folder))
    assert ran.returncode == 0, ran.stderr
    seen = json.loads(ran.stdout)

    # Importing the package imports the client and the table of roots, and no model.
    assert seen["at_import"] == [f"{GENERATED}client", f"{GENERATED}models"]
    # The verb answers with the public response model, holding what the binary listed.
    assert seen["listed_type"] == (f"{GENERATED}query_response_of_qualified_task.QueryResponse")
    assert seen["listed_is_public"]
    assert sorted(seen["tasks"]) == [
        ["plan:load", "Load the rows", "in-progress"],
        ["plan:parse", "Parse the feed", "todo"],
    ]
    # It imported its own model's module and none of the others the package exports.
    assert seen["after_verb"] == sorted(
        [*seen["at_import"], f"{GENERATED}query_response_of_qualified_task"]
    )
    assert f"{GENERATED}copy_report" not in seen["after_verb"]
    # Importing an export the verb never used imports that one module, through the package.
    assert seen["after_import"] == sorted([*seen["after_verb"], f"{GENERATED}copy_report"])
    assert seen["copied"] == [["plan:parse", "created", "into:parse"]]
    assert seen["same_class"]
    assert seen["copy_report_is_public"]


@pytest.mark.parametrize("module", ["onetaskgraph_sdk", "onetaskgraph_sdk._generated.models"])
def test_a_name_never_exported_is_still_refused(module: str) -> None:
    """An unknown name raises `AttributeError` and `ImportError` as an eager module would."""
    program = (
        "import importlib, sys\n"
        f"module = importlib.import_module({module!r})\n"
        "try:\n"
        "    module.QueryResponseOfNothing\n"
        "except AttributeError as error:\n"
        "    print('attribute:', error)\n"
        "try:\n"
        f"    from {module} import QueryResponseOfNothing\n"
        "except ImportError as error:\n"
        "    print('import:', type(error).__name__, error)\n"
        f"print(sorted(n for n in sys.modules if n.startswith({GENERATED!r})))\n"
    )
    ran = fresh(program)
    assert ran.returncode == 0, ran.stderr
    lines = ran.stdout.splitlines()
    assert lines[0] == (f"attribute: module {module!r} has no attribute 'QueryResponseOfNothing'")
    assert lines[1].startswith(
        f"import: ImportError cannot import name 'QueryResponseOfNothing' from '{module}'"
    )
    # Refusing the name built nothing on the way.
    assert lines[2] == str([f"{GENERATED}client", f"{GENERATED}models"])
