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

import re
import sys

TRACE_LINE = re.compile(r"^\++(\d+\.\d+) (\S+:\d+): (.*)")


def trace_summary(trace: str, packaging: str) -> str:
    """The slowest journey lines, packaging cases and hundred-line ranges, as one line."""
    spent: dict[str, float] = {}
    text: dict[str, str] = {}
    previous: tuple[str, float] | None = None
    for line in trace.splitlines():
        match = TRACE_LINE.match(line)
        if not match:
            continue
        moment, site = float(match.group(1)), match.group(2)
        if previous is not None:
            spent[previous[0]] = spent.get(previous[0], 0.0) + moment - previous[1]
        previous = (site, moment)
        text.setdefault(site, match.group(3))
    lines = [
        f"{seconds:.0f}s {site} {text[site][:60]}"
        for site, seconds in sorted(spent.items(), key=lambda item: -item[1])[:8]
    ]
    cases: dict[str, float] = {}
    for row in packaging.splitlines():
        seconds, label = row.split("\t", 1)
        cases[label] = cases.get(label, 0.0) + float(seconds)
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


def guard_summary(starts: str, end: int) -> str:
    """Each guard's start to the next one's, the last one's to `end`, in the order they ran."""
    recorded = [line.split(" ", 1) for line in starts.splitlines() if line]
    finishes = [int(moment) for moment, _ in recorded[1:]] + [end]
    figures = [
        f"{finish - int(moment)}s {path.rsplit('/', 1)[-1]}"
        for (moment, path), finish in zip(recorded, finishes)
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
