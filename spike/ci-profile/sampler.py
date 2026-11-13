"""A process sampler: the smallest recorder that attributes a CI job's wall clock to what ran.

Every TICK seconds it reads the process table (psutil), classifies each process once by its
name and command line, and writes one JSON line: which categories had a *leaf* process alive
(a process none of whose live descendants is itself work), and the CPU seconds each category
spent since the last tick. When a process ends it writes its lifetime. Classification lives
in `classify`, which `ciprofile.py` reuses, so the recorder and the report cannot disagree.

Run: python sampler.py OUT.jsonl [--tick 0.25]   (stops when STOP_FILE appears or on SIGTERM)
"""

from __future__ import annotations

import json
import os
import re
import sys
import time

TEST_BINARY = re.compile(r"^[a-z0-9_]+-[0-9a-f]{16}$")
LINKERS = {"ld", "ld64", "cc", "clang", "gcc", "link", "lld", "rust-lld", "ld.lld", "mold", "collect2", "lld-link", "ld.bfd", "x86_64-linux-gnu-ld", "ld-classic"}
SHELLS = {"sh", "bash", "zsh", "dash", "pwsh", "powershell", "cmd", "conhost"}


def classify(name: str, cmdline: list[str]) -> str:
    base = name.lower()
    if base.endswith(".exe"):
        base = base[:-4]
    joined = " ".join(cmdline).lower()
    if base == "rustc" or base.startswith("rustc"):
        return "compile"
    if base == "rustdoc":
        return "doctest"
    if base in LINKERS:
        return "link"
    if base in {"cargo", "cargo-nextest", "cargo-llvm-cov"}:
        return "cargo"
    if base in {"llvm-profdata", "llvm-cov"}:
        return "coverage-report"
    if base.startswith("onetaskgraph"):
        return "cli-under-test"
    if base == "git" or base.startswith("git-"):
        return "git"
    if base == "node":
        if "/nx/" in joined.replace("\\", "/") or " nx " in f" {joined} " or "nx.js" in joined or "daemon" in joined:
            return "nx"
        if "op-nx" in joined or "start-cache-server" in joined or "github-cache" in joined:
            return "nx-cache-sidecar"
        return "node"
    if base in {"npm", "npx"}:
        return "node"
    if base == "bun":
        return "bun"
    if base.startswith("python") or base in {"uv", "uvx", "pytest", "maturin"}:
        return "python"
    if base == "onevcs":
        return "onevcs"
    if base == "gh":
        return "gh"
    if base in {"curl", "wget"}:
        return "network"
    if base in {"tar", "gtar", "zstd", "bsdtar", "7z"}:
        return "archive"
    if TEST_BINARY.match(base):
        return "test-binary"
    if base in SHELLS:
        return "shell"
    if base in {"just"}:
        return "just"
    return "other"


# Categories that are not work of their own: a process of one of these with a working
# descendant is the descendant's wait, so only the leaf is counted.
WORK = {"compile", "doctest", "link", "cargo", "coverage-report", "cli-under-test", "git", "node", "bun",
        "python", "onevcs", "gh", "network", "archive", "test-binary", "shell", "just", "nx", "other"}


def main() -> int:
    import psutil  # noqa: PLC0415 — installed only where the recorder runs

    out_path = sys.argv[1]
    tick = float(sys.argv[sys.argv.index("--tick") + 1]) if "--tick" in sys.argv else 0.25
    stop_file = out_path + ".stop"
    me = os.getpid()
    known: dict[int, tuple[str, str, float]] = {}  # pid -> (category, name, create_time)
    first_cpu: dict[int, float] = {}
    last_cpu: dict[int, float] = {}
    last_seen: dict[int, float] = {}
    with open(out_path, "a", encoding="utf-8") as out:
        out.write(json.dumps({"kind": "start", "t": time.time(), "tick": tick, "platform": sys.platform}) + "\n")
        previous = time.time()
        while not os.path.exists(stop_file):
            now = time.time()
            alive: dict[int, tuple[int, str]] = {}
            cpu_delta: dict[str, float] = {}
            for proc in psutil.process_iter(["pid", "ppid", "name", "create_time"]):
                info = proc.info
                pid = info["pid"]
                if pid == me or pid == 0:
                    continue
                key = known.get(pid)
                if key is None or abs(key[2] - (info["create_time"] or 0)) > 0.01:
                    try:
                        cmd = proc.cmdline()
                    except (psutil.Error, OSError):
                        cmd = []
                    category = classify(info["name"] or "", cmd)
                    key = (category, info["name"] or "", info["create_time"] or now)
                    known[pid] = key
                    first_cpu.pop(pid, None)
                try:
                    times = proc.cpu_times()
                    cpu = times.user + times.system
                except (psutil.Error, OSError):
                    cpu = last_cpu.get(pid, 0.0)
                if pid not in first_cpu:
                    first_cpu[pid] = cpu if key[2] < previous - tick else 0.0
                    last_cpu[pid] = first_cpu[pid]
                delta = max(0.0, cpu - last_cpu.get(pid, cpu))
                last_cpu[pid] = cpu
                last_seen[pid] = now
                cpu_delta[key[0]] = cpu_delta.get(key[0], 0.0) + delta
                alive[pid] = (info["ppid"] or 0, key[0])
            # leaves: a work process none of whose live descendants is work
            has_work_child: set[int] = set()
            for pid, (ppid, category) in alive.items():
                if category in WORK:
                    parent = ppid
                    seen = 0
                    while parent in alive and seen < 64:
                        has_work_child.add(parent)
                        parent = alive[parent][0]
                        seen += 1
            leaves: dict[str, int] = {}
            for pid, (_ppid, category) in alive.items():
                if category in WORK and pid not in has_work_child:
                    leaves[category] = leaves.get(category, 0) + 1
            out.write(json.dumps({"kind": "tick", "t": now, "dt": now - previous, "leaves": leaves,
                                  "cpu": {k: round(v, 4) for k, v in cpu_delta.items() if v}}) + "\n")
            for pid in [p for p in known if p not in alive]:
                category, name, created = known.pop(pid)
                if category != "other" or (last_seen.get(pid, created) - created) > 1:
                    out.write(json.dumps({"kind": "exit", "pid": pid, "name": name, "category": category,
                                          "start": created, "end": last_seen.get(pid, created),
                                          "cpu": round(last_cpu.get(pid, 0.0) - first_cpu.get(pid, 0.0), 4)}) + "\n")
                first_cpu.pop(pid, None)
                last_cpu.pop(pid, None)
                last_seen.pop(pid, None)
            out.flush()
            previous = now
            time.sleep(max(0.0, tick - (time.time() - now)))
        out.write(json.dumps({"kind": "stop", "t": time.time()}) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
