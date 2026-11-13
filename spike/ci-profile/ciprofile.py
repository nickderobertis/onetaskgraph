#!/usr/bin/env python3
"""The CI profile spike's one command.

    python3 spike/ci-profile/ciprofile.py profile <run id> [--jobs test-linux,test-macos,test-windows] [--json]

performs one measurement. It reads one hosted run — its jobs, each test job's log, the Nx
executions each job uploads and, on a demonstration run, the sampler and test-runner records —
and prints, per job:
- its wall clock divided into categories that sum to it, with an `unattributed` remainder;
- the rust-cache restore and the Nx entries the job stored;
- the Nx tasks by their share of the wall clock, and the latest-finishing task chain;
- on a demonstration run, the test step divided by process category, and per category the
  processes that ended, their summed lifetime and CPU;
- every test binary's seconds and the 20 slowest tests.

Reads are cached under target/ci-profile (or $CIPROFILE_CACHE); CIPROFILE_OFFLINE=1 reads the
cache alone.

Two more subcommands build the report:
    durations <since> <until>         median durations of successful test jobs of PR runs created in a window
    history <prs.json> --out FILE     each PR's first (or --which last) CI run, task by task, per job

Exit status: 0 the measurement printed; 2 a usage error; 3 a read refused below the rate floor
(wait for the reset and run it again); 4 a requested job did not finish in that run, or a
read failed or returned a record of a shape this harness does not read (named on standard error).
"""

from __future__ import annotations

import argparse
import asyncio
import json
import re
import statistics
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# E402: the harness's own modules are found through the sys.path entry above.
import ghcache  # noqa: E402
import jobprofile as prof  # noqa: E402
from models import (  # noqa: E402
    FirstRunRecord,
    Invalid,
    Job,
    JobName,
    JobRecord,
    LibtestEvent,
    LibtestKind,
    LibtestOutcome,
    NxExecution,
    Run,
    RunId,
    Seconds,
    TaskRecord,
    TestTime,
    items,
    obj,
    take,
    timestamp,
)

USAGE_EXIT, FLOOR_EXIT, MISSING_EXIT = 2, 3, 4


class JobMissing(LookupError):
    pass


@dataclass(frozen=True)
class TestTimes:
    per_binary: dict[str, Seconds]
    slowest: list[TestTime]
    count: int


async def executions_for(gh: ghcache.GitHub, run_id: RunId, job: str) -> tuple[NxExecution, ...] | None:
    for name in (f"nx-cache-entries-{job}", f"spike-nx-executions-{job}"):
        try:
            data = await gh.artifact_member(run_id, name, "executions.json")
        except ghcache.Offline:
            return None
        if data is not None:
            return NxExecution.report(json.loads(data))
    return None


async def spike_record(gh: ghcache.GitHub, run_id: RunId, job: str, member: str) -> list[str]:
    try:
        data = await gh.artifact_member(run_id, f"spike-profile-{job}", member)
    except ghcache.Offline:
        return []
    return data.decode("utf-8", "replace").splitlines() if data else []


def test_times(runner_lines: list[str], nextest: list[TestTime]) -> TestTimes:
    """Every test binary's seconds and the slowest tests, from the runner's record and nextest's log lines."""
    per_binary: dict[str, Seconds] = defaultdict(float)
    tests: list[TestTime] = list(nextest)
    for line in filter(str.strip, runner_lines):
        event = LibtestEvent.parse(json.loads(line))
        match (event.kind, event.outcome):
            case (LibtestKind.TEST, LibtestOutcome.OK | LibtestOutcome.FAILED) if event.exec_time is not None:
                tests.append(TestTime(event.binary, event.name or "", event.exec_time))
            case (LibtestKind.SUITE, LibtestOutcome.OK | LibtestOutcome.FAILED):
                per_binary[event.binary] += event.exec_time or 0.0
            case _:
                pass
    for t in nextest:
        per_binary[t.binary] += t.seconds
    return TestTimes(dict(per_binary), sorted(tests, key=lambda t: -t.seconds)[:20], len(tests))


def minutes(value: Seconds) -> str:
    return f"{value / 60:6.2f}"


@dataclass(frozen=True)
class JobView:
    """Everything one job's section of the measurement prints."""

    name: str
    profile: prof.JobProfile
    from_executions: bool
    division: prof.ProcessDivision | None
    times: TestTimes


def print_job(view: JobView, top: int) -> None:
    name, p, executions, division, times = view.name, view.profile, view.from_executions, view.division, view.times
    executed = [t for t in p.tasks if not t.result.replayed]
    print(
        f"\n== {name} (job {p.job.id}, {p.job.conclusion}) wall {p.wall / 60:.2f} min; Nx tasks: {len(executed)} run, "
        f"{len(p.tasks) - len(executed)} replayed [{'executions artifact' if executions else 'log headers'}]"
    )
    print("  category                          minutes   share")
    for k, v in sorted(p.buckets.items(), key=lambda kv: -kv[1]):
        print(f"  {k.value:32s} {minutes(v)}  {100 * v / p.wall:5.1f}%")
    print(f"  {'(sum)':32s} {minutes(sum(p.buckets.values()))}")
    if p.rust_cache:
        print(
            f"  rust-cache restore: {p.rust_cache.seconds} s, {p.rust_cache.size}, "
            f"{'hit' if p.rust_cache.hit else 'miss'}"
        )
    if p.stored:
        print(f"  Nx sidecar: stored {p.stored.entries} entries ({p.stored.bytes} bytes), replayed {p.stored.replayed}")
    print("  Nx tasks by share of wall clock (own seconds, result):")
    for task_id, share in sorted(p.task_share.items(), key=lambda kv: -kv[1])[:top]:
        t = next(x for x in p.tasks if x.id == task_id)
        compiled = f", compile {t.compile_end - t.start:.0f}s" if t.compile_end else ""
        print(f"    {task_id:55s} {minutes(share)} min  ({t.end - t.start:6.0f}s {t.result.value}{compiled})")
    if p.test_step:
        print(
            f"  latest-finishing chain ({len(p.latest_chain)} links, "
            f"{sum(t.end - t.start for t in p.latest_chain) / 60:.2f} min):"
        )
        for t in p.latest_chain:
            print(f"    +{(t.start - p.test_step.start) / 60:5.2f} → +{(t.end - p.test_step.start) / 60:5.2f}  {t.id}")
    if division:
        print(f"  sampler: test step {sum(division.seconds.values()) / 60:.2f} min by leaf process category:")
        for k, v in sorted(division.seconds.items(), key=lambda kv: -kv[1]):
            print(f"    {k:40s} {minutes(v)}  {100 * v / p.wall:5.1f}% of job")
        print("  sampler: processes that ended in the test step (count, summed lifetime s, CPU s):")
        for k, totals in sorted(division.exits.items(), key=lambda kv: -kv[1].lifetime):
            print(f"    {k.value:20s} {totals.count:6d} {totals.lifetime:9.1f} {totals.cpu:8.1f}")
    if times.per_binary:
        print(f"  test binaries ({len(times.per_binary)}, seconds):")
        for binary, secs in sorted(times.per_binary.items(), key=lambda kv: -kv[1])[:top]:
            print(f"    {binary:60s} {secs:8.1f}")
    if times.slowest:
        print(f"  slowest tests (of {times.count}):")
        for t in times.slowest:
            print(f"    {t.seconds:7.1f}s  {t.qualified[:110]}")


async def profile_run(gh: ghcache.GitHub, run_id: RunId, wanted: list[str], as_json: bool, top: int) -> int:
    run = await gh.run(run_id)
    jobs = {j.name: j for j in await gh.jobs(run_id)}
    print(f"run {run_id}  head {run.head_sha[:12]}  branch {run.head_branch}  created {run.created_at}")
    summary: dict = {"run": run_id, "head_sha": run.head_sha, "jobs": {}}
    missing = []
    for name in wanted:
        job = jobs.get(name)
        if job is None or not job.finished:
            missing.append(name)
            continue
        log = await gh.job_log(run_id, job)
        executions = await executions_for(gh, run_id, name)
        p = prof.profile_job(job, log, executions)
        samples = await spike_record(gh, run_id, name, "samples.jsonl")
        division = prof.by_process(prof.sampler_records(samples), p.test_step) if samples and p.test_step else None
        times = test_times(await spike_record(gh, run_id, name, "tests.jsonl"), p.nextest)
        print_job(JobView(name, p, executions is not None, division, times), top)
        summary["jobs"][name] = {
            "job_id": job.id,
            "wall_s": round(p.wall, 1),
            "buckets_s": {k.value: round(v, 1) for k, v in p.buckets.items()},
            "tasks": [
                {
                    "task": t.id,
                    "result": t.result.value,
                    "start": t.start,
                    "end": t.end,
                    "share_s": round(p.task_share.get(t.id, 0), 1),
                }
                for t in p.tasks
            ],
            "binaries_s": times.per_binary,
            "slowest": [[t.qualified, t.seconds] for t in times.slowest],
            "sampler": None
            if division is None
            else {"seconds": division.seconds, "exits": {k.value: vars(v) for k, v in division.exits.items()}},
            "rust_cache": None if p.rust_cache is None else vars(p.rust_cache),
            "stored": None if p.stored is None else vars(p.stored),
        }
    print(f"\n(GitHub reads this invocation: {gh.reads})")
    if as_json:
        target = gh.cache / f"profile-{run_id}.json"
        target.write_text(json.dumps(summary, indent=1))
        print(f"(json: {target})")
    if missing:
        print(
            f"ciprofile: run {run_id} has no finished {', '.join(missing)}; name jobs it ran with --jobs",
            file=sys.stderr,
        )
        return MISSING_EXIT
    return 0


async def durations(gh: ghcache.GitHub, since: str, until: str) -> int:
    runs = [
        r
        for r in await gh.runs_list(f"event=pull_request&created={since}..{until}", pages=3)
        if r.conclusion in ("success", "failure") and not r.head_branch.startswith("release-plz")
    ]
    per: dict[str, list[float]] = defaultdict(list)
    for run in runs:
        for job in await gh.jobs(run.id):
            if job.name in {j.value for j in JobName} and job.conclusion == "success" and job.finished:
                per[job.name].append(job.wall / 60)
    print(f"completed non-release PR runs created {since}..{until}: {len(runs)}; successful jobs:")
    for name in JobName:
        values = per[name.value]
        if len(values) >= 2:
            q = statistics.quantiles(values, n=4)
            print(
                f"  {name.value:13s} n={len(values):3d} median {statistics.median(values):5.1f} min  "
                f"p25 {q[0]:5.1f}  p75 {q[2]:5.1f}  max {max(values):5.1f}"
            )
        else:
            print(f"  {name.value:13s} n={len(values):3d} (too few for quartiles)")
    return 0


@dataclass(frozen=True)
class PullRequest:
    pr: int
    head: str
    branch: str

    @classmethod
    def parse(cls, raw: object) -> PullRequest:
        record = obj(raw, "prs.json entry")
        return cls(
            take(record, "pr", int, "prs.json"),
            take(record, "head", str, "prs.json"),
            take(record, "branch", str, "prs.json"),
        )


async def job_record(gh: ghcache.GitHub, run_id: RunId, job: Job) -> JobRecord:
    executions = await executions_for(gh, run_id, job.name)
    p = prof.profile_job(job, await gh.job_log(run_id, job), executions)
    return JobRecord(
        job.id,
        job.conclusion,
        p.wall,
        "executions" if executions is not None else "log",
        {k.value: v for k, v in p.buckets.items()},
        {t.id: TaskRecord(t.result, t.end - t.start, p.task_share.get(t.id, 0.0)) for t in p.tasks},
    )


def runs_of(pr: PullRequest, runs: list[Run]) -> list[Run]:
    """The PR's runs, oldest first: those GitHub attributes to it, and any on its final head."""
    mine = {r.id: r for r in runs if pr.pr in r.pull_requests or r.head_sha == pr.head}
    return sorted(mine.values(), key=lambda r: r.created_at)


async def history(gh: ghcache.GitHub, prs_file: Path, since: str, out_file: Path, which: str) -> int:
    prs = [PullRequest.parse(p) for p in items(json.loads(prs_file.read_text()), str(prs_file))]
    runs = await gh.runs_list(f"event=pull_request&created=>={since}", pages=4)
    out: dict[int, FirstRunRecord] = {}
    unmatched = []
    for pr in prs:
        mine = runs_of(pr, runs)
        if not mine:
            unmatched.append(pr.pr)
            continue
        run = mine[0] if which == "first" else mine[-1]
        jobs = {j.name: j for j in await gh.jobs(run.id)}
        record = FirstRunRecord(run.id, run.head_sha, run.created_at, run.conclusion, len(mine))
        for name in JobName:
            job = jobs.get(name.value)
            if job is not None and job.finished:
                record.jobs[name] = await job_record(gh, run.id, job)
        out[pr.pr] = record
        summary = "  ".join(
            f"{n.value}: {sum(not t.result.replayed for t in j.tasks.values())} run/"
            f"{sum(t.result.replayed for t in j.tasks.values())} replayed {j.wall_s / 60:.1f}m"
            for n, j in record.jobs.items()
        )
        print(f"#{pr.pr} run {run.id} ({run.conclusion}): {summary}")
        out_file.write_text(json.dumps({str(k): v.to_json() for k, v in out.items()}, indent=1))
    print(f"(GitHub reads this invocation: {gh.reads}; written {out_file})")
    if unmatched:
        print(f"ciprofile: no run since {since} for #{', #'.join(map(str, unmatched))}; widen --since", file=sys.stderr)
        return MISSING_EXIT
    return 0


def positive(text: str) -> int:
    if not re.fullmatch(r"[1-9][0-9]*", text):
        raise argparse.ArgumentTypeError(f"expected a positive whole number, not {text!r}")
    return int(text)


def instant(text: str) -> str:
    try:
        timestamp(text if "T" in text else f"{text}T00:00:00Z", "a window bound")
    except Invalid as error:
        raise argparse.ArgumentTypeError(str(error)) from error
    return text


def job_list(text: str) -> list[str]:
    names = [n for n in text.split(",") if n]
    valid = {j.value for j in JobName}
    if not names or any(n not in valid for n in names):
        raise argparse.ArgumentTypeError(f"--jobs takes a comma-separated list of {', '.join(sorted(valid))}")
    return names


def parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(
        prog="ciprofile.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("profile")
    a.add_argument("run", type=positive)
    a.add_argument("--jobs", type=job_list, default=[j.value for j in JobName])
    a.add_argument("--json", action="store_true")
    a.add_argument("--top", type=positive, default=15)
    d = sub.add_parser("durations")
    d.add_argument("since", type=instant)
    d.add_argument("until", type=instant)
    h = sub.add_parser("history")
    h.add_argument("prs_file", type=Path)
    h.add_argument("--since", type=instant, default="2026-11-01")
    h.add_argument("--out", type=Path, required=True)
    h.add_argument("--which", choices=("first", "last"), default="first")
    return ap


async def run_command(args: argparse.Namespace) -> int:
    gh = ghcache.GitHub()
    match args.cmd:
        case "profile":
            return await profile_run(gh, RunId(args.run), args.jobs, args.json, args.top)
        case "durations":
            if timestamp(args.until, "until") <= timestamp(args.since, "since"):
                print("ciprofile: the window's end must follow its start", file=sys.stderr)
                return USAGE_EXIT
            return await durations(gh, args.since, args.until)
        case "history":
            return await history(gh, args.prs_file, args.since, args.out, args.which)
    return USAGE_EXIT


def main(argv: list[str]) -> int:
    args = parser().parse_args(argv)
    try:
        return asyncio.run(run_command(args))
    except ghcache.RateFloor as error:
        print(f"ciprofile: {error}", file=sys.stderr)
        return FLOOR_EXIT
    except (ghcache.Offline, ghcache.ReadFailed, Invalid, OSError, ValueError) as error:
        print(f"ciprofile: {type(error).__name__}: {error}", file=sys.stderr)
        return MISSING_EXIT


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
