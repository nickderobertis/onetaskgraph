#!/usr/bin/env python3
"""Price the measured prototypes over the 30-PR population, in elapsed job minutes.

    python3 spike/ci-profile/prototype_replay.py deltas SUMMARIES --control R,R --side NAME=R,R... --out FILE
    python3 spike/ci-profile/prototype_replay.py replay HISTORY PRS DELTAS

`deltas` reads the run summaries `ciprofile.py profile <run> --json` wrote into SUMMARIES
(`profile-<run>.json`) for the control runs and each prototype side's runs. Per side and job it
writes three different things, kept apart because they answer different questions:
- `elapsed_saved_s`: for each Nx task, the control's mean elapsed interval (its end less its
  start) less the side's — how much sooner the task itself finishes. This is what the replay
  moves.
- measured job figures: the mean job wall clock, the Nx gate's span (first task start to last task
  end) and the latest-finishing chain's span, control and side — the job shortening the runs
  themselves showed, noise included.
- attribution and work, never a prediction of job minutes: `share_saved_s`, the change in each
  task's wall share (an instant split among the tasks running then), and `work_saved_s`, the
  change in summed test-seconds per crate (concurrent test work, from the nextest records the
  summary carries as `binaries_s`).
The sides must have run the same Nx tasks as the control in every job, or the comparison is
refused, naming the tasks one side ran and the other did not.

`replay` predicts, for every PR of HISTORY (`ciprofile.py history` output, each task with its
interval), how much sooner each job would have ended had the prototype been in place: the tasks
the prototype changes finish earlier (or later) by their measured `elapsed_saved_s`; a changed task
on the job's latest-finishing chain moves the chain's end, every task after it on the chain moving
with it, and a changed task off the chain moves only its own end; the job ends at the latest of
the two. Which tasks a PR's prototype changes:
- `pyramid`: the `onetaskgraph-e2e` tasks its run executed;
- `split`: a PR changing `onetaskgraph-core` outside the moved template files changes the core
  tasks its run executed; one changing a moved file also runs the template crate's tasks, beside
  the gate from its start; any other PR changes nothing.
Off-chain followers are not shifted and contention is not modelled; both are stated beside the
figures. It prints, per side and job, the mean predicted shortening per PR over all PRs and over
the typical PR (one whose diff changes a crate's `src/`, `tests/`, `build.rs` or `Cargo.toml`), and
how many PR jobs had the intervals to be predicted.

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

sys.path.insert(0, str(Path(__file__).resolve().parent))

# E402: the harness's own modules are found through the sys.path entry above.
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
SHORTEST_LINK = 0.5  # seconds: as jobprofile's chain, a replay holds no slot worth chaining

Interval = tuple[float, float]


def run_ids(text: str) -> list[int]:
    ids = [positive(int(x), "run id") for x in text.split(",") if x]
    if not ids or len(set(ids)) != len(ids):
        raise argparse.ArgumentTypeError("a comma-separated list of distinct run ids")
    return ids


def latest_chain(intervals: dict[str, Interval]) -> list[str]:
    """Walking back from the last end, the task finishing latest before each pointer (jobprofile's rule)."""
    pool = {t: iv for t, iv in intervals.items() if iv[1] - iv[0] > SHORTEST_LINK}
    if not pool:
        return []
    pointer, chain = max(e for _, e in pool.values()), []
    while before := [t for t, (s, e) in pool.items() if e <= pointer + 1 and s < pointer - 0.5]:
        link = max(before, key=lambda t: (pool[t][1], -pool[t][0]))
        chain.append(link)
        pointer = pool.pop(link)[0]
    return list(reversed(chain))


def span(intervals: dict[str, Interval], chain: list[str] | None = None) -> float:
    picked = [intervals[t] for t in chain] if chain is not None else list(intervals.values())
    return max(e for _, e in picked) - min(s for s, _ in picked) if picked else 0.0


@dataclass(frozen=True)
class JobSummary:
    wall_s: float
    intervals: dict[str, Interval]
    shares: dict[str, float]
    work: dict[str, float]


def summary(directory: Path, run: int) -> dict[str, JobSummary]:
    path = directory / f"profile-{run}.json"
    record = obj(json.loads(path.read_text()), str(path))
    out = {}
    for name, raw_job in obj(record.get("jobs"), f"{path} jobs").items():
        JobName(name)
        job = obj(raw_job, f"{path} {name}")
        intervals: dict[str, Interval] = {}
        shares: dict[str, float] = {}
        for raw_task in items(job.get("tasks"), f"{path} {name} tasks"):
            task = obj(raw_task, f"{path} {name} task")
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
        out[name] = JobSummary(seconds(job.get("wall_s"), f"{path} {name} wall"), intervals, shares, dict(work))
    return out


def mean_of(values: list[dict[str, float]]) -> dict[str, float]:
    keys = {k for v in values for k in v}
    return {k: statistics.mean(v.get(k, 0.0) for v in values) for k in keys}


def job_figures(present: list[JobSummary]) -> dict[str, float]:
    return {
        "wall_s": round(statistics.mean(p.wall_s for p in present), 1),
        "gate_span_s": round(statistics.mean(span(p.intervals) for p in present), 1),
        "chain_span_s": round(statistics.mean(span(p.intervals, latest_chain(p.intervals)) for p in present), 1),
    }


def deltas(directory: Path, control: list[int], sides: dict[str, list[int]]) -> dict:
    base = [summary(directory, r) for r in control]
    out: dict = {"control": control, "sides": {}}
    for side, runs in sides.items():
        measured = [summary(directory, r) for r in runs]
        per_job = {}
        for job in JobName:
            c = [s[job.value] for s in base if job.value in s]
            m = [s[job.value] for s in measured if job.value in s]
            if not c or not m:
                continue
            c_tasks, m_tasks = {t for p in c for t in p.intervals}, {t for p in m for t in p.intervals}
            if c_tasks != m_tasks:
                raise Invalid(
                    f"{side} {job.value}: the selections differ (control only {sorted(c_tasks - m_tasks)},"
                    f" {side} only {sorted(m_tasks - c_tasks)}); hold them equal before comparing"
                )

            def diff(get, c=c, m=m) -> dict[str, float]:
                before, after = mean_of([get(p) for p in c]), mean_of([get(p) for p in m])
                return {k: round(before.get(k, 0.0) - after.get(k, 0.0), 1) for k in sorted(set(before) | set(after))}

            per_job[job.value] = {
                "runs": [len(c), len(m)],
                "control": job_figures(c),
                "side": job_figures(m),
                "elapsed_saved_s": diff(lambda p: {t: e - s for t, (s, e) in p.intervals.items()}),
                "share_saved_s": diff(lambda p: p.shares),
                "work_saved_s": diff(lambda p: p.work),
            }
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


def predicted_shortening(intervals: dict[str, Interval], changed: dict[str, float]) -> float:
    """How much sooner the job's last task ends when each changed task finishes `changed[t]` sooner."""
    if not intervals:
        return 0.0
    end = max(e for _, e in intervals.values())
    first = min(s for s, _ in intervals.values())
    chain = set(latest_chain(intervals))
    chain_end = end - sum(v for t, v in changed.items() if t in chain)
    others = [e - changed.get(t, 0.0) for t, (_, e) in intervals.items() if t not in chain]
    # A task the PR's run did not have (the split's new crate) runs beside the gate from its start.
    added = [first - v for t, v in changed.items() if t not in intervals]
    return end - max([chain_end, *others, *added])


def replay(history_raw: object, prs_raw: object, measured: dict) -> list[str]:
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
                value = predicted_shortening(intervals, changed_tasks(side, prs[pr], set(executed), elapsed))
                per_pr.append(value)
                if any(TYPICAL.match(f) for f in prs[pr]):
                    typical.append(value)
            c, s = j["control"], j["side"]
            lines.append(
                f"  {job.value:12s} measured: wall {c['wall_s'] / 60:5.1f} -> {s['wall_s'] / 60:5.1f} min,"
                f" chain {c['chain_span_s'] / 60:5.1f} -> {s['chain_span_s'] / 60:5.1f} min;"
                f" predicted shortening {statistics.mean(per_pr) / 60 if per_pr else 0.0:+.2f} min/PR over"
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
    r = sub.add_parser("replay")
    for name in ("history", "prs", "deltas"):
        r.add_argument(name, type=Path)
    args = ap.parse_args(argv)
    try:
        if args.cmd == "deltas":
            sides = {}
            for item in args.side:
                name, _, ids = item.partition("=")
                if name not in SIDES:
                    ap.error(f"--side takes {'|'.join(SIDES)}=R,R, not {item!r}")
                sides[name] = run_ids(ids)
            args.out.write_text(json.dumps(deltas(args.summaries, args.control, sides), indent=1) + "\n")
            return 0
        lines = replay(
            json.loads(args.history.read_text()),
            json.loads(args.prs.read_text()),
            obj(json.loads(args.deltas.read_text()), "deltas"),
        )
        print("\n".join(lines))
        return 0
    except (Invalid, OSError, ValueError, KeyError, argparse.ArgumentTypeError) as error:
        print(f"prototype_replay: {type(error).__name__}: {error}", file=sys.stderr)
        return USAGE_EXIT


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
