"""The timing summaries test-windows prints beside its results, one line each.

    ci_timings.py trace <journey trace> <packaging records>
    ci_timings.py guards <guard starts> <end, in epoch seconds>

`trace` reads the distribution journey's `set -x` trace, whose PS4 carries a timestamp, and
the packaging cases scripts/test-release-products.py recorded. It names the slowest lines,
the slowest cases and the time per hundred lines of the journey. `guards` reads the starts
scripts/script-start-log.sh recorded and names each scripts:test guard's time, from its start
to the next one's, the last one's to the end given. A recording it cannot read exits 1 naming
the file; anything else misused exits 2.
"""
from __future__ import annotations

from dataclasses import dataclass
import math
import re
import sys

TRACE_LINE = re.compile(r"^\++(\d+\.\d+) (\S+:\d+): (.*)")


@dataclass(frozen=True)
class TraceStep:
    """One traced line of the journey: where it is, when it began, and what it ran."""

    site: str
    moment: float
    text: str


@dataclass(frozen=True)
class GuardStart:
    """One guard scripts:test started, and when."""

    moment: int
    path: str


def trace_steps(trace: str) -> list[TraceStep]:
    steps = []
    for line in trace.splitlines():
        match = TRACE_LINE.match(line)
        if match:
            steps.append(TraceStep(match.group(2), float(match.group(1)), match.group(3)))
    return steps


def trace_summary(trace: str, packaging: str) -> str:
    """The slowest journey lines, packaging cases and hundred-line ranges, as one line."""
    steps = trace_steps(trace)
    if not steps:
        raise ValueError("the trace holds no traced line, so the journey ran without its timestamps")
    for step in steps:
        if not math.isfinite(step.moment):
            raise ValueError(f"{step.site} is traced at {step.moment}, which is not a time")
    # A background subshell's lines interleave with the journey's, so the file is not in time
    # order: each line's time runs to the next line by time, whichever process traced it.
    steps.sort(key=lambda step: step.moment)
    spent: dict[str, float] = {}
    text: dict[str, str] = {}
    for step, following in zip(steps, steps[1:]):
        spent[step.site] = spent.get(step.site, 0.0) + following.moment - step.moment
    for step in steps:
        text.setdefault(step.site, step.text)
    lines = [
        f"{seconds:.0f}s {site} {text[site][:60]}"
        for site, seconds in sorted(spent.items(), key=lambda item: -item[1])[:8]
    ]
    cases: dict[str, float] = {}
    for row in packaging.splitlines():
        seconds, label = row.split("\t", 1)
        figure = float(seconds)
        if not math.isfinite(figure) or figure < 0:
            raise ValueError(f"packaging record {row!r} is not a duration in seconds")
        cases[label] = cases.get(label, 0.0) + figure
    slowest = [
        f"{seconds:.0f}s {label[:50]}"
        for label, seconds in sorted(cases.items(), key=lambda item: -item[1])[:8]
    ]
    ranges: dict[int, float] = {}
    for site, seconds in spent.items():
        name, _, number = site.rpartition(":")
        if name == "test-distribution.sh":
            start = int(number) // 100 * 100
            ranges[start] = ranges.get(start, 0.0) + seconds
    by_range = [f"{start}-{start + 99} {seconds:.0f}s" for start, seconds in sorted(ranges.items())]
    return (
        "distribution timing: slowest lines: " + "; ".join(lines)
        + " | slowest packaging cases: " + "; ".join(slowest)
        + " | by line range: " + "; ".join(by_range)
    )


def guard_starts(starts: str) -> list[GuardStart]:
    recorded = []
    for line in starts.splitlines():
        if line:
            moment, path = line.split(" ", 1)
            recorded.append(GuardStart(int(moment), path))
    return recorded


def guard_summary(starts: str, end: int) -> str:
    """Each guard's start to the next one's, the last one's to `end`, in the order they ran."""
    recorded = guard_starts(starts)
    finishes = [start.moment for start in recorded[1:]] + [end]
    for start, finish in zip(recorded, finishes):
        if finish < start.moment:
            raise ValueError(f"{start.path} started at {start.moment}, after what follows it at {finish}")
    figures = [
        f"{finish - start.moment}s {start.path.rsplit('/', 1)[-1]}"
        for start, finish in zip(recorded, finishes)
    ]
    return "scripts:test guards, each to the next one's start: " + "; ".join(figures)


def read(path: str, missing_is_empty: bool = False) -> str:
    try:
        with open(path, encoding="utf-8", errors="replace") as handle:
            return handle.read()
    except FileNotFoundError:
        if missing_is_empty:
            return ""
        raise


def main(arguments: list[str]) -> int:
    sys.stdout.reconfigure(newline="\n")
    if len(arguments) != 3 or arguments[0] not in ("trace", "guards"):
        print(
            "usage: ci_timings.py trace <journey trace> <packaging records> | "
            "guards <guard starts> <end seconds>",
            file=sys.stderr,
        )
        return 2
    kind, recording, other = arguments
    try:
        if kind == "trace":
            print(trace_summary(read(recording), read(other, missing_is_empty=True)))
        else:
            print(guard_summary(read(recording), int(other)))
    except (OSError, ValueError) as error:
        print(
            f"ci_timings: could not summarise {recording} ({type(error).__name__}: {error}); "
            "next: the recording is the one the job wrote, so inspect it, or run "
            "'python3 scripts/test_ci_timings.py' to see which half of the summary broke",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
