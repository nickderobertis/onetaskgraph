#!/usr/bin/env python3
"""Price the measured prototypes over the 30-PR population.

    python3 spike/ci-profile/prototype_replay.py deltas SUMMARIES --control R,R --side NAME=R,R... --out FILE
    python3 spike/ci-profile/prototype_replay.py replay HISTORY PRS DELTAS

`deltas` reads the run summaries `ciprofile.py profile <run> --json` wrote into SUMMARIES
(`profile-<run>.json`) for the control runs and each prototype side's runs, and writes per side
and per job: the mean job wall clock of each, and for every Nx task the control's mean wall share
less the side's (seconds the side saved; negative where it cost). A task one side ran and the
other did not counts as zero there.

`replay` applies each side's measured task deltas to every PR of HISTORY (`ciprofile.py history`
output) by the PR's own diff (PRS) and the tasks its run executed:
- `pyramid`: the `onetaskgraph-e2e` tasks a PR's run executed save what they saved measured;
- `split`: a PR changing `onetaskgraph-core` outside the moved template files saves the core
  tasks' measured delta on the core tasks it executed (the template crate is not selected); a PR
  changing a moved file also runs the template crate's tasks, which cost their measured share;
  any other PR saves nothing.
It prints, per side and job, the mean saving per PR over all PRs and over the typical PR (one
whose diff changes a crate's `src/`, `tests/`, `build.rs` or `Cargo.toml`).

Exit status: 0 printed or written; 2 a usage error or an input of a shape it does not read.
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
from models import Invalid, JobName, history, items, obj, positive, seconds  # noqa: E402
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


def run_ids(text: str) -> list[int]:
    ids = [positive(int(x), "run id") for x in text.split(",") if x]
    if not ids or len(set(ids)) != len(ids):
        raise argparse.ArgumentTypeError("a comma-separated list of distinct run ids")
    return ids


@dataclass(frozen=True)
class JobSummary:
    wall_s: float
    shares: dict[str, float]


def summary(directory: Path, run: int) -> dict[str, JobSummary]:
    path = directory / f"profile-{run}.json"
    record = obj(json.loads(path.read_text()), str(path))
    out = {}
    for name, raw_job in obj(record.get("jobs"), f"{path} jobs").items():
        JobName(name)
        job = obj(raw_job, f"{path} {name}")
        shares: dict[str, float] = defaultdict(float)
        for raw_task in items(job.get("tasks"), f"{path} {name} tasks"):
            task = obj(raw_task, f"{path} {name} task")
            shares[str(task.get("task"))] += seconds(task.get("share_s"), f"{path} {name} share")
        out[name] = JobSummary(seconds(job.get("wall_s"), f"{path} {name} wall"), dict(shares))
    return out


def mean_job(summaries: list[dict[str, JobSummary]], job: str) -> tuple[float, dict[str, float], int]:
    present = [s[job] for s in summaries if job in s]
    if not present:
        return 0.0, {}, 0
    tasks = {t for p in present for t in p.shares}
    return (
        statistics.mean(p.wall_s for p in present),
        {t: statistics.mean(p.shares.get(t, 0.0) for p in present) for t in tasks},
        len(present),
    )


def deltas(directory: Path, control: list[int], sides: dict[str, list[int]]) -> dict:
    base = [summary(directory, r) for r in control]
    out: dict = {"control": control, "sides": {}}
    for side, runs in sides.items():
        measured = [summary(directory, r) for r in runs]
        per_job = {}
        for job in JobName:
            c_wall, c_shares, c_n = mean_job(base, job.value)
            s_wall, s_shares, s_n = mean_job(measured, job.value)
            if not c_n or not s_n:
                continue
            tasks = sorted(set(c_shares) | set(s_shares))
            per_job[job.value] = {
                "control_wall_s": round(c_wall, 1),
                "side_wall_s": round(s_wall, 1),
                "runs": [c_n, s_n],
                "saved_s": {t: round(c_shares.get(t, 0.0) - s_shares.get(t, 0.0), 1) for t in tasks},
            }
        out["sides"][side] = {"runs": runs, "jobs": per_job}
    return out


def pr_saving(side: str, files: frozenset[str], executed: set[str], saved: dict[str, float]) -> float:
    if side == "pyramid":
        return sum(saved.get(t, 0.0) for t in executed & E2E_TASKS)
    if side == "split":
        moved = any(MOVED.match(f) for f in files)
        if not moved and not any(CORE.match(f) for f in files):
            return 0.0
        core = sum(saved.get(t, 0.0) for t in executed if t.startswith(CORE_PREFIX))
        template = sum(v for t, v in saved.items() if t.startswith(TEMPLATE_PREFIX)) if moved else 0.0
        return core + template
    raise Invalid(f"no replay rule for side {side!r}")


def replay(history_raw: object, prs_raw: object, measured: dict) -> list[str]:
    runs, prs = history(history_raw), parse_prs(prs_raw)
    lines = []
    for side, record in obj(measured.get("sides"), "deltas sides").items():
        if side not in SIDES:
            raise Invalid(f"deltas name an unknown side {side!r}")
        jobs = obj(record.get("jobs"), f"{side} jobs")
        lines.append(f"{side}: runs {record.get('runs')} against control {measured.get('control')}")
        for job in JobName:
            if job.value not in jobs:
                continue
            saved = {t: float(v) for t, v in obj(jobs[job.value].get("saved_s"), f"{side} {job.value}").items()}
            per_pr, typical = [], []
            for pr, first in runs.items():
                if pr not in prs:
                    raise Invalid(f"#{pr} is in the history but not in the PR list")
                record_job = first.jobs.get(job)
                executed: set[str] = (
                    {str(t) for t, r in record_job.tasks.items() if not r.result.replayed} if record_job else set()
                )
                value = pr_saving(side, prs[pr], executed, saved)
                per_pr.append(value)
                if any(TYPICAL.match(f) for f in prs[pr]):
                    typical.append(value)
            j = jobs[job.value]
            lines.append(
                f"  {job.value:12s} job wall {j['control_wall_s'] / 60:5.1f} -> {j['side_wall_s'] / 60:5.1f} min;"
                f" replayed saving {statistics.mean(per_pr) / 60:+.2f} min/PR over {len(per_pr)},"
                f" {statistics.mean(typical) / 60 if typical else 0.0:+.2f} min/PR over the {len(typical)} typical"
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
        print(
            "\n".join(
                replay(
                    json.loads(args.history.read_text()),
                    json.loads(args.prs.read_text()),
                    obj(json.loads(args.deltas.read_text()), "deltas"),
                )
            )
        )
        return 0
    except (Invalid, OSError, ValueError, KeyError, argparse.ArgumentTypeError) as error:
        print(f"prototype_replay: {type(error).__name__}: {error}", file=sys.stderr)
        return USAGE_EXIT


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
