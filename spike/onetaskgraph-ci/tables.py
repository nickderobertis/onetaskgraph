#!/usr/bin/env python3
"""Render the figures document `measure.py figures` wrote as the report's Markdown tables.

Usage: python3 spike/onetaskgraph-ci/tables.py <figures.json> [<local-run-dir>...]

Prints nothing it did not read from those files: per-job durations, per-run job counts,
CD per release, cycle-time phases, and — for each local run directory local-profile.sh
wrote — its wall clock and every Nx task's duration.
"""

import json
import sys
from pathlib import Path


def s(st, key):
    return "–" if not st or st.get(key) is None else f"{st[key]:g}"


def triple(st):
    return f"{s(st, 'median')} / {s(st, 'p90')} / {s(st, 'max')}"


def main() -> None:
    figures = json.loads(Path(sys.argv[1]).read_text())
    print("### Per job and matrix leg (seconds; median / p90 / max over successful runs)\n")
    print("| Workflow | Event | Job | OS | Successful | Duration | Wait from run created | Failed or cancelled |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for j in figures["per_job"]:
        d = j["duration_s_success"]
        print(f"| {j['workflow']} | {j['event']} | {j['job']} | {j['os']} | {d['n'] if d else 0} | {triple(d)} | "
              f"{triple(j['wait_from_run_created_s_success'])} | {j['failed_or_cancelled']} |")

    pr = figures["per_run"]
    ci = pr["ci_pull_request_runs"]
    print("\n### Jobs per run (executed on a runner; skipped and never-started jobs excluded)\n")
    print("| Population | n | Total | Linux | Windows | macOS |")
    print("| --- | --- | --- | --- | --- | --- |")
    print(f"| `ci.yml` PR run, successful | {ci['successful']} | {triple(ci['jobs_per_run_success'])} | "
          + " | ".join(triple(ci["jobs_per_run_by_os_success"][o]) for o in ("Linux", "Windows", "macOS")) + " |")
    h = pr["pr_head_commit_all_workflows"]
    print(f"| PR head commit, every workflow | {h['head_commits']} | {triple(h['jobs_per_head_commit'])} | "
          + " | ".join(triple(h["by_os"][o]) for o in ("Linux", "Windows", "macOS")) + " |")
    cd = figures["cd"]["summary"]
    print(f"| One release's CD runs | {cd['complete']} | {triple(cd['jobs_per_release'])} | "
          + " | ".join(triple(cd["jobs_per_release_by_os"][o]) for o in ("Linux", "Windows", "macOS")) + " |")
    print("\n| Population | Workflow | Jobs (median / p90 / max) |\n| --- | --- | --- |")
    for wf, st in h["by_workflow"].items():
        print(f"| PR head commit | {wf} | {triple(st)} |")
    for wf, st in cd["jobs_per_release_by_workflow"].items():
        print(f"| Release CD | {wf} | {triple(st)} |")
    p = pr["push_main_commit_all_workflows"]
    for wf, st in p["by_workflow"].items():
        print(f"| Push to main (every merge) | {wf} | {triple(st)} |")

    print("\n### Longest job (seconds; median / p90 / max)\n")
    print(f"- `ci.yml` PR run, successful ({ci['successful']} runs): {triple(ci['longest_job_s_success'])}")
    print(f"- One release's CD runs, all workflows ({cd['complete']} releases): {triple(cd['longest_job_s'])}")
    print(f"- One release's `release-plz.yml` and `release.yml` runs only: {triple(cd['longest_job_s_release_workflows_only'])}")

    c = figures["cycle"]["summary"]
    print(f"\n### Cycle time ({c['eligible']} of {c['merged_prs_in_window']} merged PRs eligible; seconds)\n")
    print("| Phase | Median | p90 | Max |\n| --- | --- | --- | --- |")
    for key, label in (("checks_s", "Last-commit checks"), ("merge_to_release_s", "Merge to release"),
                       ("cycle_s", "Sum"), ("release_cycle_time_max_per_release_s", "Max over one release's PRs")):
        print(f"| {label} | {s(c[key], 'median')} | {s(c[key], 'p90')} | {s(c[key], 'max')} |")

    for run_dir in sys.argv[2:]:
        wall = json.loads((Path(run_dir) / "wall.json").read_text())
        tasks = json.loads((Path(run_dir) / "tasks.json").read_text())
        print(f"\n### Local `{wall['command']}` ({Path(run_dir).name}): exit {wall['exit']}, {wall['wall_s']} s\n")
        print("| Nx task | Status | Seconds |\n| --- | --- | --- |")
        for t in sorted(tasks, key=lambda t: -t["seconds"]):
            print(f"| `{t['task']}` | {t['status']} | {t['seconds']} |")


if __name__ == "__main__":
    main()
