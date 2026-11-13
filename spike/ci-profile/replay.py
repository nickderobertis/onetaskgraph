#!/usr/bin/env python3
"""Replay recent pull requests against finer Nx keys.

    python3 spike/ci-profile/replay.py HISTORY PRS INPUTS_AFTER INPUTS_BEFORE [--json OUT] [--root DIR]

HISTORY is `ciprofile.py history` output (each PR's first CI run, task by task, per job, written
by `models.FirstRunRecord`); PRS lists each PR's number and changed files (its squash commit's
diff); each INPUTS directory holds `nx show target inputs <task> --json` for each task, resolved
on the tree after #3956 and on its parent, so a PR is matched against the keys in force when it
ran.

For every task a PR's run executed it names the changed files that are inputs of that task — no
file at all means the task is not keyed on the diff: an uncached target, a `run` phase, or a key
that moved through a runtime input `nx show target inputs` does not list. Each `Option` is a
rule over those matches: a task every one of whose matched files the option takes out of its key
would have replayed, and the saving is that task's measured share of the job's wall clock (the
time it held a slot, split with whatever ran beside it) — an estimate of the job's shortening.

What the distribution journeys read is derived from the repository at --root, not listed here:
crate sources and manifests, and every file the commands of `scripts:distribution-check` and
`scripts:distribution-test` name literally, followed through each script those name in turn — a
lower bound, since a path a script builds at run time is not seen; the option's saving is an
upper bound for the same reason.

Exit status: 0 the replay printed; 2 a usage error or an input of a shape it does not read.
"""

from __future__ import annotations

import argparse
import json
import posixpath
import re
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from enum import Enum, StrEnum
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# E402: models sits beside this file and is found through the sys.path entry above.
from models import FirstRunRecord, Invalid, JobName, TaskId, history, items, obj, take, task_id  # noqa: E402

MERGED_3956 = 3956
USAGE_EXIT = 2
REPO_ROOT = Path(__file__).resolve().parents[2]
# Files no cargo test reads: formatter, linter and supply-chain configuration, prose.
NON_SEMANTIC = re.compile(
    r"^(biome\.json|\.gitignore|rustfmt\.toml|clippy\.toml|deny\.toml|llmlint\.yml|"
    r"AGENTS\.md|CLAUDE\.md|docs/.*|.*CHANGELOG\.md)$"
)
SOURCES = re.compile(r"^(Cargo\.(toml|lock)|crates/[^/]+/(src/.*|Cargo\.toml|build\.rs)|rust-toolchain\.toml)$")
NAMED_FILE = re.compile(r"(?<![\w/.-])((?:[\w.-]+/)*[\w.-]+\.(?:toml|json|lock|sh|py|mjs|ts|yml|yaml))(?![\w/])")
FOLLOWED = (".sh", ".py", ".mjs", ".ts")
WORKSPACE_MANIFESTS = frozenset(
    {"Cargo.lock", "Cargo.toml", "nx.json", ".tool-versions", "rust-toolchain.toml", ".cargo/config.toml"}
)
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


def distribution_named_files(root: Path) -> frozenset[str]:
    """The repository files the distribution journeys' commands name literally, followed through each script so named.

    A path a script builds at run time (`npm/platforms/${platform}/package.json`) or names as a directory
    (`sdks/python`) is not seen, so this is a lower bound on what the journeys read."""
    where = "scripts/project.json"
    targets = obj(obj(json.loads((root / where).read_text()), where).get("targets"), f"{where} targets")
    pending: list[str] = []
    for name in ("distribution-check", "distribution-test"):
        options = obj(obj(targets.get(name), f"{where} {name}").get("options"), f"{where} {name} options")
        commands = items(options["commands"], f"{name} commands") if "commands" in options else [options.get("command")]
        if not all(isinstance(c, str) and c for c in commands):
            raise Invalid(f"{where} {name}: every command must be text")
        pending += [n for c in commands for n in NAMED_FILE.findall(c)]
    found: set[str] = set()
    while pending:
        path = posixpath.normpath(pending.pop().removeprefix("./"))
        if path in found or not (root / path).is_file():
            continue
        found.add(path)
        if path.endswith(FOLLOWED):
            pending += NAMED_FILE.findall((root / path).read_text(encoding="utf-8", errors="replace"))
    return frozenset(found)


@dataclass(frozen=True)
class Publishable:
    """What a distribution journey is known to read: crate sources and manifests, and the files its scripts name."""

    named: frozenset[str]

    def __contains__(self, path: str) -> bool:
        return bool(SOURCES.match(path)) or path in self.named


def kind(path: str, task: TaskId) -> InputKind:
    own = task.split(":")[0]
    if m := re.match(r"crates/([^/]+)/(.*)", path):
        crate, rest = m.groups()
        side = "own" if crate == own else "dep"
        part = (
            "src"
            if rest.startswith(("src/", "build.rs"))
            else "tests"
            if rest.startswith("tests/")
            else "manifest"
            if rest in ("Cargo.toml", "project.json")
            else "other"
        )
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


@dataclass(frozen=True)
class Diff:
    """One PR's changed files."""

    files: frozenset[str]
    publishable: Publishable

    def replays(self, option: Option, task: TaskId, matched: list[str]) -> bool:
        """Whether `option` would have replayed this executed task."""
        kinds = {kind(f, task) for f in matched}
        rust = task.startswith("onetaskgraph")
        match option:
            case Option.NON_SEMANTIC_GLOBALS:
                return bool(matched) and rust and kinds == {InputKind.NON_SEMANTIC}
            case Option.DEPENDENCY_TESTS:
                return (
                    bool(matched)
                    and rust
                    and InputKind.DEP_TESTS in kinds
                    and kinds <= {InputKind.DEP_TESTS, InputKind.NON_SEMANTIC}
                )
            case Option.DISTRIBUTION_WHEN_PUBLISHABLE:
                return task in DISTRIBUTION and not any(f in self.publishable for f in self.files)


def repository_path(value: object, where: str) -> str:
    if not isinstance(value, str) or not value or value.startswith("/") or "\\" in value:
        raise Invalid(f"{where}: {value!r} is not a repository-relative path")
    if posixpath.normpath(value) != value or value.split("/")[0] == "..":
        raise Invalid(f"{where}: {value!r} is not a normalized repository path")
    return value


def parse_prs(raw: object) -> dict[int, frozenset[str]]:
    out = {}
    for entry in items(raw, "prs"):
        record = obj(entry, "prs entry")
        pr = take(record, "pr", int, "prs entry")
        if pr in out:
            raise Invalid(f"#{pr} is listed twice")
        out[pr] = frozenset(repository_path(f, f"#{pr} files") for f in items(record.get("files"), f"#{pr} files"))
    return out


def load_inputs(directory: Path) -> dict[TaskId, frozenset[str]]:
    if not directory.is_dir() or not any(directory.glob("*.json")):
        raise Invalid(f"{directory}: expected a directory of `nx show target inputs` records")
    out = {}
    for path in sorted(directory.glob("*.json")):
        record = obj(json.loads(path.read_text()), str(path))
        if path.stem.count("__") != 1:
            raise Invalid(f"{path}: an inputs file is named <project>__<target>.json")
        task = task_id(path.stem.replace("__", ":"), str(path))
        project, target = task.split(":")
        named = f"{record.get('project', project)}:{record.get('target', target)}"
        if named != task:
            raise Invalid(f"{path}: holds the inputs of {named}, not {task}")
        out[task] = frozenset(repository_path(f, str(path)) for f in items(record.get("files"), str(path)))
    return out


@dataclass(frozen=True)
class TaskReason:
    matched: tuple[str, ...] | None
    why: str
    share_s: float


@dataclass
class JobTotals:
    run: int = 0
    replayed: int = 0
    wall: float = 0.0


@dataclass
class ReplayResult:
    prs: int = 0
    totals: dict[JobName, JobTotals] = field(default_factory=lambda: {j: JobTotals() for j in JobName})
    saved: dict[Option, dict[JobName, float]] = field(default_factory=lambda: {o: defaultdict(float) for o in Option})
    by_kind: dict[JobName, dict[str, float]] = field(default_factory=lambda: {j: defaultdict(float) for j in JobName})
    reasons: dict[int, dict[JobName, dict[TaskId, TaskReason]]] = field(default_factory=dict)

    def to_json(self) -> dict:
        return {
            "prs": self.prs,
            "totals": {j.value: vars(t) for j, t in self.totals.items()},
            "saved_s": {o.value: {j.value: v for j, v in per.items()} for o, per in self.saved.items()},
            "match_kinds_s": {j.value: dict(v) for j, v in self.by_kind.items()},
            "rows": {
                str(pr): {
                    j.value: {t: {"matched": r.matched, "why": r.why, "share_s": r.share_s} for t, r in tasks.items()}
                    for j, tasks in jobs.items()
                }
                for pr, jobs in self.reasons.items()
            },
        }


def replay(
    runs: dict[int, FirstRunRecord],
    prs: dict[int, frozenset[str]],
    keys_after: dict[TaskId, frozenset[str]],
    keys_before: dict[TaskId, frozenset[str]],
    publishable: Publishable,
) -> ReplayResult:
    result = ReplayResult(prs=len(runs))
    for pr, first in runs.items():
        if pr not in prs:
            raise Invalid(f"#{pr} is in the history but not in the PR list")
        diff, keys = Diff(prs[pr], publishable), (keys_after if pr > MERGED_3956 else keys_before)
        result.reasons[pr] = {}
        for job, record in first.jobs.items():
            totals = result.totals[job]
            executed = {t: r for t, r in record.tasks.items() if not r.result.replayed}
            totals.run += len(executed)
            totals.replayed += len(record.tasks) - len(executed)
            totals.wall += record.wall_s
            reasons = result.reasons[pr][job] = {}
            for task, ran in executed.items():
                if task not in keys:
                    result.by_kind[job]["(not resolved)"] += ran.share_s
                    reasons[task] = TaskReason(None, "inputs not resolved for this target", ran.share_s)
                    continue
                matched = sorted(diff.files & keys[task])
                kinds = {kind(f, task) for f in matched}
                result.by_kind[job][most_specific(kinds).value if kinds else "(not keyed on the diff)"] += ran.share_s
                why = ",".join(k.value for k in sorted(kinds, key=lambda k: k.rank)) or (
                    "no changed file is an input: uncached target, a `run` phase or a runtime input"
                )
                reasons[task] = TaskReason(tuple(matched[:8]), why, ran.share_s)
                for option in Option:
                    if diff.replays(option, task, matched):
                        result.saved[option][job] += ran.share_s
    return result


def report(result: ReplayResult) -> None:
    n = max(result.prs, 1)
    print(f"{result.prs} pull requests, each PR's first CI run")
    print("job           tasks run  replayed  mean wall")
    for job, t in result.totals.items():
        print(f"  {job.value:12s} {t.run:9d} {t.replayed:9d}  {t.wall / n / 60:6.1f} min")
    print(
        "\nexecuted tasks' wall share by the most specific kind of changed input they matched (minutes over all PRs):"
    )
    for job, parts in result.by_kind.items():
        print(
            f"  {job.value}: " + "; ".join(f"{k} {v / 60:.0f}" for k, v in sorted(parts.items(), key=lambda kv: -kv[1]))
        )
    print("\noption: minutes of wall share the option would have replayed, total over the PRs (and per PR)")
    for option, per_job in result.saved.items():
        print(
            f"  {option.value}: "
            + "  ".join(
                f"{job.value} {per_job.get(job, 0) / 60:.1f} ({per_job.get(job, 0) / 60 / n:.2f}/PR)" for job in JobName
            )
        )


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        prog="replay.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    for name in ("history", "prs", "inputs_after", "inputs_before"):
        ap.add_argument(name, type=Path)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--root", type=Path, default=REPO_ROOT)
    args = ap.parse_args(argv)
    try:
        result = replay(
            history(json.loads(args.history.read_text())),
            parse_prs(json.loads(args.prs.read_text())),
            load_inputs(args.inputs_after),
            load_inputs(args.inputs_before),
            Publishable(distribution_named_files(args.root)),
        )
    except (OSError, ValueError) as error:
        print(f"replay: {type(error).__name__}: {error}", file=sys.stderr)
        return USAGE_EXIT
    report(result)
    if args.json:
        try:
            args.json.write_text(json.dumps(result.to_json(), indent=1))
        except OSError as error:
            print(f"replay: {type(error).__name__}: {error}", file=sys.stderr)
            return USAGE_EXIT
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
