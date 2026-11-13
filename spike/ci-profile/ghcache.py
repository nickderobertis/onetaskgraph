"""Read-only, cached GitHub reads for the CI profile spike.

Every read goes through the host's `gh` login, is written once under the cache directory and
never asked again, and refuses to start while `gh api rate_limit` reports fewer than
FLOOR core requests remaining (the dispatch's stop line).
"""

from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import zipfile
from pathlib import Path

REPO = "nickderobertis/onetaskgraph"
FLOOR = 2500


class RateFloor(RuntimeError):
    pass


def cache_dir() -> Path:
    root = os.environ.get("CIPROFILE_CACHE")
    path = Path(root) if root else Path(__file__).resolve().parents[2] / "target" / "ci-profile"
    path.mkdir(parents=True, exist_ok=True)
    return path


_requests = 0


def requests_made() -> int:
    return _requests


def remaining() -> int:
    out = subprocess.run(
        ["gh", "api", "rate_limit", "--jq", ".resources.core.remaining"],
        check=True, capture_output=True, text=True,
    ).stdout
    return int(out.strip())


def _check_floor() -> None:
    # Ask once every 25 reads; rate_limit itself is free.
    if _requests % 25 == 0:
        left = remaining()
        if left < FLOOR:
            raise RateFloor(f"only {left} core requests remain (floor {FLOOR}); stop and wait for the reset")


def _get(path: str) -> bytes:
    global _requests
    _check_floor()
    _requests += 1
    proc = subprocess.run(
        ["gh", "api", "--allow-escape-sequences", f"repos/{REPO}/{path}"],
        capture_output=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"gh api {path}: {proc.stderr.decode(errors='replace').strip()}")
    return proc.stdout


def _cached(name: str, path: str) -> bytes:
    target = cache_dir() / name
    if target.exists():
        return target.read_bytes()
    data = _get(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    return data


def run(run_id: int) -> dict:
    return json.loads(_cached(f"runs/{run_id}/run.json", f"actions/runs/{run_id}"))


def jobs(run_id: int) -> list[dict]:
    return json.loads(_cached(f"runs/{run_id}/jobs.json", f"actions/runs/{run_id}/jobs?per_page=50"))["jobs"]


def job_log(run_id: int, job_id: int) -> str:
    return _cached(f"runs/{run_id}/logs/{job_id}.txt", f"actions/jobs/{job_id}/logs").decode("utf-8", "replace")


def artifacts(run_id: int) -> list[dict]:
    return json.loads(_cached(f"runs/{run_id}/artifacts.json", f"actions/runs/{run_id}/artifacts?per_page=100"))["artifacts"]


def artifact_member(run_id: int, name: str, member: str) -> bytes | None:
    """One file out of a named artifact, cached unzipped; None when the run has no such artifact."""
    target = cache_dir() / f"runs/{run_id}/artifacts/{name}/{member}"
    if target.exists():
        return target.read_bytes()
    marker = target.with_name(target.name + ".absent")
    if marker.exists():
        return None
    found = [a for a in artifacts(run_id) if a["name"] == name and not a["expired"]]
    if not found:
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text("")
        return None
    data = _get(f"actions/artifacts/{found[0]['id']}/zip")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        names = archive.namelist()
        target.parent.mkdir(parents=True, exist_ok=True)
        for entry in names:
            if entry.endswith((".json", ".jsonl", ".xml", ".txt", ".csv")):
                out = target.parent / entry
                out.parent.mkdir(parents=True, exist_ok=True)
                out.write_bytes(archive.read(entry))
    if not target.exists():
        marker.write_text("")
        return None
    return target.read_bytes()


def runs_list(query: str, pages: int = 3) -> list[dict]:
    out: list[dict] = []
    for page in range(1, pages + 1):
        body = json.loads(_cached(f"lists/{query.replace('/', '_').replace('&', '_').replace('=', '_')}.{page}.json",
                                  f"actions/workflows/ci.yml/runs?{query}&per_page=100&page={page}"))
        out += body["workflow_runs"]
        if len(body["workflow_runs"]) < 100:
            break
    return out


if __name__ == "__main__":
    print(remaining(), file=sys.stderr)
