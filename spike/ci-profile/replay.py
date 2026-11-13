#!/usr/bin/env python3
"""Replay recent pull requests against finer Nx keys.

    python3 spike/ci-profile/replay.py HISTORY PRS INPUTS_AFTER INPUTS_BEFORE [--json OUT]

HISTORY is `ciprofile.py history` output (each PR's first CI run, task by task, per job); PRS
lists each PR's number and changed files (its squash commit's diff); each INPUTS directory holds
`nx show target inputs <task> --json` for each task, resolved on the tree after #3956 and on its
parent, so a PR is matched against the keys in force when it ran.

For every task a PR's run executed it names the changed files that are inputs of that task — no
file at all means the task is not keyed on the diff: an uncached target, a `run` phase, or a key
that moved through a runtime input `nx show target inputs` does not list. Each `Option` is a
rule over those matches: a task every one of whose matched files the option takes out of its key
would have replayed, and the saving is that task's measured share of the job's wall clock (the
time it held a slot, split with whatever ran beside it) — an estimate of the job's shortening.

Exit status: 0 the replay printed; 2 a usage error or an input of a shape it does not read.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from dataclasses import dataclass
from enum import Enum, StrEnum
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# E402: models sits beside this file and is found through the sys.path entry above.
from models import Invalid, JobName, TaskId, items, obj, seconds, take  # noqa: E402

MERGED_3956 = 3956
USAGE_EXIT = 2
# Files no cargo test reads: formatter, linter and supply-chain configuration, prose.
NON_SEMANTIC = re.compile(r"^(biome\.json|\.gitignore|rustfmt\.toml|clippy\.toml|deny\.toml|llmlint\.yml|"
                          r"AGENTS\.md|CLAUDE\.md|docs/.*|.*CHANGELOG\.md)$")
# What the distribution journeys build and install.
PUBLISHABLE = re.compile(r"^(Cargo\.(toml|lock)|crates/[^/]+/(src/.*|Cargo\.toml|build\.rs)|pyproject\.toml|"
                         r"package\.json|bun\.lock|uv\.lock|npm/.*|sdks/.*|release-targets\.toml|release-plz\.toml|"
                         r"config/registry-interfaces\.toml|scripts/(release.*|.*distribution.*|set-version.*|"
                         r"npm-registry-auth.*|install.*)|rust-toolchain\.toml|\.cargo/config\.toml)$")
WORKSPACE_MANIFESTS = frozenset({"Cargo.lock", "Cargo.toml", "nx.json", ".tool-versions", "rust-toolchain.toml",
                                 ".cargo/config.toml"})
DISTRIBUTION = frozenset({"scripts:distribution-test", "scripts:distribution-check"})


class InputKind(StrEnum):
    """What a changed input is to the task it re-ran, most specific first."""
    OWN_SRC = "own-src"
    OWN_TESTS = "own-tests"
    OWN_MANIFEST = "own-manifest"
    DEP_SRC = "dep-src"
    DEP_TESTS = "dep-tests"
    DEP_MANIFEST = "dep-manifest"
    WORKSPACE_MANIFEST = "workspace-manifest"
    SCRIPTS = "scripts"
    GITHUB = ".github"
    SDKS = "sdks"
    README = "readme"
    OWN_OTHER = "own-other"
    DEP_OTHER = "dep-other"
    NON_SEMANTIC = "non-semantic-global"
    OTHER = "other"

    @property
    def rank(self) -> int:
        return list(InputKind).index(self)


class Option(Enum):
    NON_SEMANTIC_GLOBALS = "non-semantic-globals-out-of-rust-keys"
    DEPENDENCY_TESTS = "dependents-skip-dependency-tests"
    DISTRIBUTION_WHEN_PUBLISHABLE = "distribution-only-when-publishable-changes"


def kind(path: str, task: TaskId) -> InputKind:
    own = task.split(":")[0]
    if m := re.match(r"crates/([^/]+)/(.*)", path):
        crate, rest = m.groups()
        side = "own" if crate == own else "dep"
        part = ("src" if rest.startswith(("src/", "build.rs")) else "tests" if rest.startswith("tests/")
                else "manifest" if rest in ("Cargo.toml", "project.json") else "other")
        return InputKind(f"{side}-{part}")
    if NON_SEMANTIC.match(path):
        return InputKind.NON_SEMANTIC
    if path in WORKSPACE_MANIFESTS:
        return InputKind.WORKSPACE_MANIFEST
    if path == "README.md":
        return InputKind.README
    for prefix, found in (("scripts/", InputKind.SCRIPTS), (".github/", InputKind.GITHUB), ("sdks/", InputKind.SDKS)):
        if path.startswith(prefix):
            return found
    return InputKind.OTHER


def most_specific(kinds: set[InputKind]) -> InputKind:
    return min(kinds, key=lambda k: k.rank)


def replays(option: Option, task: TaskId, matched: list[str], files: set[str]) -> bool:
    """Whether `option` would have replayed this executed task."""
    kinds = {kind(f, task) for f in matched}
    rust = task.startswith("onetaskgraph")
    match option:
        case Option.NON_SEMANTIC_GLOBALS:
            return bool(matched) and rust and kinds == {InputKind.NON_SEMANTIC}
        case Option.DEPENDENCY_TESTS:
            return bool(matched) and rust and InputKind.DEP_TESTS in kinds and kinds <= {InputKind.DEP_TESTS, InputKind.NON_SEMANTIC}
        case Option.DISTRIBUTION_WHEN_PUBLISHABLE:
            return task in DISTRIBUTION and not any(PUBLISHABLE.match(f) for f in files)


@dataclass(frozen=True)
class RanTask:
    id: TaskId
    executed: bool
    seconds: float
    share: float


@dataclass(frozen=True)
class FirstRun:
    pr: int
    run: int
    jobs: dict[JobName, tuple[float, str | None, tuple[RanTask, ...]]]


def parse_history(raw: object) -> list[FirstRun]:
    out = []
    for pr, entry in obj(raw, "history").items():
        record = obj(entry, f"history #{pr}")
        jobs = {}
        for name, job in obj(record.get("jobs", {}), f"#{pr} jobs").items():
            j = obj(job, f"#{pr} {name}")
            tasks = tuple(RanTask(TaskId(t), obj(v, t).get("result") == "executed",
                                  seconds(take(v, "seconds", (int, float), t), t), seconds(take(v, "share_s", (int, float), t), t))
                          for t, v in obj(j.get("tasks"), f"#{pr} {name} tasks").items())
            jobs[JobName(name)] = (seconds(take(j, "wall_s", (int, float), name), name), j.get("conclusion"), tasks)
        out.append(FirstRun(int(pr), take(record, "run", int, f"#{pr}"), jobs))
    return out


def parse_prs(raw: object) -> dict[int, set[str]]:
    out = {}
    for entry in items(raw, "prs"):
        record = obj(entry, "prs entry")
        files = items(record.get("files"), f"#{record.get('pr')} files")
        if not all(isinstance(f, str) for f in files):
            raise Invalid(f"#{record.get('pr')}: files must be paths")
        out[take(record, "pr", int, "prs entry")] = set(files)
    return out


def load_inputs(directory: Path) -> dict[TaskId, set[str]]:
    out = {}
    for path in sorted(directory.glob("*.json")):
        files = items(obj(json.loads(path.read_text()), str(path)).get("files"), str(path))
        if not all(isinstance(f, str) for f in files):
            raise Invalid(f"{path}: files must be paths")
        out[TaskId(path.stem.replace("__", ":"))] = set(files)
    return out


def replay(history: list[FirstRun], prs: dict[int, set[str]], after: dict[TaskId, set[str]],
           before: dict[TaskId, set[str]]) -> dict:
    saved: dict[Option, dict[JobName, float]] = {o: defaultdict(float) for o in Option}
    by_kind: dict[JobName, dict[str, float]] = {j: defaultdict(float) for j in JobName}
    totals = {j: {"run": 0, "replayed": 0, "wall": 0.0} for j in JobName}
    rows = []
    for first in history:
        if first.pr not in prs:
            raise Invalid(f"#{first.pr} is in the history but not in the PR list")
        files, keys = prs[first.pr], (after if first.pr > MERGED_3956 else before)
        row = {"pr": first.pr, "run": first.run, "files": len(files), "jobs": {}}
        for job, (wall, conclusion, tasks) in first.jobs.items():
            executed = [t for t in tasks if t.executed]
            totals[job]["run"] += len(executed)
            totals[job]["replayed"] += len(tasks) - len(executed)
            totals[job]["wall"] += wall
            reasons = {}
            for task in executed:
                if task.id not in keys:
                    by_kind[job]["(not resolved)"] += task.share
                    reasons[task.id] = {"matched": None, "why": "inputs not resolved for this target"}
                    continue
                matched = sorted(files & keys[task.id])
                kinds = {kind(f, task.id) for f in matched}
                by_kind[job][most_specific(kinds).value if kinds else "(not keyed on the diff)"] += task.share
                reasons[task.id] = {"matched": matched[:8], "matched_count": len(matched), "share_s": task.share,
                                    "seconds": task.seconds, "why": ",".join(k.value for k in sorted(kinds, key=lambda k: k.rank))
                                    or "no changed file is an input: uncached target, a `run` phase or a runtime input"}
                for option in Option:
                    if replays(option, task.id, matched, files):
                        saved[option][job] += task.share
            row["jobs"][job.value] = {"run": len(executed), "replayed": len(tasks) - len(executed), "wall_s": wall,
                                      "conclusion": conclusion, "tasks": reasons}
        rows.append(row)
    return {"rows": rows, "totals": {j.value: v for j, v in totals.items()},
            "saved_s": {o.value: {j.value: v for j, v in per.items()} for o, per in saved.items()},
            "match_kinds_s": {j.value: dict(v) for j, v in by_kind.items()}}


def report(result: dict) -> None:
    n = len(result["rows"])
    print(f"{n} pull requests, each PR's first CI run")
    print("job           tasks run  replayed  mean wall")
    for job, t in result["totals"].items():
        print(f"  {job:12s} {t['run']:9d} {t['replayed']:9d}  {t['wall'] / max(n, 1) / 60:6.1f} min")
    print("\nexecuted tasks' wall share by the most specific kind of changed input they matched (minutes over all PRs):")
    for job, parts in result["match_kinds_s"].items():
        print(f"  {job}: " + "; ".join(f"{k} {v / 60:.0f}" for k, v in sorted(parts.items(), key=lambda kv: -kv[1])))
    print("\noption: minutes of wall share the option would have replayed, total over the PRs (and per PR)")
    for option, per_job in result["saved_s"].items():
        print(f"  {option}: " + "  ".join(f"{job.value} {per_job.get(job.value, 0) / 60:.1f} "
                                           f"({per_job.get(job.value, 0) / 60 / max(n, 1):.2f}/PR)" for job in JobName))


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(prog="replay.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for name in ("history", "prs", "inputs_after", "inputs_before"):
        ap.add_argument(name, type=Path)
    ap.add_argument("--json", type=Path)
    args = ap.parse_args(argv)
    try:
        result = replay(parse_history(json.loads(args.history.read_text())), parse_prs(json.loads(args.prs.read_text())),
                        load_inputs(args.inputs_after), load_inputs(args.inputs_before))
    except (OSError, ValueError) as error:
        print(f"replay: {type(error).__name__}: {error}", file=sys.stderr)
        return USAGE_EXIT
    report(result)
    if args.json:
        args.json.write_text(json.dumps(result, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
