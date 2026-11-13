"""A process sampler: the smallest recorder that attributes a CI job's wall clock to what ran.

    python sampler.py OUT.jsonl [--tick SECONDS]

Every tick it reads the process table (psutil), classifies each process once by its name and
command line (`classify`, whose categories are `models.Category`, the enum the report reads),
and writes one JSON line: which categories had a *leaf* process alive — a process none of whose
live descendants is itself work — and the CPU seconds each category spent since the last tick.
When a process ends it writes its lifetime and CPU. It stops when OUT.jsonl.stop appears.

Exit status: 0 stopped by its stop file; 2 a usage error (named on standard error).
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# E402: models sits beside this file and is found through the sys.path entry above.
from models import Category, Exit, Tick  # noqa: E402

TEST_BINARY = re.compile(r"^[a-z0-9_]+-[0-9a-f]{16}$")
LINKERS = frozenset(
    {
        "ld",
        "ld64",
        "cc",
        "clang",
        "gcc",
        "link",
        "lld",
        "rust-lld",
        "ld.lld",
        "mold",
        "collect2",
        "lld-link",
        "ld.bfd",
        "x86_64-linux-gnu-ld",
        "ld-classic",
    }
)
SHELLS = frozenset({"sh", "bash", "zsh", "dash", "pwsh", "powershell", "cmd", "conhost"})
PYTHON_TOOLS = frozenset({"uv", "uvx", "pytest", "maturin"})
NAMED = {
    "rustdoc": Category.DOCTEST,
    "cargo": Category.CARGO,
    "cargo-nextest": Category.CARGO,
    "cargo-llvm-cov": Category.CARGO,
    "llvm-profdata": Category.COVERAGE_REPORT,
    "llvm-cov": Category.COVERAGE_REPORT,
    "npm": Category.NODE,
    "npx": Category.NODE,
    "bun": Category.BUN,
    "onevcs": Category.ONEVCS,
    "gh": Category.GH,
    "curl": Category.NETWORK,
    "wget": Category.NETWORK,
    "tar": Category.ARCHIVE,
    "gtar": Category.ARCHIVE,
    "zstd": Category.ARCHIVE,
    "bsdtar": Category.ARCHIVE,
    "7z": Category.ARCHIVE,
    "just": Category.JUST,
}
USAGE_EXIT = 2


def node_category(command_line: str) -> Category:
    path = command_line.replace("\\", "/")
    if "op-nx" in path or "start-cache-server" in path or "github-cache" in path:
        return Category.NX_CACHE_SIDECAR
    if "/nx/" in path or " nx " in f" {path} " or "nx.js" in path or "daemon" in path:
        return Category.NX
    return Category.NODE


def classify(name: str, cmdline: list[str]) -> Category:
    base = name.lower().removesuffix(".exe")
    match base:
        case _ if base.startswith("rustc"):
            return Category.COMPILE
        case _ if base in NAMED:
            return NAMED[base]
        case _ if base in LINKERS:
            return Category.LINK
        case _ if base.startswith("onetaskgraph"):
            return Category.CLI_UNDER_TEST
        case _ if base == "git" or base.startswith("git-"):
            return Category.GIT
        case "node":
            return node_category(" ".join(cmdline).lower())
        case _ if base.startswith("python") or base in PYTHON_TOOLS:
            return Category.PYTHON
        case _ if TEST_BINARY.match(base):
            return Category.TEST_BINARY
        case _ if base in SHELLS:
            return Category.SHELL
        case _:
            return Category.OTHER


@dataclass
class Seen:
    """A process the sampler has classified, with what it last read of it."""

    category: Category
    created: float
    first_cpu: float
    last_cpu: float
    last_seen: float


@dataclass(frozen=True)
class Alive:
    """A process present in this tick's table."""

    parent: int
    category: Category


@dataclass(frozen=True)
class Sample:
    at: float
    alive: dict[int, Alive]
    cpu: dict[Category, float]


def leaves(alive: dict[int, Alive]) -> dict[Category, int]:
    """Count, per category, the processes none of whose live descendants is itself working."""
    has_working_child: set[int] = set()
    for process in alive.values():
        if process.category is Category.NX_CACHE_SIDECAR:  # a background server, never anyone's work
            continue
        parent, hops = process.parent, 0
        while parent in alive and hops < 64:
            has_working_child.add(parent)
            parent, hops = alive[parent].parent, hops + 1
    counts: dict[Category, int] = {}
    for pid, process in alive.items():
        if pid not in has_working_child:
            counts[process.category] = counts.get(process.category, 0) + 1
    return counts


def exits(seen: dict[int, Seen], alive: dict[int, Alive]) -> list[Exit]:
    """Forget each process no longer alive; a brief unclassified one is not worth a record."""
    out = []
    for pid in [p for p in seen if p not in alive]:
        gone = seen.pop(pid)
        if gone.category is not Category.OTHER or gone.last_seen - gone.created > 1:
            out.append(
                Exit(
                    gone.category,
                    gone.created,
                    max(gone.last_seen, gone.created),
                    max(0.0, gone.last_cpu - gone.first_cpu),
                )
            )
    return out


def sample(psutil, seen: dict[int, Seen], me: int, previous: float, tick: float) -> Sample:
    now = time.time()
    alive: dict[int, Alive] = {}
    cpu: dict[Category, float] = {}
    for proc in psutil.process_iter(["pid", "ppid", "name", "create_time"]):
        info = proc.info
        pid, created = info["pid"], info["create_time"] or now
        if pid in (me, 0):
            continue
        known = seen.get(pid)
        if known is None or abs(known.created - created) > 0.01:
            try:
                command = proc.cmdline()
            except (psutil.Error, OSError):
                command = []
            known = Seen(classify(info["name"] or "", command), created, 0.0, 0.0, now)
            seen[pid] = known
            if created < previous - tick:  # it was running before this sampler saw it: count only its CPU from now
                known.first_cpu = known.last_cpu = cpu_of(psutil, proc, 0.0)
        total = cpu_of(psutil, proc, known.last_cpu)
        cpu[known.category] = cpu.get(known.category, 0.0) + max(0.0, total - known.last_cpu)
        known.last_cpu, known.last_seen = total, now
        alive[pid] = Alive(info["ppid"] or 0, known.category)
    return Sample(now, alive, cpu)


def cpu_of(psutil, proc, fallback: float) -> float:
    try:
        times = proc.cpu_times()
        return times.user + times.system
    except (psutil.Error, OSError):
        return fallback


def positive_seconds(text: str) -> float:
    value = float(text)
    if not math.isfinite(value) or value <= 0:
        raise argparse.ArgumentTypeError(f"--tick takes a finite positive number of seconds, not {text}")
    return value


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        prog="sampler.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("out")
    parser.add_argument("--tick", type=positive_seconds, default=1.0)
    args = parser.parse_args(argv)
    # Installed only where the recorder runs (the demonstration's own venv), so imported after the usage check.
    import psutil  # noqa: PLC0415  # ty: ignore[unresolved-import]

    stop = Path(args.out + ".stop")
    seen: dict[int, Seen] = {}
    with open(args.out, "a", encoding="utf-8") as out:
        out.write(json.dumps({"kind": "start", "t": time.time(), "tick": args.tick, "platform": sys.platform}) + "\n")
        previous = time.time()
        while not stop.exists():
            taken = sample(psutil, seen, os.getpid(), previous, args.tick)
            records = [Tick(taken.at, taken.at - previous, leaves(taken.alive), taken.cpu), *exits(seen, taken.alive)]
            out.writelines(json.dumps(r.to_json()) + "\n" for r in records)
            out.flush()
            previous = taken.at
            time.sleep(max(0.0, args.tick - (time.time() - taken.at)))
        out.write(json.dumps({"kind": "stop", "t": time.time()}) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
