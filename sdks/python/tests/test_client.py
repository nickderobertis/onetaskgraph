"""Public-boundary tests for the generated SDK."""

from __future__ import annotations

import asyncio
import json
import os
import subprocess
import sys
from collections.abc import Awaitable
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client, GlobalId, OnetaskgraphError, __version__
from onetaskgraph_sdk._generated.models import QueryResponseOfQualifiedTask

WORKSPACE = Path(__file__).parents[3]


def run[T](awaitable: Awaitable[T]) -> T:
    """Drive one public async SDK call to completion in a script-shaped test."""
    return asyncio.run(awaitable)


@pytest.fixture(scope="session")
def binary() -> Path:
    """Build and return the real workspace executable."""
    subprocess.run(
        ["cargo", "build", "--quiet", "-p", "onetaskgraph", "--bin", "onetaskgraph"],
        cwd=WORKSPACE,
        check=True,
    )
    suffix = ".exe" if sys.platform == "win32" else ""
    return (WORKSPACE / "target" / "debug" / f"onetaskgraph{suffix}").resolve()


def configured(tmp_path: Path, *, failing: bool = False) -> Path:
    """Create real Markdown and in-memory sources, optionally with one broken source."""
    markdown = tmp_path / "markdown" / "tasks"
    markdown.mkdir(parents=True)
    (markdown / "M-1.md").write_text(
        "---\ntitle: Markdown task\nstatus: Todo\n---\nBody from disk\n", encoding="utf-8"
    )
    sources: dict[str, object] = {
        "memory": {
            "plugin": "in-memory",
            "config": {
                "tasks": [
                    {
                        "id": "T-1",
                        "title": "Memory task",
                        "status": {"category": "todo", "name": "Todo"},
                        "labels": [],
                        "project": "P-1",
                    }
                ],
                "projects": [
                    {
                        "id": "P-1",
                        "title": "Memory project",
                        "status": {"category": "todo", "name": "Todo"},
                        "labels": [],
                    }
                ],
                "labels": [{"id": "L-1", "name": "sdk"}],
            },
        },
        "markdown": {
            "plugin": "local-md",
            "config": {"root": str(tmp_path / "markdown"), "status_mapping": {"todo": "todo"}},
        },
    }
    if failing:
        sources["broken"] = {"plugin": "github-projects", "config": {}}
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    return tmp_path


def test_real_sources_and_typed_partial_failure(binary: Path, tmp_path: Path) -> None:
    """Return validated rows, plans, and a typed failure from actual sources."""
    client = Client(
        binary,
        cwd=configured(tmp_path, failing=True),
        environment={"ONETASKGRAPH_SDK_BINARY": str(tmp_path / "wrong")},
    )
    response = run(client.task_list())
    assert {item.item.title for item in response.items} == {"Markdown task", "Memory task"}
    assert {plan.source.root for plan in response.plan.per_source} == {"memory", "markdown"}
    assert response.errors[0].source.root == "broken"
    assert response.errors[0].error.root.kind == "config"


def test_public_error_contains_exit_status(binary: Path, tmp_path: Path) -> None:
    """Expose a malformed invocation as the documented typed client exception."""
    client = Client(binary, cwd=configured(tmp_path))
    with pytest.raises(OnetaskgraphError) as caught:
        run(client.task_show(id="not-qualified"))
    assert caught.value.exit_code == 1
    assert "qualify the id" in str(caught.value)


def test_real_binary_response_is_rejected_against_the_wrong_contract(
    binary: Path, tmp_path: Path
) -> None:
    """Reject real process JSON when it does not match the selected generated model."""
    client = Client(binary, cwd=configured(tmp_path))
    with pytest.raises(OnetaskgraphError, match="outside its emitted schema"):
        run(client._invoke(["sources", "list"], QueryResponseOfQualifiedTask))


def test_every_generated_method_drives_the_binary(binary: Path, tmp_path: Path) -> None:
    """Exercise every generated command method and representative CLI option shapes."""
    client = Client(binary, cwd=configured(tmp_path))
    assert run(
        client.task_list(
            source=("memory",),
            status=["todo"],
            limit=2,
            explain=True,
            page=None,
            no_project=False,
        )
    ).items
    assert run(client.task_show(id=GlobalId(root="memory:T-1"))).items
    assert run(client.task_deps(id="memory:T-1")).items == []
    assert run(client.project_list(source=["memory"])).items
    assert run(client.project_show(id="memory:P-1")).items
    assert run(client.project_deps(id="memory:P-1")).items == []
    assert run(client.label_list(source=["memory"])).items
    assert run(client.search(text="Memory", kind="task")).items
    assert run(client.sources_list())
    assert run(client.config_show()).settings


def test_binary_resolution_order(binary: Path, tmp_path: Path) -> None:
    """Use environment before the packaged PATH fallback and reject no executable."""
    config = configured(tmp_path)
    relative_binary = Path(os.path.relpath(binary, Path.cwd()))
    from_explicit_relative = Client(relative_binary, cwd=config)
    assert run(from_explicit_relative.task_list()).items
    from_environment = Client(
        cwd=config, environment={"ONETASKGRAPH_SDK_BINARY": str(relative_binary), "PATH": ""}
    )
    assert run(from_environment.task_list()).items
    from_path = Client(cwd=config, environment={"PATH": str(binary.parent)})
    assert run(from_path.task_list()).items
    with pytest.raises(FileNotFoundError, match="binary not found"):
        Client(environment={"PATH": ""})
    with pytest.raises(FileNotFoundError, match="not an executable"):
        Client(tmp_path / "missing")


def test_generator_rejects_drift_and_unmapped_commands(tmp_path: Path) -> None:
    """Name stale output and a newly discovered command with no client method."""
    expected = tmp_path / "expected"
    actual = tmp_path / "actual"
    expected.mkdir()
    actual.mkdir()
    (expected / "effective_config.py").write_text("current", encoding="utf-8")
    (actual / "effective_config.py").write_text("stale", encoding="utf-8")
    stale = subprocess.run(
        [
            sys.executable,
            "-c",
            "from pathlib import Path; import generate; "
            f"generate.check_generated(Path({str(expected)!r}), Path({str(actual)!r}))",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert stale.returncode == 1
    assert "effective_config.py" in stale.stderr
    assert "uv run python generate.py" in stale.stderr
    invalid_bundle = subprocess.run(
        [sys.executable, "-c", "import generate; generate.validate_schema_bundle([])"],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert invalid_bundle.returncode == 1
    assert "invalid schema bundle" in invalid_bundle.stderr
    changed_option = subprocess.run(
        [
            sys.executable,
            "-c",
            "import generate; generate.validate_option_placeholders({'limit':'TEXT'}, ['limit'])",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert changed_option.returncode == 1
    assert "value shape" in changed_option.stderr
    missing_roots = subprocess.run(
        [sys.executable, "-c", "import generate; generate.validate_schema_bundle({'roots':{}})"],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert missing_roots.returncode == 1
    assert "missing roots" in missing_roots.stderr
    missing_choices = subprocess.run(
        [sys.executable, "-c", "import generate; generate.choice_values('', 'kind')"],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert missing_choices.returncode == 1
    assert "did not report option" in missing_choices.stderr
    empty_choices = subprocess.run(
        [
            sys.executable,
            "-c",
            "import generate; generate.choice_values('--kind <KIND>\\n', 'kind')",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert empty_choices.returncode == 1
    assert "possible values" in empty_choices.stderr
    malformed_root = subprocess.run(
        [
            sys.executable,
            "-c",
            "import json, generate; "
            "bundle=json.loads(generate.run_workspace_binary('schema')); "
            "bundle['roots']['QueryPlan']=[]; generate.validate_schema_bundle(bundle)",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert malformed_root.returncode == 1
    assert "non-object roots: QueryPlan" in malformed_root.stderr
    missing_minimum = subprocess.run(
        [
            sys.executable,
            "-c",
            "import generate; generate.documented_minimum({}, 'Capabilities.max_page_size')",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert missing_minimum.returncode == 1
    assert "Capabilities.max_page_size" in missing_minimum.stderr
    unmapped = subprocess.run(
        [
            sys.executable,
            "-c",
            "from pathlib import Path; import generate; "
            f"generate.generate_client([('future',)], Path({str(tmp_path)!r}))",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        text=True,
        capture_output=True,
        check=False,
    )
    assert unmapped.returncode == 1
    assert "future" in unmapped.stderr


def test_generator_write_mode_uses_real_binary(tmp_path: Path) -> None:
    """Regenerate into a fresh destination from the real schema and command surface."""
    destination = tmp_path / "generated"
    subprocess.run(
        [
            sys.executable,
            "-c",
            "import json; from pathlib import Path; import generate; "
            "bundle=generate.validate_schema_bundle("
            "json.loads(generate.run_workspace_binary('schema'))); "
            f"generate.generate(bundle, check=False, destination=Path({str(destination)!r}))",
        ],
        cwd=WORKSPACE / "sdks" / "python",
        check=True,
    )
    assert (destination / "client.py").is_file()


def test_distribution_version() -> None:
    """Keep the one public version aligned with the manifest."""
    assert __version__ == "0.1.0"
