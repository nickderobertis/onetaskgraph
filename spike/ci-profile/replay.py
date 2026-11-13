#!/usr/bin/env python3
"""Replay recent pull requests against finer Nx keys.

    python3 spike/ci-profile/replay.py <history.json> <prs.json> <inputs dir after #3956> <inputs dir before> [--json OUT]

history.json is `ciprofile.py history` output (each PR's first CI run, task by task, per job);
prs.json lists each PR's number and changed files (its squash commit's diff); the inputs dirs
hold `nx show target inputs <task> --json` for each task, resolved on the tree after #3956 and
on #3956's parent, so a PR is matched against the keys that were in force when it ran.

For every task a PR's run executed it names the changed files that are inputs of that task
(no file at all means the task is not keyed on the diff: an uncached target, a `run` phase,
or a dependency). Each option below is a rule over those matches; a task every one of whose
matched files the option removes from its key would have replayed instead, and the wall
clock saved is that task's measured share of the job (the time it held a slot, split with
whatever ran beside it) — an estimate of the job's shortening, labelled as one in the report.
"""

from __future__ import annotations

import json
import re
import sys
from collections import defaultdict
from pathlib import Path

JOBS = ("test-linux", "test-macos", "test-windows")
MERGED_3956 = 3956

# Files no cargo test reads: formatter, linter and supply-chain configuration, prose.
NON_SEMANTIC = re.compile(r"^(biome\.json|\.gitignore|rustfmt\.toml|clippy\.toml|deny\.toml|llmlint\.yml|"
                          r"AGENTS\.md|CLAUDE\.md|docs/.*|.*CHANGELOG\.md)$")
# What the distribution journeys build and install: crate sources and manifests, the SDKs,
# the npm launcher, the release scripts and declarations.
PUBLISHABLE = re.compile(r"^(Cargo\.(toml|lock)|crates/[^/]+/(src/.*|Cargo\.toml|build\.rs)|pyproject\.toml|"
                         r"package\.json|bun\.lock|uv\.lock|npm/.*|sdks/.*|release-targets\.toml|release-plz\.toml|"
                         r"config/registry-interfaces\.toml|scripts/(release.*|.*distribution.*|set-version.*|"
                         r"npm-registry-auth.*|install.*)|rust-toolchain\.toml|\.cargo/config\.toml)$")


def crate_of(task: str) -> str:
    return task.split(":")[0]


def kind(path: str, task: str) -> str:
    own = crate_of(task)
    m = re.match(r"crates/([^/]+)/(.*)", path)
    if m:
        crate, rest = m.groups()
        part = "src" if rest.startswith(("src/", "build.rs")) else "tests" if rest.startswith("tests/") else \
            "manifest" if rest in ("Cargo.toml", "project.json") else "other"
        return f"{'own' if crate == own else 'dep'}-{part}"
    if NON_SEMANTIC.match(path):
        return "non-semantic-global"
    if path in ("Cargo.lock", "Cargo.toml", "nx.json", ".tool-versions", "rust-toolchain.toml", ".cargo/config.toml"):
        return "workspace-manifest"
    if path == "README.md":
        return "readme"
    for prefix in ("scripts/", ".github/", "sdks/"):
        if path.startswith(prefix):
            return prefix.rstrip("/")
    return "other"


PRIORITY = ["own-src", "own-tests", "own-manifest", "dep-src", "dep-tests", "dep-manifest", "workspace-manifest",
            "scripts", ".github", "sdks", "readme", "own-other", "dep-other", "non-semantic-global", "other"]


def most_specific(kinds: set[str]) -> str:
    return min(kinds, key=lambda k: PRIORITY.index(k) if k in PRIORITY else len(PRIORITY))


def load_inputs(directory: str) -> dict[str, set[str]]:
    out = {}
    for f in Path(directory).glob("*.json"):
        try:
            out[f.stem.replace("__", ":")] = set(json.loads(f.read_text())["files"])
        except (ValueError, KeyError):
            pass
    return out


def options(task: str, matched: list[str], files: set[str]) -> dict[str, bool]:
    """Which options would have replayed this executed task."""
    kinds = {kind(f, task) for f in matched}
    out = {}
    out["non-semantic-globals-out-of-rust-keys"] = bool(matched) and task.startswith("onetaskgraph") and \
        all(kind(f, task) == "non-semantic-global" for f in matched)
    out["dependents-skip-dependency-tests"] = bool(matched) and task.startswith("onetaskgraph") and \
        all(k in ("dep-tests", "non-semantic-global") for k in kinds) and "dep-tests" in kinds
    out["distribution-only-when-publishable-changes"] = task in ("scripts:distribution-test", "scripts:distribution-check") \
        and not any(PUBLISHABLE.match(f) for f in files)
    return out


def main() -> int:
    history = json.loads(Path(sys.argv[1]).read_text())
    prs = {str(p["pr"]): p for p in json.loads(Path(sys.argv[2]).read_text())}
    after, before = load_inputs(sys.argv[3]), load_inputs(sys.argv[4])
    out_path = next((sys.argv[i + 1] for i, a in enumerate(sys.argv) if a == "--json"), None)
    saved = defaultdict(lambda: defaultdict(float))  # option -> job -> seconds
    match_kinds = defaultdict(lambda: defaultdict(float))  # job -> kind -> executed task-seconds
    totals = defaultdict(lambda: {"run": 0, "replayed": 0, "wall": 0.0})
    rows = []
    for pr, entry in history.items():
        files = set(prs[pr]["files"])
        inputs = after if int(pr) > MERGED_3956 else before
        line = {"pr": int(pr), "run": entry["run"], "files": len(files), "jobs": {}}
        for job in JOBS:
            j = entry["jobs"].get(job)
            if not j:
                continue
            executed = {t: v for t, v in j["tasks"].items() if v["result"] == "executed"}
            totals[job]["run"] += len(executed)
            totals[job]["replayed"] += len(j["tasks"]) - len(executed)
            totals[job]["wall"] += j["wall_s"]
            reasons = {}
            for task, v in executed.items():
                if task not in inputs:
                    reasons[task] = {"matched": None, "why": "inputs not resolved (a dependency task or a target outside the resolved set)"}
                    match_kinds[job]["(not resolved)"] += v["share_s"]
                    continue
                matched = sorted(files & inputs[task])
                kinds = {kind(f, task) for f in matched}
                why = "no changed file is an input: uncached target or a `run` phase" if not matched else \
                    ",".join(sorted(kinds, key=lambda k: PRIORITY.index(k) if k in PRIORITY else 99))
                reasons[task] = {"matched": matched[:8], "matched_count": len(matched), "why": why,
                                 "share_s": v["share_s"], "seconds": v["seconds"]}
                match_kinds[job][most_specific(kinds) if matched else "(not keyed on the diff)"] += v["share_s"]
                for option, replays in options(task, matched, files).items():
                    if replays:
                        saved[option][job] += v["share_s"]
            line["jobs"][job] = {"run": len(executed), "replayed": len(j["tasks"]) - len(executed),
                                 "wall_s": j["wall_s"], "conclusion": j["conclusion"], "tasks": reasons}
        rows.append(line)
    n = len(rows)
    print(f"{n} pull requests, each PR's first CI run")
    print("job           tasks run  replayed  mean wall")
    for job in JOBS:
        t = totals[job]
        print(f"  {job:12s} {t['run']:9d} {t['replayed']:9d}  {t['wall'] / max(n, 1) / 60:6.1f} min")
    print("\nexecuted tasks' wall share by the most specific kind of changed input they matched (minutes over all PRs):")
    for job in JOBS:
        parts = sorted(match_kinds[job].items(), key=lambda kv: -kv[1])
        print(f"  {job}: " + "; ".join(f"{k} {v / 60:.0f}" for k, v in parts))
    print("\noption: minutes of wall share the option would have replayed, total over the PRs (and per PR)")
    for option, per_job in saved.items():
        print(f"  {option}: " + "  ".join(f"{job} {per_job[job] / 60:.1f} ({per_job[job] / 60 / max(n, 1):.2f}/PR)" for job in JOBS))
    if out_path:
        Path(out_path).write_text(json.dumps({"rows": rows, "saved_s": {k: dict(v) for k, v in saved.items()},
                                              "match_kinds_s": {k: dict(v) for k, v in match_kinds.items()}}, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
