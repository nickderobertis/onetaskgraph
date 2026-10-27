"""The SDK host target warns only for measured, over host-variable budgets."""

from __future__ import annotations

import json
import shlex
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

PACKAGE = Path(__file__).parents[1]
WORKSPACE = PACKAGE.parents[1]

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="the CPU host target is POSIX")


def run_target(budgets: Path) -> subprocess.CompletedProcess[str]:
    """Drive the configured target command with a real budgets file and onebudgetspec."""
    project = json.loads((PACKAGE / "project.json").read_text(encoding="utf-8"))
    command = shlex.split(project["targets"]["budgets-host"]["options"]["command"])
    command = [
        str(budgets) if argument == "sdks/python/budgets.yaml" else argument for argument in command
    ]
    return subprocess.run(
        command,
        cwd=WORKSPACE,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )


def write_budgets(path: Path, *, variable: bool, value: int, broken: bool = False) -> None:
    """Register a reported fixture measured by a real subprocess, with a strict threshold."""
    report = (
        "import sys; sys.exit(3)"
        if broken
        else (
            "import json, os; from pathlib import Path; "
            f"Path(os.environ['ONEBUDGETSPEC_RESULT']).write_text(json.dumps({{'value': {value}}}))"
        )
    )
    command = json.dumps([sys.executable, "-c", report])
    labels = "[host, host-variable]" if variable else "[host]"
    path.write_text(
        "schema_version: 1\nbudgets:\n  - id: fixture\n"
        "    description: A subprocess reports a known reading.\n"
        f"    labels: {labels}\n    measure: reported\n    command: {command}\n"
        "    unit: seconds\n    direction: max\n    threshold: 1\n",
        encoding="utf-8",
    )


@pytest.mark.parametrize(
    ("variable", "value", "status", "warning"),
    [(True, 2, 0, True), (False, 2, 1, False), (True, 0, 0, False)],
)
def test_host_target_applies_warning_only_to_variable_over_readings(
    tmp_path: Path, variable: bool, value: int, status: int, warning: bool
) -> None:
    """An over variable reading warns; a strict over refuses; a within reading succeeds."""
    budgets = tmp_path / "budgets.yaml"
    write_budgets(budgets, variable=variable, value=value)
    ran = run_target(budgets)
    assert ran.returncode == status, ran.stdout + ran.stderr
    assert ("warning:" in ran.stderr) is warning, ran.stdout + ran.stderr
    assert ("over" if value > 1 else "within") in ran.stdout


@pytest.mark.parametrize("variable", [True, False])
def test_a_failed_measurement_refuses_even_when_variable(tmp_path: Path, variable: bool) -> None:
    """A measurement error cannot be mistaken for an over reading and downgraded."""
    budgets = tmp_path / "budgets.yaml"
    write_budgets(budgets, variable=variable, value=0, broken=True)
    ran = run_target(budgets)
    assert ran.returncode > 1, ran.stdout + ran.stderr
    assert "warning:" not in ran.stderr
    assert "error" in (ran.stdout + ran.stderr).lower()


def test_the_registered_import_budget_is_host_variable() -> None:
    """The real import budget opts in, leaving other host budgets strict."""
    budgets = yaml.safe_load((PACKAGE / "budgets.yaml").read_text(encoding="utf-8"))
    (budget,) = budgets["budgets"]
    assert budget["id"] == "sdk-import-cpu-seconds"
    assert set(budget["labels"]) == {"host", "host-variable"}


def test_direct_checks_remain_strict_for_ci(tmp_path: Path) -> None:
    """The label changes only the host target's handling, never onebudgetspec's verdict."""
    budgets = tmp_path / "budgets.yaml"
    write_budgets(budgets, variable=True, value=2)
    ran = subprocess.run(
        ["bun", "run", "onebudgetspec", "check", str(budgets), "--label", "host"],
        cwd=WORKSPACE,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    assert ran.returncode == 1, ran.stdout + ran.stderr
    assert "over" in ran.stdout
    assert "warning:" not in ran.stderr


@pytest.mark.parametrize(
    ("strict_value", "strict_broken", "variable_broken", "status", "warning"),
    [
        (2, False, False, 1, False),
        (2, False, True, 3, False),
        (0, True, False, 3, False),
        (0, False, False, 0, True),
    ],
)
def test_mixed_budgets_preserve_errors_and_strict_failures(
    tmp_path: Path,
    strict_value: int,
    strict_broken: bool,
    variable_broken: bool,
    status: int,
    warning: bool,
) -> None:
    """Both passes run, and a variable over cannot hide a strict failure or either error."""
    strict = tmp_path / "strict.yaml"
    variable = tmp_path / "variable.yaml"
    write_budgets(strict, variable=False, value=strict_value, broken=strict_broken)
    write_budgets(variable, variable=True, value=2, broken=variable_broken)
    combined = yaml.safe_load(strict.read_text(encoding="utf-8"))
    other = yaml.safe_load(variable.read_text(encoding="utf-8"))["budgets"][0]
    other["id"] = "variable-fixture"
    combined["budgets"].append(other)
    budgets = tmp_path / "budgets.yaml"
    budgets.write_text(yaml.safe_dump(combined), encoding="utf-8")

    ran = run_target(budgets)
    assert ran.returncode == status, ran.stdout + ran.stderr
    assert ("warning:" in ran.stderr) is warning, ran.stdout + ran.stderr
    assert "budget fixture:" in ran.stdout
    assert "budget variable-fixture:" in ran.stdout
