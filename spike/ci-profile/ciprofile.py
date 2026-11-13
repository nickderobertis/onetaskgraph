#!/usr/bin/env python3
"""The CI profile spike's one command.

    python3 spike/ci-profile/ciprofile.py profile <run id> [--jobs test-linux,test-macos,test-windows] [--json]

performs one measurement: it reads one hosted run (job steps, job logs, the Nx executions
each job uploads, and — on a spike demonstration run — the sampler and test-runner records),
and prints, per job, its wall clock divided into categories that sum to it, the Nx tasks with
their share of the wall clock, the critical path, the rust-cache restore, the Nx entries the
job stored, and the slowest tests and test binaries. Reads are cached under target/ci-profile
(or $CIPROFILE_CACHE) and refused below the dispatch's rate-limit floor.

Other subcommands, used to build the report:
    durations <since> <until>      median job durations of successful PR runs in a window
    reuse <pr>...                   tasks replayed and run per job on each PR's first run
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ghcache  # noqa: E402
import profile as prof  # noqa: E402

TEST_JOBS = ("test-linux", "test-macos", "test-windows")


def executions_for(run_id: int, job: str):
    data = ghcache.artifact_member(run_id, f"nx-cache-entries-{job}", "executions.json")
    if data is None:
        data = ghcache.artifact_member(run_id, f"spike-nx-executions-{job}", "executions.json")
    return json.loads(data) if data else None


def spike_records(run_id: int, job: str):
    names = [a["name"] for a in ghcache.artifacts(run_id)]
    if f"spike-profile-{job}" not in names:
        return None, None
    samples = ghcache.artifact_member(run_id, f"spike-profile-{job}", "samples.jsonl")
    tests = ghcache.artifact_member(run_id, f"spike-profile-{job}", "tests.jsonl")
    return (samples.decode().splitlines() if samples else None,
            tests.decode().splitlines() if tests else None)


def fmt_min(seconds: float) -> str:
    return f"{seconds / 60:6.2f}"


def test_records(lines):
    """(binary seconds, test seconds) from the spike test runner's record."""
    per_test, per_binary = [], defaultdict(float)
    for raw in lines or []:
        try:
            e = json.loads(raw)
        except ValueError:
            continue
        if e.get("type") == "test" and e.get("event") in ("ok", "failed") and "exec_time" in e:
            per_test.append((f"{e['binary'].rsplit('-', 1)[0]}::{e['name']}", float(e["exec_time"])))
        if e.get("type") == "suite" and e.get("event") in ("ok", "failed"):
            per_binary[e["binary"].rsplit("-", 1)[0]] += float(e.get("exec_time", 0))
    return per_binary, per_test


def profile_run(run_id: int, jobs_wanted, as_json: bool, top: int) -> int:
    run = ghcache.run(run_id)
    out = {"run": run_id, "head_sha": run["head_sha"], "head_branch": run["head_branch"],
           "event": run["event"], "created_at": run["created_at"], "jobs": {}}
    jobs = {j["name"]: j for j in ghcache.jobs(run_id)}
    print(f"run {run_id}  head {run['head_sha'][:12]}  branch {run['head_branch']}  created {run['created_at']}")
    for name in jobs_wanted:
        job = jobs.get(name)
        if not job or job.get("conclusion") in (None, "skipped"):
            print(f"\n{name}: not run in this run")
            continue
        log = ghcache.job_log(run_id, job["id"])
        executions = executions_for(run_id, name)
        p = prof.profile_job(job, log, executions)
        samples, tests = spike_records(run_id, name)
        sampled = prof.load_sampler(samples, p.test_step) if samples and p.test_step else None
        per_binary, per_test = test_records(tests)
        if not per_test and p.nextest:
            per_test = list(p.nextest)
            for test, secs in p.nextest:
                per_binary[test.split("$")[0]] += secs
        executed = [t for t in p.tasks if t.result == "executed"]
        replayed = [t for t in p.tasks if t.result != "executed"]
        print(f"\n== {name} (job {job['id']}, {job['conclusion']}) wall {p.wall / 60:.2f} min; "
              f"Nx tasks: {len(executed)} run, {len(replayed)} replayed"
              f" [{'executions artifact' if executions else 'log headers'}]")
        print("  category                          minutes   share")
        for k, v in sorted(p.buckets.items(), key=lambda kv: -kv[1]):
            print(f"  {k:32s} {fmt_min(v)}  {100 * v / p.wall:5.1f}%")
        print(f"  {'(sum)':32s} {fmt_min(sum(p.buckets.values()))}")
        if p.rust_cache:
            print(f"  rust-cache restore: {p.rust_cache}")
        if p.stored:
            print(f"  Nx sidecar: stored {p.stored[0]} entries ({p.stored[1]} bytes), replayed {p.stored[2]}")
        print("  Nx tasks by share of wall clock (own seconds, result):")
        for task_name, share in sorted(p.task_share.items(), key=lambda kv: -kv[1])[:top]:
            t = next(x for x in p.tasks if x.name == task_name)
            comp = f", compile {t.compile_end - t.start:.0f}s" if t.compile_end else ""
            print(f"    {task_name:55s} {fmt_min(share)} min  ({t.end - t.start:6.0f}s {t.result}{comp})")
        crit = sum(b - a for _, a, b in p.critical)
        print(f"  critical path ({len(p.critical)} links, {crit / 60:.2f} min of tasks):")
        for task_name, a, b in p.critical:
            print(f"    +{(a - p.test_step[0]) / 60:5.2f} → +{(b - p.test_step[0]) / 60:5.2f}  {task_name}")
        if sampled:
            total = sum(sampled["buckets"].values())
            print(f"  sampler: test step {total / 60:.2f} min by leaf process category:")
            for k, v in sorted(sampled["buckets"].items(), key=lambda kv: -kv[1]):
                print(f"    {k:40s} {fmt_min(v)}  {100 * v / p.wall:5.1f}% of job")
            print("  sampler: processes that exited in the test step (count, summed lifetime s):")
            for k, v in sorted(sampled["exits"].items(), key=lambda kv: -kv[1]["seconds"]):
                print(f"    {k:20s} {v['count']:6d} {v['seconds']:9.1f}")
        if per_binary:
            print(f"  test binaries ({len(per_binary)}, seconds):")
            for b, secs in sorted(per_binary.items(), key=lambda kv: -kv[1])[:top]:
                print(f"    {b:60s} {secs:8.1f}")
        if per_test:
            print(f"  slowest tests (of {len(per_test)}):")
            for test, secs in sorted(per_test, key=lambda kv: -kv[1])[:20]:
                print(f"    {secs:7.1f}s  {test[:110]}")
        out["jobs"][name] = {
            "job_id": job["id"], "wall_s": round(p.wall, 1),
            "buckets_s": {k: round(v, 1) for k, v in p.buckets.items()},
            "tasks": [{"task": t.name, "result": t.result, "start": t.start, "end": t.end,
                       "compile_end": t.compile_end, "share_s": round(p.task_share.get(t.name, 0), 1)} for t in p.tasks],
            "critical": p.critical, "rust_cache": p.rust_cache, "stored": p.stored,
            "sampler": sampled, "binaries_s": dict(per_binary),
            "slowest": sorted(per_test, key=lambda kv: -kv[1])[:20],
            "steps": p.steps,
        }
    print(f"\n(GitHub reads this invocation: {ghcache.requests_made()})")
    if as_json:
        target = ghcache.cache_dir() / f"profile-{run_id}.json"
        target.write_text(json.dumps(out, indent=1))
        print(f"(json: {target})")
    return 0


def durations(since: str, until: str) -> int:
    runs = [r for r in ghcache.runs_list(f"event=pull_request&created={since}..{until}")
            if r["conclusion"] in ("success", "failure") and not r["head_branch"].startswith("release-plz")]
    per = defaultdict(list)
    for r in runs:
        for j in ghcache.jobs(r["id"]):
            if j["name"] in TEST_JOBS and j["conclusion"] == "success":
                per[j["name"]].append((prof.ts(j["completed_at"]) - prof.ts(j["started_at"])) / 60)
    print(f"completed non-release PR runs created {since}..{until}: {len(runs)}; successful jobs:")
    for name in TEST_JOBS:
        v = per[name]
        if v:
            print(f"  {name:13s} n={len(v):3d} median {statistics.median(v):5.1f} min  "
                  f"p25 {statistics.quantiles(v, n=4)[0]:5.1f}  p75 {statistics.quantiles(v, n=4)[2]:5.1f}  max {max(v):5.1f}")
    return 0


def job_tasks(run_id: int, job: dict) -> dict:
    """One job's Nx tasks with their result and own seconds, and its wall clock."""
    log = ghcache.job_log(run_id, job["id"])
    executions = executions_for(run_id, job["name"])
    p = prof.profile_job(job, log, executions)
    return {"job_id": job["id"], "conclusion": job["conclusion"], "wall_s": round(p.wall, 1),
            "source": "executions" if executions else "log",
            "buckets_s": {k: round(v, 1) for k, v in p.buckets.items()},
            "tasks": {t.name: {"result": t.result, "seconds": round(t.end - t.start, 1),
                               "share_s": round(p.task_share.get(t.name, 0), 1)} for t in p.tasks}}


def history(prs_file: str, since: str, out_file: str, which: str) -> int:
    """For each PR in prs_file, its first (or last) CI run's test jobs, task by task."""
    prs = json.loads(Path(prs_file).read_text())
    runs = ghcache.runs_list(f"event=pull_request&created=>={since}", pages=4)
    by_pr: dict[int, list[dict]] = defaultdict(list)
    for r in runs:
        for pr in r.get("pull_requests") or []:
            by_pr[pr["number"]].append(r)
        if not r.get("pull_requests"):
            for p in prs:
                if r["head_sha"] == p["head"] or r["head_branch"] == p["branch"]:
                    by_pr[p["pr"]].append(r)
    out = {}
    for p in prs:
        candidates = sorted({r["id"]: r for r in by_pr.get(p["pr"], [])}.values(), key=lambda r: r["created_at"])
        if not candidates:
            print(f"#{p['pr']}: no run found since {since}")
            continue
        run = candidates[0] if which == "first" else candidates[-1]
        jobs = {j["name"]: j for j in ghcache.jobs(run["id"])}
        entry = {"run": run["id"], "head_sha": run["head_sha"], "created_at": run["created_at"],
                 "conclusion": run["conclusion"], "runs_of_pr": len(candidates), "jobs": {}}
        for name in TEST_JOBS:
            job = jobs.get(name)
            if job and job.get("completed_at") and job.get("conclusion") not in ("skipped", None):
                entry["jobs"][name] = job_tasks(run["id"], job)
        out[p["pr"]] = entry
        summary = "  ".join(f"{n}: {len([t for t in j['tasks'].values() if t['result'] == 'executed'])} run/"
                            f"{len([t for t in j['tasks'].values() if t['result'] != 'executed'])} replayed "
                            f"{j['wall_s'] / 60:.1f}m" for n, j in entry["jobs"].items())
        print(f"#{p['pr']} run {run['id']} ({run['conclusion']}): {summary}")
        Path(out_file).write_text(json.dumps(out, indent=1))
    print(f"(GitHub reads this invocation: {ghcache.requests_made()}; written {out_file})")
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("profile")
    a.add_argument("run", type=int)
    a.add_argument("--jobs", default=",".join(TEST_JOBS))
    a.add_argument("--json", action="store_true")
    a.add_argument("--top", type=int, default=15)
    d = sub.add_parser("durations")
    d.add_argument("since")
    d.add_argument("until")
    h = sub.add_parser("history")
    h.add_argument("prs_file")
    h.add_argument("--since", default="2026-11-01")
    h.add_argument("--out", required=True)
    h.add_argument("--which", choices=("first", "last"), default="first")
    args = ap.parse_args(argv)
    try:
        if args.cmd == "history":
            return history(args.prs_file, args.since, args.out, args.which)
        if args.cmd == "profile":
            return profile_run(args.run, args.jobs.split(","), args.json, args.top)
        if args.cmd == "durations":
            return durations(args.since, args.until)
    except ghcache.RateFloor as error:
        print(f"ciprofile: {error}", file=sys.stderr)
        return 3
    return 2


if __name__ == "__main__":
    sys.exit(main())
