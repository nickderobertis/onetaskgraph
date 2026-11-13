#!/usr/bin/env python3
"""Price the measured prototypes over the 30-PR population, in elapsed job minutes.

    python3 spike/ci-profile/prototype_replay.py deltas SUMMARIES --control R,R --side NAME=R,R... --out FILE
        [--side-only JOB:TASK]... [--leave-out JOB:TASK]... [--graphs DIR]
    python3 spike/ci-profile/prototype_replay.py replay HISTORY PRS DELTAS --graphs DIR

`deltas` reads the run summaries `ciprofile.py profile <run> --json` wrote into SUMMARIES
(`profile-<run>.json`) for the control runs and each prototype side's runs. Per side and job it
writes, kept apart because they answer different questions:
- measured job figures, the evidence: each run's job wall clock and Nx gate span (first task start
  to last task end), control and side, with their means;
- `elapsed_saved_s`: for each Nx task, the control's mean elapsed interval less the side's — how
  much sooner the task itself finished. This is what the replay moves;
- `model_on_control_s` (with `--graphs`, which holds `run-<id>.json` from `taskgraphs.py`): the
  model's prediction of the gate shortening, applied to the control runs' own timelines — to set
  beside the measured change, as a check on the model;
- attribution and work, never a prediction of job minutes: `share_saved_s`, each task's wall
  share change, and `work_saved_s`, summed test-seconds per crate (Linux's nextest records).
The sides must have scheduled the same Nx tasks in every job, or the comparison is refused, naming
the tasks one side scheduled and the other did not; a task the prototype itself adds is let
through only when declared with `--side-only JOB:TASK`, and is recorded. A task both sides
scheduled but only one executed is compared over the runs that executed it and recorded under
`executed_by_one_side`. A task that ended for a reason the comparison is not about (a prototype
that fails a journey unrelated to what it measures, ending it early) is left out of both sides'
timelines with `--leave-out JOB:TASK`, recorded under `left_out`; the job wall clock cannot be
corrected for it and is reported as measured.

`replay` estimates, for every PR of HISTORY (`ciprofile.py history` output, each task with its
interval), how much sooner each job would have ended had the prototype been in place, with
`critical_path.py`: the PR's own run re-timed over Nx's task graph at that PR's commit (`--graphs`,
`pr-<n>.json`), the changed tasks taking their measured `elapsed_saved_s` less. Which tasks a PR's
prototype changes:
- `pyramid`: the `onetaskgraph-e2e` tasks its run executed;
- `split`: a PR changing `onetaskgraph-core` outside the moved template files changes the core
  tasks its run executed; one changing a moved file also runs the template crate's tasks, beside
  the gate from its start; any other PR changes nothing.
It prints, per side and job, the measured job figures and the estimated shortening per PR over all
PRs and over the typical PR (one whose diff changes a crate's `src/`, `tests/`, `build.rs` or
`Cargo.toml`). Those are a model's estimates; the measured figures are the evidence.

Exit status: 0 printed or written; 2 a usage error, an input of a shape it does not read, or
sides whose task selections differ.
"""

from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

# E402: the harness's own modules are found through the sys.path entry above.
from critical_path import Interval, executed_deps, gate_span, predicted_shortening  # noqa: E402
from models import FirstRunRecord, Invalid, JobName, history, items, obj, positive, seconds  # noqa: E402
from replay import parse_prs  # noqa: E402

USAGE_EXIT = 2
SIDES = ("pyramid", "split")
E2E_TASKS = frozenset({"onetaskgraph-e2e:test", "onetaskgraph-e2e:coverage"})
CORE_PREFIX = "onetaskgraph-core:"
TEMPLATE_PREFIX = "onetaskgraph-template:"
# What the split moves out of the engine crate: a change to one of these selects the new crate.
MOVED = re.compile(
    r"crates/onetaskgraph-core/(src/template/|src/yaml\.rs$|tests/templates\.rs$|tests/fixtures/template-front-matter)"
)
CORE = re.compile(r"crates/onetaskgraph-core/")
TYPICAL = re.compile(r"crates/[^/]+/(src/|tests/|build\.rs$|Cargo\.toml$)")
WORK_BINARY = re.compile(r"^instrumented:(?P<crate>[\w-]+)::")


def run_ids(text: str) -> list[int]:
    ids = [positive(int(x), "run id") for x in text.split(",") if x]
    if not ids or len(set(ids)) != len(ids):
        raise argparse.ArgumentTypeError("a comma-separated list of distinct run ids")
    return ids


@dataclass(frozen=True)
class JobSummary:
    wall_s: float
    intervals: dict[str, Interval]
    shares: dict[str, float]
    work: dict[str, float]
    scheduled: frozenset[str]


def summary(directory: Path, run: int, leave_out: frozenset[tuple[str, str]] = frozenset()) -> dict[str, JobSummary]:
    path = directory / f"profile-{run}.json"
    record = obj(json.loads(path.read_text()), str(path))
    out = {}
    for name, raw_job in obj(record.get("jobs"), f"{path} jobs").items():
        JobName(name)
        job = obj(raw_job, f"{path} {name}")
        intervals: dict[str, Interval] = {}
        shares: dict[str, float] = {}
        scheduled: set[str] = set()
        for raw_task in items(job.get("tasks"), f"{path} {name} tasks"):
            task = obj(raw_task, f"{path} {name} task")
            if (name, str(task.get("task"))) in leave_out:
                continue
            scheduled.add(str(task.get("task")))
            if task.get("result") != "executed":
                continue
            where = f"{path} {name} {task.get('task')}"
            start, end = seconds(task.get("start"), f"{where} start"), seconds(task.get("end"), f"{where} end")
            intervals[str(task.get("task"))] = (start, end)
            shares[str(task.get("task"))] = seconds(task.get("share_s"), f"{where} share")
        work: dict[str, float] = defaultdict(float)
        for binary, spent in obj(job.get("binaries_s") or {}, f"{path} {name} binaries").items():
            if m := WORK_BINARY.match(binary):
                work[m.group("crate")] += seconds(spent, f"{path} {name} {binary}")
        wall = seconds(job.get("wall_s"), f"{path} {name} wall")
        out[name] = JobSummary(wall, intervals, shares, dict(work), frozenset(scheduled))
    return out


def mean_of(values: list[dict[str, float]]) -> dict[str, float]:
    """Each key's mean over the records that hold it."""
    keys = {k for v in values for k in v}
    return {k: statistics.mean(v[k] for v in values if k in v) for k in keys}


def job_figures(present: list[JobSummary]) -> dict:
    walls = [round(p.wall_s, 1) for p in present]
    spans = [round(gate_span(p.intervals), 1) for p in present]
    return {
        "wall_s": round(statistics.mean(walls), 1),
        "gate_span_s": round(statistics.mean(spans), 1),
        "runs_wall_s": walls,
        "runs_gate_span_s": spans,
    }


def load_graph(directory: Path, key: str) -> dict[str, list[str]]:
    path = directory / f"{key}.json"
    record = obj(json.loads(path.read_text()), str(path))
    return {
        str(k): [str(d) for d in items(v, f"{path} {k}")] for k, v in obj(record.get("dependencies"), str(path)).items()
    }


def elapsed(p: JobSummary) -> dict[str, float]:
    return {t: e - s for t, (s, e) in p.intervals.items()}


def added_cost(measured: list[JobSummary], added: list[str]) -> dict[str, float]:
    """A task only the prototype runs costs its whole mean elapsed interval: a negative saving."""
    means = mean_of([elapsed(p) for p in measured])
    return {t: -round(means[t], 1) for t in added}


@dataclass(frozen=True)
class Comparison:
    """What a comparison declares: the tasks the prototype adds, the tasks left out of both sides, the graphs."""

    side_only: frozenset[tuple[str, str]] = frozenset()
    leave_out: frozenset[tuple[str, str]] = frozenset()
    graphs: Path | None = None


def deltas(
    directory: Path, control: list[int], sides: dict[str, list[int]], declared: Comparison | None = None
) -> dict:
    declared = declared or Comparison()
    side_only, leave_out, graphs = declared.side_only, declared.leave_out, declared.graphs
    base = [summary(directory, r, leave_out) for r in control]
    out: dict = {"control": control, "sides": {}}
    for side, runs in sides.items():
        measured = [summary(directory, r, leave_out) for r in runs]
        per_job: dict[str, dict[str, Any]] = {}
        for job in JobName:
            c = [s[job.value] for s in base if job.value in s]
            m = [s[job.value] for s in measured if job.value in s]
            if not c or not m:
                continue
            c_sched = frozenset().union(*(p.scheduled for p in c))
            m_sched = frozenset().union(*(p.scheduled for p in m))
            added = {t for j, t in side_only if j == job.value}
            if c_sched - m_sched or (m_sched - c_sched) - added:
                raise Invalid(
                    f"{side} {job.value}: the selections differ (control only {sorted(c_sched - m_sched)},"
                    f" {side} only {sorted((m_sched - c_sched) - added)}); hold them equal before comparing"
                )
            c_exec, m_exec = {t for p in c for t in p.intervals}, {t for p in m for t in p.intervals}

            def diff(get, keys=None, c=c, m=m) -> dict[str, float]:
                before, after = mean_of([get(p) for p in c]), mean_of([get(p) for p in m])
                names = sorted(set(before) & set(after)) if keys is None else sorted(keys)
                return {k: round(before.get(k, 0.0) - after.get(k, 0.0), 1) for k in names}

            per_job[job.value] = {
                "runs": [len(c), len(m)],
                "control": job_figures(c),
                "side": job_figures(m),
                "side_only": sorted(m_sched - c_sched),
                "left_out": sorted(t for j, t in leave_out if j == job.value),
                "executed_by_one_side": {"control": sorted(c_exec - m_exec), side: sorted((m_exec - c_exec) & c_sched)},
                "elapsed_saved_s": (saved := diff(elapsed) | added_cost(m, sorted((m_sched - c_sched) & m_exec))),
                "share_saved_s": diff(lambda p: p.shares),
                "work_saved_s": diff(lambda p: p.work, {k for p in c + m for k in p.work}),
            }
            if graphs is not None:
                model = []
                for run, summaries in zip(control, base, strict=True):
                    if job.value in summaries:
                        p = summaries[job.value]
                        deps = executed_deps(load_graph(graphs, f"run-{run}"), set(p.intervals))
                        changed = {t: v for t, v in saved.items() if t in p.intervals or t in m_sched - c_sched}
                        model.append(round(predicted_shortening(p.intervals, deps, changed), 1))
                per_job[job.value]["model_on_control_s"] = model
        out["sides"][side] = {"runs": runs, "jobs": per_job}
    return out


def changed_tasks(side: str, files: frozenset[str], executed: set[str], elapsed: dict[str, float]) -> dict[str, float]:
    """The tasks a prototype changes in one PR's job, each with how much sooner it finishes."""
    if side == "pyramid":
        return {t: elapsed.get(t, 0.0) for t in executed & E2E_TASKS}
    if side == "split":
        moved = any(MOVED.match(f) for f in files)
        if not moved and not any(CORE.match(f) for f in files):
            return {}
        changed = {t: elapsed.get(t, 0.0) for t in executed if t.startswith(CORE_PREFIX)}
        if moved:
            changed.update({t: v for t, v in elapsed.items() if t.startswith(TEMPLATE_PREFIX)})
        return changed
    raise Invalid(f"no replay rule for side {side!r}")


def replay(history_raw: object, prs_raw: object, measured: dict, graphs: Path) -> list[str]:
    runs: dict[int, FirstRunRecord] = history(history_raw)
    prs = parse_prs(prs_raw)
    lines = []
    for side, record in obj(measured.get("sides"), "deltas sides").items():
        if side not in SIDES:
            raise Invalid(f"deltas name an unknown side {side!r}")
        jobs = obj(record.get("jobs"), f"{side} jobs")
        lines.append(f"{side}: runs {record.get('runs')} against control {measured.get('control')}")
        for job in JobName:
            if job.value not in jobs:
                continue
            j = jobs[job.value]
            elapsed = {t: float(v) for t, v in obj(j.get("elapsed_saved_s"), f"{side} {job.value}").items()}
            per_pr, typical, unpredicted = [], [], 0
            for pr, first in runs.items():
                if pr not in prs:
                    raise Invalid(f"#{pr} is in the history but not in the PR list")
                record_job = first.jobs.get(job)
                tasks = record_job.tasks if record_job else {}
                executed = {str(t): r for t, r in tasks.items() if not r.result.replayed}
                if any(r.start is None or r.end is None for r in executed.values()):
                    unpredicted += 1
                    continue
                intervals = {t: (float(r.start or 0.0), float(r.end or 0.0)) for t, r in executed.items()}
                deps = executed_deps(load_graph(graphs, f"pr-{pr}"), set(executed))
                value = predicted_shortening(intervals, deps, changed_tasks(side, prs[pr], set(executed), elapsed))
                per_pr.append(value)
                if any(TYPICAL.match(f) for f in prs[pr]):
                    typical.append(value)
            c, s = j["control"], j["side"]

            def minutes(values: list[float]) -> str:
                return ", ".join(f"{v / 60:.1f}" for v in values)

            lines.append(
                f"  {job.value:12s} measured gate span (min) {minutes(c['runs_gate_span_s'])} ->"
                f" {minutes(s['runs_gate_span_s'])}, wall {minutes(c['runs_wall_s'])} -> {minutes(s['runs_wall_s'])};"
                f" estimated shortening {statistics.mean(per_pr) / 60 if per_pr else 0.0:+.2f} min/PR over"
                f" {len(per_pr)}, {statistics.mean(typical) / 60 if typical else 0.0:+.2f} over the {len(typical)}"
                f" typical ({unpredicted} without intervals)"
            )
    return lines


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        prog="prototype_replay.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = ap.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("deltas")
    d.add_argument("summaries", type=Path)
    d.add_argument("--control", type=run_ids, required=True)
    d.add_argument("--side", action="append", required=True)
    d.add_argument("--out", type=Path, required=True)
    d.add_argument("--side-only", action="append", default=[], help="JOB:TASK the prototype itself adds")
    d.add_argument("--leave-out", action="append", default=[], help="JOB:TASK left out of both sides' timelines")
    d.add_argument("--graphs", type=Path, help="taskgraphs.py output holding run-<id>.json for the control runs")
    r = sub.add_parser("replay")
    for name in ("history", "prs", "deltas"):
        r.add_argument(name, type=Path)
    r.add_argument("--graphs", type=Path, required=True, help="taskgraphs.py output holding pr-<n>.json")
    args = ap.parse_args(argv)
    try:
        if args.cmd == "deltas":
            sides = {}
            for item in args.side:
                name, _, ids = item.partition("=")
                if name not in SIDES:
                    ap.error(f"--side takes {'|'.join(SIDES)}=R,R, not {item!r}")
                sides[name] = run_ids(ids)
            declared, left = set(), set()
            for flag, item in [("--side-only", i) for i in args.side_only] + [
                ("--leave-out", i) for i in args.leave_out
            ]:
                job, _, task = item.partition(":")
                if job not in {j.value for j in JobName} or ":" not in task:
                    ap.error(f"{flag} takes JOB:PROJECT:TARGET, not {item!r}")
                (declared if flag == "--side-only" else left).add((job, task))
            comparison = Comparison(frozenset(declared), frozenset(left), args.graphs)
            measured = deltas(args.summaries, args.control, sides, comparison)
            args.out.write_text(json.dumps(measured, indent=1) + "\n")
            return 0
        lines = replay(
            json.loads(args.history.read_text()),
            json.loads(args.prs.read_text()),
            obj(json.loads(args.deltas.read_text()), "deltas"),
            args.graphs,
        )
        print("\n".join(lines))
        return 0
    except (Invalid, OSError, ValueError, KeyError, argparse.ArgumentTypeError) as error:
        print(f"prototype_replay: {type(error).__name__}: {error}", file=sys.stderr)
        return USAGE_EXIT


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
