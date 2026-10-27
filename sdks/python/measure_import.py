"""Measure the CPU a fresh interpreter spends importing the SDK, validating once and exiting.

The figure `sdk-import-cpu-seconds` in `budgets.yaml` names: the median user+system CPU of
a fresh interpreter of this environment that imports `onetaskgraph_sdk`, validates the
committed copy report into `CopyReport` and exits normally, teardown included. Beside it,
as detail, the interpreter alone (`python -c pass`), the import alone, and the same run
ending in `os._exit(0)` — the teardown share is the difference between the last and the
figure. Every figure is taken from 15 runs, interleaved so a change in the host's load
falls on all of them alike, after one untimed run of each so no figure pays for writing
bytecode.

The figure goes to the file `ONEBUDGETSPEC_RESULT` names, which onebudgetspec compares
with the threshold; the script never reads a threshold itself. Run outside onebudgetspec,
it prints what it measured to standard error and reports nothing.

`--compare DIR` adds, interleaved with the others, the budget's own run against the
package found under DIR (a `src` directory holding another `onetaskgraph_sdk`) placed
first on `PYTHONPATH`, so two trees are read under the same load. It reports the same
statistics for it and leaves the reported figure the environment's own.

The CPU is the child's own, read from `os.wait4`, so it counts nothing of this process and
nothing of a sibling: which is why this measures on POSIX alone, where the budget's host is.
The detail ends with the host's load average when the runs finished, so a reading taken on a
busy host says so beside it.
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
from pathlib import Path

RUNS = 15
"""Runs per figure, as the budget's workload states."""

REPORT = Path(__file__).with_name("measure_import_copy_report.json")
"""One copy report the store binary produced, `project copy` of a three-item plan."""

VALIDATE = (
    "import pathlib, onetaskgraph_sdk\n"
    f"onetaskgraph_sdk.CopyReport.model_validate_json(pathlib.Path({str(REPORT)!r}).read_bytes())\n"
)

PROGRAMS = {
    "baseline": "pass",
    "import": "import onetaskgraph_sdk",
    "full": VALIDATE,
    "abrupt": VALIDATE + "import os\nos._exit(0)\n",
}
"""What each fresh interpreter runs, by the figure it is read into; `full` is the budget's."""


def cpu_seconds(program: str, environment: dict[str, str]) -> float:
    """Run one fresh interpreter of this environment and return its user+system CPU."""
    process = subprocess.Popen(
        [sys.executable, "-c", program],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    assert process.stderr is not None
    error = process.stderr.read()
    _, status, usage = os.wait4(process.pid, 0)
    process.returncode = os.waitstatus_to_exitcode(status)
    if process.returncode != 0:
        raise SystemExit(
            f"measure_import.py: a measured interpreter exited {process.returncode}:\n"
            f"{error.decode(errors='replace')}"
            "next: run the program above with this environment's python to see why"
        )
    return usage.ru_utime + usage.ru_stime


def summary(samples: list[float]) -> str:
    """The median and interquartile range of one figure, in seconds."""
    first, _, third = statistics.quantiles(samples, n=4, method="inclusive")
    return f"median {statistics.median(samples):.3f} s (IQR {first:.3f}-{third:.3f})"


def main() -> None:
    """Measure every figure, print them, and report the budget's figure."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--compare", type=Path, help="a src directory to measure beside")
    # Fewer only to prove the script itself quickly; the budget's figure is taken from 15.
    parser.add_argument("--runs", type=int, default=RUNS, help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.runs < 2:
        parser.error("--runs takes at least 2, the fewest an interquartile range is read from")
    if not hasattr(os, "wait4"):
        raise SystemExit("measure_import.py: this measures with os.wait4, which needs POSIX")

    environment = {
        name: value for name, value in os.environ.items() if not name.startswith("ONEBUDGETSPEC_")
    }
    environment.pop("PYTHONPATH", None)
    runs: dict[str, tuple[str, dict[str, str]]] = {
        name: (program, environment) for name, program in PROGRAMS.items()
    }
    if arguments.compare is not None:
        compared = {**environment, "PYTHONPATH": str(arguments.compare.resolve())}
        located = subprocess.run(
            [sys.executable, "-c", "import onetaskgraph_sdk; print(onetaskgraph_sdk.__file__)"],
            env=compared,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        if not Path(located).is_relative_to(arguments.compare.resolve()):
            raise SystemExit(
                f"measure_import.py: --compare {arguments.compare} imported {located}; "
                "next: name the directory that holds the onetaskgraph_sdk package"
            )
        runs["compared"] = (VALIDATE, compared)

    for program, variables in runs.values():
        cpu_seconds(program, variables)
    samples: dict[str, list[float]] = {name: [] for name in runs}
    for _ in range(arguments.runs):
        for name, (program, variables) in runs.items():
            samples[name].append(cpu_seconds(program, variables))

    medians = {name: statistics.median(values) for name, values in samples.items()}
    figure = medians["full"]
    detail = (
        f"{arguments.runs} runs each of {sys.executable} (Python {sys.version.split()[0]}); "
        f"baseline (python -c pass) {summary(samples['baseline'])}; "
        f"import alone {summary(samples['import'])}; "
        f"import, validate and exit {summary(samples['full'])}; "
        f"validation {figure - medians['import']:.3f} s (figure less import alone); "
        f"teardown {figure - medians['abrupt']:.3f} s "
        f"(figure less the same run ending in os._exit(0), {summary(samples['abrupt'])}); "
        f"spread {min(samples['full']):.3f}-{max(samples['full']):.3f} s; "
        "load average {:.2f} {:.2f} {:.2f} at the end".format(*os.getloadavg())
    )
    if "compared" in samples:
        detail += f"; --compare {arguments.compare}: {summary(samples['compared'])}"
    print(detail, file=sys.stderr)

    result = os.environ.get("ONEBUDGETSPEC_RESULT")
    if result:
        with open(result, "w", encoding="utf-8", newline="\n") as handle:
            json.dump({"value": round(figure, 4), "detail": detail}, handle)


if __name__ == "__main__":
    main()
