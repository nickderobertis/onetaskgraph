#!/usr/bin/env python3
"""Spike harness: where onetaskgraph's CI and CD time goes.

One command reads a repository's GitHub Actions run, job, pull-request and release records,
read-only, through this host's `gh`, and writes the per-job, per-run and per-PR cycle-time
figures as one JSON document:

    python3 spike/onetaskgraph-ci/measure.py figures \
        --repo nickderobertis/onetaskgraph --since 2026-09-15 --until 2026-10-15 \
        --raw-dir <raw> --log <requests.jsonl> --out <figures.json> [--offline]

Every response is kept verbatim under --raw-dir, keyed by its request path, and a response
already there is never requested again. With --offline nothing is requested at all: the
figures are computed from --raw-dir alone, so the committed raw responses reproduce them.
A figure whose inputs are absent from --raw-dir is left out and counted under `missing`.

Before every request the log records the core quota GitHub's `/rate_limit` endpoint last
reported and how many core requests were made since it said so. The endpoint is polled every
POLL_EVERY requests, itself logged and counted toward the cap. No request is made when the
last reported figure, less the requests made since, would fall below FLOOR, or when the
log's own count has reached CAP.

`fetch <path> --out <file>` reads one further endpoint through the same logged, capped path.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import statistics
import subprocess
import sys
from pathlib import Path

CAP = 3000
FLOOR = 2500
POLL_EVERY = 10

WORKFLOWS = [
    "ci.yml",
    "release-plz.yml",
    "release.yml",
    "visual-docs.yml",
    "notignored.yml",
    "live-janitor.yml",
]
RELEASE_TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")


def iso(value: str | None) -> dt.datetime | None:
    if not value:
        return None
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))


def seconds(start: str | None, end: str | None) -> float | None:
    a, b = iso(start), iso(end)
    if a is None or b is None:
        return None
    return (b - a).total_seconds()


def stats(values: list[float]) -> dict | None:
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, -(-9 * len(ordered) // 10))  # nearest-rank p90
    return {
        "n": len(ordered),
        "median": round(statistics.median(ordered), 1),
        "p90": round(ordered[rank - 1], 1),
        "max": round(ordered[-1], 1),
    }


def runner_os(job: dict) -> str:
    text = " ".join(job.get("labels") or []) + " " + (job.get("runner_name") or "")
    text = text.lower()
    for needle, name in (("windows", "Windows"), ("macos", "macOS"), ("ubuntu", "Linux"), ("linux", "Linux")):
        if needle in text:
            return name
    return "unknown"


# --------------------------------------------------------------------------------------------
# The logged, capped, cached request path.


class Budget(Exception):
    pass


class Client:
    def __init__(self, raw: Path, log: Path, offline: bool):
        self.raw = raw
        self.log = log
        self.offline = offline
        self.raw.mkdir(parents=True, exist_ok=True)
        self.count = 0
        self.last_reported: int | None = None
        self.since_report = 0
        if log.exists():
            for line in log.read_text().splitlines():
                entry = json.loads(line)
                if entry.get("kind") in ("request", "rate_limit"):
                    self.count += 1
                elif entry.get("kind") == "pre-harness":
                    self.count += entry["requests"]

    def _write_log(self, entry: dict) -> None:
        entry["at"] = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
        with self.log.open("a") as handle:
            handle.write(json.dumps(entry) + "\n")

    def _poll(self) -> None:
        if self.count >= CAP:
            raise Budget(f"request cap {CAP} reached before polling /rate_limit")
        out = subprocess.run(["gh", "api", "rate_limit"], capture_output=True, text=True)
        self.count += 1
        if out.returncode != 0:
            self._write_log({"kind": "rate_limit", "seq": self.count, "error": out.stderr.strip()})
            raise Budget(f"/rate_limit failed: {out.stderr.strip()}")
        core = json.loads(out.stdout)["resources"]["core"]
        self.last_reported = core["remaining"]
        self.since_report = 0
        self._write_log({"kind": "rate_limit", "seq": self.count, "core_remaining": core["remaining"],
                         "core_limit": core["limit"], "core_reset": core["reset"]})

    def path_file(self, path: str) -> Path:
        return self.raw / (re.sub(r"[^A-Za-z0-9._=-]+", "_", path.strip("/")) + ".json")

    def get(self, path: str, text: bool = False):
        target = self.path_file(path)
        if text:
            target = target.with_suffix(".txt")
        if target.exists():
            return target.read_text() if text else json.loads(target.read_text())
        if self.offline:
            return None
        if self.last_reported is None or self.since_report >= POLL_EVERY:
            self._poll()
        if self.count >= CAP:
            raise Budget(f"request cap {CAP} reached; not requesting {path}")
        if self.last_reported - self.since_report - 1 < FLOOR:
            self._poll()
            if self.last_reported - 1 < FLOOR:
                raise Budget(f"core remaining {self.last_reported} is at the {FLOOR} floor; not requesting {path}")
        self.count += 1
        self._write_log({"kind": "request", "seq": self.count, "method": "GET", "path": path,
                         "core_remaining_last_reported": self.last_reported,
                         "core_requests_since_report": self.since_report})
        self.since_report += 1
        out = subprocess.run(["gh", "api", "-H", "Accept: application/vnd.github+json", path],
                             capture_output=True, text=True)
        if out.returncode != 0:
            self._write_log({"kind": "error", "seq": self.count, "path": path, "stderr": out.stderr.strip()[:500]})
            raise RuntimeError(f"GET {path} failed: {out.stderr.strip()}")
        if text:
            target.write_text(out.stdout)
            return out.stdout
        data = json.loads(out.stdout)
        target.write_text(json.dumps(data, indent=1, sort_keys=True))
        return data


# --------------------------------------------------------------------------------------------
# Collection.


def windows(since: dt.date, until: dt.date, days: int = 7):
    start = since
    while start <= until:
        end = min(start + dt.timedelta(days=days - 1), until)
        yield start, end
        start = end + dt.timedelta(days=1)


def collect_runs(client: Client, repo: str, since: dt.date, until: dt.date, missing: list) -> list[dict]:
    runs: dict[int, dict] = {}
    for workflow in WORKFLOWS:
        for a, b in windows(since, until):
            page = 1
            while True:
                path = (f"repos/{repo}/actions/workflows/{workflow}/runs?created={a.isoformat()}..{b.isoformat()}"
                        f"&per_page=100&page={page}")
                data = client.get(path)
                if data is None:
                    missing.append(path)
                    break
                for run in data["workflow_runs"]:
                    run["_workflow_file"] = workflow
                    runs[run["id"]] = run
                if page * 100 >= data["total_count"] or not data["workflow_runs"]:
                    break
                page += 1
    return sorted(runs.values(), key=lambda r: r["created_at"])


def collect_jobs(client: Client, repo: str, run: dict, missing: list) -> list[dict] | None:
    jobs: list[dict] = []
    page = 1
    while True:
        path = f"repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100&page={page}"
        data = client.get(path)
        if data is None:
            missing.append(path)
            return None
        jobs.extend(data["jobs"])
        if page * 100 >= data["total_count"] or not data["jobs"]:
            return jobs
        page += 1


def collect_pulls(client: Client, repo: str, since: dt.date, missing: list) -> list[dict]:
    pulls: list[dict] = []
    page = 1
    while True:
        path = f"repos/{repo}/pulls?state=closed&sort=updated&direction=desc&per_page=100&page={page}"
        data = client.get(path)
        if data is None:
            missing.append(path)
            break
        pulls.extend(data)
        if len(data) < 100 or iso(data[-1]["updated_at"]).date() < since:
            break
        page += 1
    return pulls


def collect_releases(client: Client, repo: str, since: dt.date, missing: list) -> list[dict]:
    releases: list[dict] = []
    page = 1
    while True:
        path = f"repos/{repo}/releases?per_page=100&page={page}"
        data = client.get(path)
        if data is None:
            missing.append(path)
            break
        releases.extend(data)
        if len(data) < 100 or iso(data[-1]["created_at"]).date() < since:
            break
        page += 1
    return releases


def git_record(client: Client, name: str, compute):
    """A local-git answer, kept under raw/git so the offline run reads the same answer."""
    target = client.raw / "git" / f"{name}.json"
    if target.exists():
        return json.loads(target.read_text())
    if client.offline:
        return None
    value = compute()
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(value, indent=1, sort_keys=True))
    return value


def version_key(tag: str):
    match = RELEASE_TAG.match(tag)
    return tuple(int(x) for x in match.groups()) if match else None


def first_release_containing(sha: str, git_dir: Path) -> str | None:
    out = subprocess.run(["git", "-C", str(git_dir), "tag", "--contains", sha], capture_output=True, text=True)
    if out.returncode != 0:
        return None
    tags = [t for t in out.stdout.split() if version_key(t)]
    return min(tags, key=version_key) if tags else None


def tag_commit(tag: str, git_dir: Path) -> str | None:
    out = subprocess.run(["git", "-C", str(git_dir), "rev-list", "-n", "1", tag], capture_output=True, text=True)
    return out.stdout.strip() or None


# --------------------------------------------------------------------------------------------
# Job logs: the phases of a `check` job, read off the markers the recipes print.

LOG_LINE = re.compile(r"^\ufeff?(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?Z (.*)")
PHASE_MARKERS = [
    (re.compile(r"^##\[group\]Run just bootstrap"), "bootstrap"),
    (re.compile(r"^##\[group\]Run zero="), "derive-base"),
    (re.compile(r"^##\[group\]Run just (check-generated|gate-generated)"), "generate-check"),
    (re.compile(r"^nx: run workspace:deny"), "deny"),
    (re.compile(r"^nx: affected -t format-check"), "format-check"),
    (re.compile(r"^nx: affected -t lint"), "lint"),
    (re.compile(r"^nx: affected -t typecheck"), "typecheck"),
    (re.compile(r"^nx: affected -t test"), "test"),
    (re.compile(r"^nx: affected -t coverage"), "coverage"),
    (re.compile(r"^nx: run scripts:distribution-check"), "distribution-check"),
    (re.compile(r"^nx: run scripts:distribution-test"), "distribution-test"),
]


def log_phases(text: str) -> dict[str, float]:
    """Seconds per phase: from one marker to the next, the last ending at the next step."""
    marks: list[tuple[dt.datetime, str]] = []
    end = None
    for line in text.splitlines():
        m = LOG_LINE.match(line)
        if not m:
            continue
        t = dt.datetime.fromisoformat(m.group(1) + "+00:00") + dt.timedelta(seconds=float(m.group(2) or 0))
        body = m.group(3)
        hit = next((label for rx, label in PHASE_MARKERS if rx.match(body)), None)
        if hit:
            # `just bootstrap` and the gate's own `just check` print the generate marker's
            # step; within the step, only the first marker of each label counts.
            if hit not in (label for _, label in marks):
                marks.append((t, hit))
            end = None
        elif marks and end is None and body.startswith("##[group]Run") and marks[-1][1] != "bootstrap":
            end = t
        elif marks and body.startswith("Post job cleanup") and end is None:
            end = t
    phases = {}
    for (a, label), nxt in zip(marks, marks[1:] + [(end, None)]):
        if nxt[0] is not None:
            phases[label] = round((nxt[0] - a).total_seconds(), 1)
    return phases


# --------------------------------------------------------------------------------------------
# Analysis.


def executed(job: dict) -> bool:
    """A job that ran on a runner: skipped jobs and jobs cancelled before a runner took them
    (no runner_name) are excluded, because neither occupied a runner."""
    return job.get("conclusion") != "skipped" and bool(job.get("runner_name"))


def analyse(args, client: Client) -> dict:
    since = dt.date.fromisoformat(args.since)
    until = dt.date.fromisoformat(args.until)
    missing: list[str] = []
    runs = collect_runs(client, args.repo, since, until, missing)

    janitor = [r for r in runs if r["_workflow_file"] == "live-janitor.yml"]
    sampled_janitor = {r["id"] for r in janitor[:: args.janitor_every]}
    jobs_by_run: dict[int, list[dict]] = {}
    for run in runs:
        if run["_workflow_file"] == "live-janitor.yml" and run["id"] not in sampled_janitor:
            continue
        if run.get("status") != "completed":
            continue
        jobs = collect_jobs(client, args.repo, run, missing)
        if jobs is not None:
            jobs_by_run[run["id"]] = jobs
    run_by_id = {r["id"]: r for r in runs}

    # ---- per job, per matrix leg -------------------------------------------------------
    groups: dict[tuple, dict] = {}
    for run_id, jobs in jobs_by_run.items():
        run = run_by_id[run_id]
        for job in jobs:
            if not executed(job):
                continue
            key = (run["name"], run["event"], job["name"])
            entry = groups.setdefault(key, {"workflow": run["name"], "event": run["event"], "job": job["name"],
                                            "os": runner_os(job), "conclusions": {}, "_dur": [], "_wait": [],
                                            "_pure_queue": []})
            entry["conclusions"][job["conclusion"] or job["status"]] = entry["conclusions"].get(
                job["conclusion"] or job["status"], 0) + 1
            if job["conclusion"] == "success":
                entry["_dur"].append(seconds(job["started_at"], job["completed_at"]))
                entry["_wait"].append(seconds(run["created_at"], job["started_at"]))
                entry["_pure_queue"].append(seconds(job["created_at"], job["started_at"]))
    per_job = []
    for entry in sorted(groups.values(), key=lambda e: (e["workflow"], e["event"], e["job"])):
        per_job.append({
            "workflow": entry["workflow"], "event": entry["event"], "job": entry["job"], "os": entry["os"],
            "conclusions": entry["conclusions"],
            "failed_or_cancelled": sum(v for k, v in entry["conclusions"].items() if k in ("failure", "cancelled", "timed_out")),
            "duration_s_success": stats(entry.pop("_dur")),
            "wait_from_run_created_s_success": stats(entry.pop("_wait")),
            "queue_from_job_created_s_success": stats(entry.pop("_pure_queue")),
        })

    # ---- per step, for the jobs whose time the report attributes ----------------------------
    step_groups: dict[tuple, list[float]] = {}
    for run_id, jobs in jobs_by_run.items():
        run = run_by_id[run_id]
        if run["_workflow_file"] not in ("ci.yml", "release.yml", "release-plz.yml"):
            continue
        kind = "release-pr" if (run["head_branch"] or "").startswith("release-plz-") else "other"
        for job in jobs:
            if job.get("conclusion") != "success":
                continue
            for step in job.get("steps") or []:
                d = seconds(step.get("started_at"), step.get("completed_at"))
                if d is None or step.get("conclusion") != "success":
                    continue
                key = (run["name"], run["event"], kind, job["name"], step["name"])
                step_groups.setdefault(key, []).append(d)
    per_step = [{"workflow": k[0], "event": k[1], "branch_kind": k[2], "job": k[3], "step": k[4],
                 "duration_s": stats(v)} for k, v in sorted(step_groups.items())
                if max(v) >= 30]

    # ---- phases of the `check` jobs whose logs were read ----------------------------------------
    phase_values: dict[tuple, list[float]] = {}
    logs_read = []
    for run_id, jobs in jobs_by_run.items():
        for job in jobs:
            text = client.get(f"repos/{args.repo}/actions/jobs/{job['id']}/logs", text=True) \
                if client.path_file(f"repos/{args.repo}/actions/jobs/{job['id']}/logs").with_suffix(".txt").exists() else None
            if text is None:
                continue
            phases = log_phases(text)
            logs_read.append({"job_id": job["id"], "run_id": run_id, "job": job["name"],
                              "event": run_by_id[run_id]["event"], "phases_s": phases})
            for label, value in phases.items():
                phase_values.setdefault((job["name"], label), []).append(value)
    check_phases = {"logs": logs_read,
                    "summary": [{"job": k[0], "phase": k[1], "seconds": stats(v)}
                                for k, v in sorted(phase_values.items())]}

    # ---- per job, release PR against every other PR -----------------------------------------
    split: dict[tuple, list[float]] = {}
    for run_id, jobs in jobs_by_run.items():
        run = run_by_id[run_id]
        if run["_workflow_file"] != "ci.yml":
            continue
        kind = "release-pr" if (run["head_branch"] or "").startswith("release-plz-") else "other"
        for job in jobs:
            if job.get("conclusion") == "success":
                split.setdefault((run["event"], kind, job["name"]), []).append(
                    seconds(job["started_at"], job["completed_at"]))
    per_job_by_branch_kind = [{"event": k[0], "branch_kind": k[1], "job": k[2], "duration_s": stats(v)}
                              for k, v in sorted(split.items())]

    # ---- per run --------------------------------------------------------------------------
    def job_counts(run_ids) -> dict:
        by_wf: dict[str, int] = {}
        by_os: dict[str, int] = {}
        total = 0
        for run_id in run_ids:
            for job in jobs_by_run.get(run_id, []):
                if executed(job):
                    total += 1
                    wf = run_by_id[run_id]["name"]
                    by_wf[wf] = by_wf.get(wf, 0) + 1
                    by_os[runner_os(job)] = by_os.get(runner_os(job), 0) + 1
        return {"total": total, "by_workflow": by_wf, "by_os": by_os}

    def span(run_ids) -> tuple[str | None, str | None, float | None, str | None]:
        first = min((run_by_id[r]["created_at"] for r in run_ids), default=None)
        ends = [(j["completed_at"], j["name"]) for r in run_ids for j in jobs_by_run.get(r, []) if executed(j) and j["completed_at"]]
        last = max(ends, default=(None, None))
        return first, last[0], seconds(first, last[0]), last[1]

    def longest(run_ids) -> dict | None:
        best = None
        for r in run_ids:
            for j in jobs_by_run.get(r, []):
                if executed(j) and j.get("completed_at"):
                    d = seconds(j["started_at"], j["completed_at"])
                    if best is None or d > best["seconds"]:
                        best = {"seconds": d, "job": j["name"], "workflow": run_by_id[r]["name"], "run_id": r}
        return best

    # A pull_request run created after its PR closed (visual-docs' closed-event cleanup) is not
    # one of that head commit's checks.
    pulls = collect_pulls(client, args.repo, since, missing)
    closed_at = {p["head"]["sha"]: p["closed_at"] for p in pulls if p.get("closed_at")}
    pr_runs: dict[str, list[int]] = {}
    for run in runs:
        if run["event"] == "pull_request" and run["id"] in jobs_by_run:
            if run["head_sha"] in closed_at and run["created_at"] > closed_at[run["head_sha"]]:
                continue
            pr_runs.setdefault(run["head_sha"], []).append(run["id"])

    ci_pr_runs = []
    for run in runs:
        if run["_workflow_file"] == "ci.yml" and run["event"] == "pull_request" and run["id"] in jobs_by_run:
            counts = job_counts([run["id"]])
            lj = longest([run["id"]])
            ci_pr_runs.append({"run_id": run["id"], "head_sha": run["head_sha"], "conclusion": run["conclusion"],
                               "created_at": run["created_at"], "jobs": counts["total"], "by_os": counts["by_os"],
                               "longest_job_s": lj and lj["seconds"], "longest_job": lj and lj["job"]})
    complete = [r for r in ci_pr_runs if r["conclusion"] == "success"]
    per_run = {
        "ci_pull_request_runs": {
            "runs": len(ci_pr_runs), "successful": len(complete),
            "jobs_per_run_success": stats([r["jobs"] for r in complete]),
            "jobs_per_run_by_os_success": {os_: stats([r["by_os"].get(os_, 0) for r in complete])
                                            for os_ in ("Linux", "Windows", "macOS")},
            "longest_job_s_success": stats([r["longest_job_s"] for r in complete if r["longest_job_s"]]),
            "detail": ci_pr_runs,
        },
        "pr_head_commit_all_workflows": {
            "head_commits": len(pr_runs),
            "jobs_per_head_commit": stats([job_counts(ids)["total"] for ids in pr_runs.values()]),
            "by_workflow": {wf: stats([job_counts(ids)["by_workflow"].get(wf, 0) for ids in pr_runs.values()])
                            for wf in sorted({run_by_id[i]["name"] for ids in pr_runs.values() for i in ids})},
            "by_os": {os_: stats([job_counts(ids)["by_os"].get(os_, 0) for ids in pr_runs.values()])
                      for os_ in ("Linux", "Windows", "macOS")},
        },
    }
    rp: dict[str, dict] = {}
    for run in runs:
        if run["_workflow_file"] == "ci.yml" and run["event"] == "pull_request" \
                and (run["head_branch"] or "").startswith("release-plz-"):
            entry = rp.setdefault(run["head_branch"], {"runs": 0, "head_commits": set(), "conclusions": {}})
            entry["runs"] += 1
            entry["head_commits"].add(run["head_sha"])
            key = run["conclusion"] or run["status"]
            entry["conclusions"][key] = entry["conclusions"].get(key, 0) + 1
    per_run["release_pr_ci_runs"] = {
        "release_prs": len(rp),
        "ci_runs_per_release_pr": stats([e["runs"] for e in rp.values()]),
        "head_commits_per_release_pr": stats([len(e["head_commits"]) for e in rp.values()]),
        "conclusions": {k: sum(e["conclusions"].get(k, 0) for e in rp.values())
                        for k in sorted({k for e in rp.values() for k in e["conclusions"]})},
    }
    push_main = [r for r in runs if r["event"] == "push" and r["head_branch"] == "main" and r["id"] in jobs_by_run]
    per_push: dict[str, list[int]] = {}
    for run in push_main:
        per_push.setdefault(run["head_sha"], []).append(run["id"])
    per_run["push_main_commit_all_workflows"] = {
        "commits": len(per_push),
        "jobs_per_commit": stats([job_counts(ids)["total"] for ids in per_push.values()]),
        "by_workflow": {wf: stats([job_counts(ids)["by_workflow"].get(wf, 0) for ids in per_push.values()])
                        for wf in sorted({run_by_id[i]["name"] for ids in per_push.values() for i in ids})},
    }

    # ---- releases and their CD ----------------------------------------------------------
    releases = [r for r in collect_releases(client, args.repo, since, missing)
                if version_key(r["tag_name"]) and iso(r["created_at"]).date() >= since]
    git_dir = Path(args.git_dir)
    cd: dict[str, dict] = {}
    for rel in sorted(releases, key=lambda r: version_key(r["tag_name"])):
        tag = rel["tag_name"]
        commit = git_record(client, f"tag-commit-{tag}", lambda: tag_commit(tag, git_dir))
        if commit is None:
            missing.append(f"git:tag-commit-{tag}")
            continue
        ids = [r["id"] for r in runs if r["id"] in jobs_by_run and (
            (r["event"] == "push" and r["head_sha"] == commit)
            or (r["event"] == "release" and r["head_branch"] == tag)
            or (r["event"] == "workflow_run" and r["head_sha"] == commit))]
        release_only = [i for i in ids if run_by_id[i]["_workflow_file"] in ("release-plz.yml", "release.yml")]
        first, last, total, last_job = span(ids)
        incomplete = [i for i in ids if run_by_id[i]["status"] != "completed"]
        has_release_run = any(run_by_id[i]["event"] == "release" for i in ids)
        cd[tag] = {
            "tag": tag, "commit": commit, "published_at": rel.get("published_at"),
            "runs": [{"id": i, "workflow": run_by_id[i]["name"], "event": run_by_id[i]["event"],
                      "conclusion": run_by_id[i]["conclusion"]} for i in ids],
            "complete": bool(ids) and has_release_run and not incomplete,
            "last_job_completed_at": last, "last_job": last_job,
            "jobs": job_counts(ids), "jobs_release_workflows_only": job_counts(release_only),
            "longest_job": longest(ids), "longest_job_release_workflows_only": longest(release_only),
            "last_job_completed_at_release_workflows_only": span(release_only)[1],
            "release_workflows_last_job": span(release_only)[3],
        }
    complete_cd = [c for c in cd.values() if c["complete"]]
    cd_summary = {
        "releases_in_window": len(cd), "complete": len(complete_cd),
        "jobs_per_release": stats([c["jobs"]["total"] for c in complete_cd]),
        "jobs_per_release_by_workflow": {wf: stats([c["jobs"]["by_workflow"].get(wf, 0) for c in complete_cd])
                                         for wf in sorted({w for c in complete_cd for w in c["jobs"]["by_workflow"]})},
        "jobs_per_release_by_os": {os_: stats([c["jobs"]["by_os"].get(os_, 0) for c in complete_cd])
                                   for os_ in ("Linux", "Windows", "macOS")},
        "jobs_per_release_release_workflows_only": stats([c["jobs_release_workflows_only"]["total"] for c in complete_cd]),
        "longest_job_s": stats([c["longest_job"]["seconds"] for c in complete_cd if c["longest_job"]]),
        "longest_job_s_release_workflows_only": stats([c["longest_job_release_workflows_only"]["seconds"]
                                                      for c in complete_cd if c["longest_job_release_workflows_only"]]),
    }

    # ---- cycle time -----------------------------------------------------------------------
    merged = [p for p in pulls if p.get("merged_at") and p["base"]["ref"] == "main"
              and since <= iso(p["merged_at"]).date() <= until]
    cycles = []
    for pr in sorted(merged, key=lambda p: p["merged_at"]):
        head = pr["head"]["sha"]
        ids = [i for i in pr_runs.get(head, []) if run_by_id[i]["created_at"] <= pr["merged_at"]]
        record = {"number": pr["number"], "title": pr["title"], "head_sha": head,
                  "merge_commit_sha": pr["merge_commit_sha"], "merged_at": pr["merged_at"]}
        if not ids:
            record["ineligible"] = "no head-commit pull_request run in the window"
            cycles.append(record)
            continue
        first, last, checks_s, last_job = span(ids)
        record.update({"checks_first_run_created_at": first, "checks_last_job_completed_at": last,
                       "checks_last_job": last_job, "checks_s": checks_s,
                       "checks_runs": [{"id": i, "workflow": run_by_id[i]["name"]} for i in ids]})
        tag = git_record(client, f"first-release-{pr['merge_commit_sha']}",
                         lambda: first_release_containing(pr["merge_commit_sha"], git_dir))
        record["release"] = tag
        rel = cd.get(tag) if tag else None
        if rel is None or not rel["complete"]:
            record["ineligible"] = "no completed release in the window contains the merge commit"
        else:
            record["merge_to_release_s"] = seconds(pr["merged_at"], rel["last_job_completed_at"])
            record["release_last_job_completed_at"] = rel["last_job_completed_at"]
            record["cycle_s"] = checks_s + record["merge_to_release_s"]
            # The merge-to-release phase split at the release PR's own merge (the tagged
            # commit), and the same phase ended at the release workflows' last job alone.
            release_pr = next((p for p in merged if p["merge_commit_sha"] == rel["commit"]), None)
            if release_pr is not None:
                record["wait_for_release_pr_merge_s"] = seconds(pr["merged_at"], release_pr["merged_at"])
                record["release_pr_merge_to_cd_end_s"] = seconds(release_pr["merged_at"], rel["last_job_completed_at"])
                record["release_pr_merge_to_release_workflows_end_s"] = seconds(
                    release_pr["merged_at"], rel["last_job_completed_at_release_workflows_only"])
            record["merge_to_release_workflows_end_s"] = seconds(pr["merged_at"], rel["last_job_completed_at_release_workflows_only"])
            record["cd_last_job"] = rel["last_job"]
        cycles.append(record)
    eligible = [c for c in cycles if "cycle_s" in c]
    per_release_max: dict[str, float] = {}
    for c in eligible:
        per_release_max[c["release"]] = max(per_release_max.get(c["release"], 0), c["cycle_s"])
    cycle_summary = {
        "merged_prs_in_window": len(cycles), "eligible": len(eligible),
        "checks_s": stats([c["checks_s"] for c in eligible]),
        "merge_to_release_s": stats([c["merge_to_release_s"] for c in eligible]),
        "cycle_s": stats([c["cycle_s"] for c in eligible]),
        "release_cycle_time_max_per_release_s": stats(list(per_release_max.values())),
        "wait_for_release_pr_merge_s": stats([c["wait_for_release_pr_merge_s"] for c in eligible
                                              if "wait_for_release_pr_merge_s" in c]),
        "release_pr_merge_to_cd_end_s": stats([c["release_pr_merge_to_cd_end_s"] for c in eligible
                                               if "release_pr_merge_to_cd_end_s" in c]),
        "release_pr_merge_to_release_workflows_end_s": stats([c["release_pr_merge_to_release_workflows_end_s"]
                                                             for c in eligible
                                                             if c.get("release_pr_merge_to_release_workflows_end_s") is not None]),
        "merge_to_release_workflows_end_s": stats([c["merge_to_release_workflows_end_s"] for c in eligible
                                                   if c.get("merge_to_release_workflows_end_s") is not None]),
        "cd_last_job": dict(sorted(((k, sum(1 for c in cd.values() if c["complete"] and c["last_job"] == k))
                                    for k in {c["last_job"] for c in cd.values() if c["complete"]}),
                                   key=lambda kv: -kv[1])),
        "prs_per_release": stats([sum(1 for c in eligible if c["release"] == t) for t in per_release_max]),
    }

    return {
        "repo": args.repo, "since": args.since, "until": args.until,
        "runs_listed": {wf: sum(1 for r in runs if r["_workflow_file"] == wf) for wf in WORKFLOWS},
        "runs_with_jobs_read": len(jobs_by_run),
        "sampling": {"live-janitor.yml": f"jobs read for every {args.janitor_every}th completed run "
                                         f"({len(sampled_janitor)} of {len(janitor)})"},
        "per_job": per_job, "per_job_by_branch_kind": per_job_by_branch_kind, "per_step": per_step,
        "check_phases_from_logs": check_phases,
        "per_run": per_run,
        "cd": {"summary": cd_summary, "releases": list(cd.values())},
        "cycle": {"summary": cycle_summary, "prs": cycles},
        "missing": sorted(set(missing)),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    fig = sub.add_parser("figures", help="read the records and write the figures document")
    fig.add_argument("--repo", required=True)
    fig.add_argument("--since", required=True, help="first day of the window, YYYY-MM-DD")
    fig.add_argument("--until", default=dt.date.today().isoformat(), help="last day of the window, inclusive")
    fig.add_argument("--raw-dir", required=True, type=Path)
    fig.add_argument("--log", required=True, type=Path)
    fig.add_argument("--out", required=True, type=Path)
    fig.add_argument("--git-dir", default=".", help="a clone holding the release tags")
    fig.add_argument("--janitor-every", type=int, default=6)
    fig.add_argument("--offline", action="store_true")
    one = sub.add_parser("fetch", help="read one further endpoint through the logged, capped path")
    one.add_argument("path")
    one.add_argument("--raw-dir", required=True, type=Path)
    one.add_argument("--log", required=True, type=Path)
    one.add_argument("--text", action="store_true", help="keep the response as text (a job log)")
    args = parser.parse_args()

    client = Client(args.raw_dir, args.log, getattr(args, "offline", False))
    try:
        if args.cmd == "fetch":
            data = client.get(args.path, text=args.text)
            print(client.path_file(args.path).with_suffix(".txt" if args.text else ".json"))
            return 0 if data is not None else 1
        figures = analyse(args, client)
    except Budget as stop:
        print(f"stopped: {stop}. Next: wait for the core quota to reset, then re-run; cached responses are not re-read.",
              file=sys.stderr)
        return 3
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(figures, indent=1) + "\n")
    print(f"wrote {args.out}: {figures['runs_with_jobs_read']} runs with jobs, "
          f"{figures['cycle']['summary']['eligible']} eligible PRs, {len(figures['missing'])} missing inputs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
