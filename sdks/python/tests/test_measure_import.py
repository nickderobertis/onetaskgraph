"""The import budget's measurement, driven as onebudgetspec drives it: a real run of the script.

Each run spawns real interpreters of this environment, as the budget's own does, with fewer
runs per figure so the proof costs seconds; what is asserted is what the script reports and
refuses, never a time.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

PACKAGE = Path(__file__).parents[1]
SCRIPT = PACKAGE / "measure_import.py"


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
