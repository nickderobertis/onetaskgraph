"""The import budget's measurement, driven as onebudgetspec drives it: a real run of the script.

Each run spawns real interpreters of this environment, as the budget's own does, with fewer
runs per figure so the proof costs seconds; what is asserted is what the script reports and
refuses, never a time.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

PACKAGE = Path(__file__).parents[1]
WORKSPACE = PACKAGE.parents[1]
SCRIPT = PACKAGE / "measure_import.py"

pytestmark = pytest.mark.skipif(
    not hasattr(os, "wait4"), reason="the measurement reads a child's CPU with POSIX os.wait4"
)


def measure(*arguments: str, result: Path | None = None) -> subprocess.CompletedProcess[str]:
    """Run the script as the budget's command does, reporting to `result` when one is given."""
    environment = {
        name: value for name, value in os.environ.items() if not name.startswith("ONEBUDGETSPEC_")
    }
    if result is not None:
        environment["ONEBUDGETSPEC_RESULT"] = str(result)
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--runs", "2", *arguments],
        cwd=PACKAGE,
        env=environment,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )


def test_the_figure_and_every_share_reach_the_file_onebudgetspec_names(tmp_path: Path) -> None:
    """The figure is the full run's median, and the detail names each share beside it."""
    result = tmp_path / "result.json"
    ran = measure(result=result)
    assert ran.returncode == 0, ran.stderr

    reported = json.loads(result.read_text(encoding="utf-8"))
    assert sorted(reported) == ["detail", "value"]
    assert isinstance(reported["value"], float) and reported["value"] > 0
    detail = reported["detail"]
    for share in (
        "baseline (python -c pass) median",
        "import alone median",
        "import, validate and exit median",
        "validation",
        "teardown",
        "os._exit(0)",
        "spread",
        "load average",
    ):
        assert share in detail, f"the detail does not name {share!r}: {detail}"
    assert f"import, validate and exit median {reported['value']:.3f} s" in detail
    # What onebudgetspec records is what a person running the script reads.
    assert ran.stderr.strip() == detail


def test_run_outside_onebudgetspec_it_prints_and_writes_nothing(tmp_path: Path) -> None:
    """With no result file named, the figures go to standard error and nowhere else."""
    ran = measure()
    assert ran.returncode == 0, ran.stderr
    assert "import, validate and exit median" in ran.stderr
    assert ran.stdout == ""


def test_compare_measures_another_tree_beside_this_one(tmp_path: Path) -> None:
    """Another copy of the package is measured, interleaved, and named in the detail."""
    shutil.copytree(PACKAGE / "src", tmp_path / "src")
    result = tmp_path / "result.json"
    ran = measure("--compare", str(tmp_path / "src"), result=result)
    assert ran.returncode == 0, ran.stderr
    detail = json.loads(result.read_text(encoding="utf-8"))["detail"]
    assert f"--compare {tmp_path / 'src'}: median" in detail


def test_compare_refuses_a_directory_without_the_package(tmp_path: Path) -> None:
    """A directory that does not hold the package would measure this one twice; it is refused."""
    result = tmp_path / "result.json"
    ran = measure("--compare", str(tmp_path), result=result)
    assert ran.returncode != 0
    assert f"--compare {tmp_path} imported" in ran.stderr
    assert "next: name the directory that holds the onetaskgraph_sdk package" in ran.stderr
    assert not result.exists()


def test_a_measured_interpreter_that_fails_fails_the_measurement(tmp_path: Path) -> None:
    """A run that cannot validate is not a figure: the script exits naming why, reporting none."""
    broken = tmp_path / "src" / "onetaskgraph_sdk"
    broken.mkdir(parents=True)
    (broken / "__init__.py").write_text("", encoding="utf-8")
    result = tmp_path / "result.json"
    ran = measure("--compare", str(tmp_path / "src"), result=result)
    assert ran.returncode != 0
    assert "a measured interpreter exited 1" in ran.stderr
    assert "has no attribute 'CopyReport'" in ran.stderr
    assert not result.exists()


def test_fewer_than_two_runs_is_refused() -> None:
    """One run has no interquartile range, so the script refuses before it measures."""
    ran = subprocess.run(
        [sys.executable, str(SCRIPT), "--runs", "1"],
        cwd=PACKAGE,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    assert ran.returncode == 2
    assert "--runs takes at least 2" in ran.stderr


def onebudgetspec(budgets: Path) -> tuple[int, dict[str, object]]:
    """Check `budgets` through the workspace's onebudgetspec, answering its exit and result."""
    bun = shutil.which("bun")
    if bun is None:
        pytest.fail(f"bun is missing; run `just bootstrap` from {WORKSPACE}")
    environment = {
        name: value for name, value in os.environ.items() if not name.startswith("ONEBUDGETSPEC_")
    }
    checked = subprocess.run(
        [bun, "run", "onebudgetspec", "check", str(budgets), "--label", "host", "--json"],
        cwd=WORKSPACE,
        env=environment,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    assert checked.returncode in {0, 1}, f"onebudgetspec errored:\n{checked.stdout}{checked.stderr}"
    (result,) = json.loads(checked.stdout)["results"]
    return checked.returncode, result


def test_onebudgetspec_reads_the_report_and_compares_it_with_the_threshold(
    tmp_path: Path,
) -> None:
    """The figure the script writes is the actual onebudgetspec compares, with its detail.

    Which verdict comes back depends on the host, so none is required: what is held is that
    the verdict and the exit status are the comparison of that actual with the threshold.
    """
    budgets = tmp_path / "budgets.yaml"
    command = json.dumps([sys.executable, str(SCRIPT), "--runs", "2"])
    threshold = 0.35
    budgets.write_text(
        "schema_version: 1\nbudgets:\n  - id: sdk-import-cpu-seconds\n"
        "    description: The script under test, at two runs a figure.\n"
        f"    labels: [host]\n    measure: reported\n    command: {command}\n"
        f"    unit: seconds\n    direction: max\n    threshold: {threshold}\n",
        encoding="utf-8",
    )
    exit_status, result = onebudgetspec(budgets)
    assert result["error"] is None
    actual = float(str(result["actual"]))
    detail = str(result["detail"])
    figure = re.search(r"import, validate and exit median ([0-9.]+) s", detail)
    assert figure is not None, detail
    assert abs(actual - float(figure.group(1))) < 0.001
    within = actual <= threshold
    assert (exit_status, result["verdict"]) == ((0, "within") if within else (1, "over"))


def test_the_registered_budget_runs_its_own_command_through_onebudgetspec() -> None:
    """The committed budgets.yaml resolves its command and is measured, never errored.

    Whether the figure is within the threshold depends on the host's load, so the verdict is
    either; what this holds is that the declared path reaches a figure at all.
    """
    exit_status, result = onebudgetspec(PACKAGE / "budgets.yaml")
    assert result["id"] == "sdk-import-cpu-seconds"
    assert result["error"] is None
    assert (exit_status, result["verdict"]) in {(0, "within"), (1, "over")}
    assert "15 runs each of" in str(result["detail"])
