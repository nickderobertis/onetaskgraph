"""An async, typed, read-only and cached GitHub client for the CI profile spike.

Every read goes through the host's `gh` login (`gh api`), is written once under the cache
directory and never asked again, and is refused — `RateFloor` — while `gh api rate_limit`
reports fewer than REST_FLOOR core or GRAPHQL_FLOOR GraphQL requests remaining; the check
spends neither allowance and runs before every batch of CHECK_EVERY reads. With
CIPROFILE_OFFLINE set a read that is not cached raises `Offline` instead of being made.
Listings are paged to their end, so a run's jobs and artifacts are all of them.
"""

from __future__ import annotations

import asyncio
import io
import json
import os
import shutil
import zipfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from models import Artifact, Invalid, Job, RateLimit, Run, RunId, items, obj

REPO = "nickderobertis/onetaskgraph"
# This host's login shares both allowances with other agents and with the live test lanes, which
# pause when the account is under 40% of either; reads stop above that line so they never cause a pause.
REST_FLOOR = 2500
GRAPHQL_FLOOR = 2000
CHECK_EVERY = 25
PAGE = 100
COMPLETE = ".complete"
KEPT_SUFFIXES = (".json", ".jsonl", ".xml", ".txt", ".csv", ".spawns")


class RateFloor(RuntimeError):
    """An allowance this host's login shares is below its floor; wait for its reset."""


class Offline(LookupError):
    """CIPROFILE_OFFLINE is set and the read is not in the cache."""


class ReadFailed(RuntimeError):
    """`gh api` refused or failed a read."""


def default_cache() -> Path:
    root = os.environ.get("CIPROFILE_CACHE")
    return Path(root) if root else Path(__file__).resolve().parents[2] / "target" / "ci-profile"


@dataclass
class GitHub:
    cache: Path = field(default_factory=default_cache)
    offline: bool = field(default_factory=lambda: bool(os.environ.get("CIPROFILE_OFFLINE")))
    reads: int = 0

    async def _gh(self, *args: str) -> bytes:
        proc = await asyncio.create_subprocess_exec(
            "gh", *args, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
        )
        out, err = await proc.communicate()
        if proc.returncode != 0:
            raise ReadFailed(f"gh {' '.join(args)}: {err.decode(errors='replace').strip()}")
        return out

    async def rate_limit(self) -> RateLimit:
        return RateLimit.parse(json.loads(await self._gh("api", "rate_limit")))

    async def _read(self, path: str) -> bytes:
        if self.offline:
            raise Offline(path)
        if self.reads % CHECK_EVERY == 0:
            limit = await self.rate_limit()
            if limit.rest < REST_FLOOR:
                raise RateFloor(f"only {limit.rest} REST core requests remain (floor {REST_FLOOR}); wait for the reset")
            if limit.graphql < GRAPHQL_FLOOR:
                raise RateFloor(
                    f"only {limit.graphql} GraphQL points remain (floor {GRAPHQL_FLOOR}); wait for the reset"
                )
        self.reads += 1
        return await self._gh("api", "--allow-escape-sequences", f"repos/{REPO}/{path}")

    async def _cached(self, name: str, path: str) -> bytes:
        target = self.cache / name
        if target.exists():
            return target.read_bytes()
        data = await self._read(path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        return data

    async def _pages(self, name: str, path: str, key: str) -> list[Any]:
        found: list[Any] = []
        page = 1
        while True:
            body = obj(
                json.loads(await self._cached(f"{name}.{page}.json", f"{path}per_page={PAGE}&page={page}")), path
            )
            batch = items(body.get(key), f"{path} {key}")
            found += batch
            total = body.get("total_count")
            if total is not None and (isinstance(total, bool) or not isinstance(total, int) or total < 0):
                raise Invalid(f"{path}: total_count {total!r} is not a count")
            if len(batch) < PAGE or (total is not None and len(found) >= total):
                return found
            page += 1

    async def run(self, run_id: RunId) -> Run:
        target = self.cache / f"runs/{run_id}/run.json"
        if not target.exists():
            for listing in sorted((self.cache / "lists").glob("*.json")):
                for entry in items(
                    obj(json.loads(listing.read_text()), str(listing)).get("workflow_runs"), str(listing)
                ):
                    if isinstance(entry, dict) and entry.get("id") == run_id:
                        target.parent.mkdir(parents=True, exist_ok=True)
                        target.write_text(json.dumps(entry))
        return Run.parse(json.loads(await self._cached(f"runs/{run_id}/run.json", f"actions/runs/{run_id}")))

    async def _listing(self, run_id: RunId, kind: str) -> list[Any]:
        """A run's jobs or artifacts: a whole one-page listing an earlier release of this cache kept, or every page."""
        legacy = self.cache / f"runs/{run_id}/{kind}.json"
        if legacy.exists():
            body = obj(json.loads(legacy.read_text()), str(legacy))
            found = items(body.get(kind), f"{legacy} {kind}")
            total = body.get("total_count", len(found))
            if isinstance(total, int) and not isinstance(total, bool) and 0 <= total <= len(found):
                return found
        return await self._pages(f"runs/{run_id}/{kind}", f"actions/runs/{run_id}/{kind}?", kind)

    async def jobs(self, run_id: RunId) -> list[Job]:
        return [Job.parse(j) for j in await self._listing(run_id, "jobs")]

    async def job_log(self, run_id: RunId, job: Job) -> str:
        data = await self._cached(f"runs/{run_id}/logs/{job.id}.txt", f"actions/jobs/{job.id}/logs")
        return data.decode("utf-8", "replace")

    async def artifacts(self, run_id: RunId) -> list[Artifact]:
        return [Artifact.parse(a) for a in await self._listing(run_id, "artifacts")]

    async def artifact_member(self, run_id: RunId, name: str, member: str) -> bytes | None:
        """One file of a named artifact, cached unzipped; None when the run holds no such artifact or file.

        An artifact is downloaded and extracted whole, then marked complete; a directory without the
        mark is an interrupted extraction and is downloaded again."""
        directory = self.cache / f"runs/{run_id}/artifacts/{name}"
        complete = directory / COMPLETE
        if not complete.exists():
            found = [a for a in await self.artifacts(run_id) if a.name == name and not a.expired]
            directory.mkdir(parents=True, exist_ok=True)
            if found:
                extract(await self._read(f"actions/artifacts/{found[0].id}/zip"), directory)
            complete.write_text("present\n" if found else "absent\n")
        target = directory / member
        return target.read_bytes() if target.is_file() else None

    async def runs_list(self, query: str, pages: int) -> list[Run]:
        name = "lists/" + query.replace("/", "_").replace("&", "_").replace("=", "_")
        out: list[Run] = []
        for page in range(1, pages + 1):
            body = obj(
                json.loads(
                    await self._cached(
                        f"{name}.{page}.json", f"actions/workflows/ci.yml/runs?{query}&per_page={PAGE}&page={page}"
                    )
                ),
                query,
            )
            batch = items(body.get("workflow_runs"), "workflow_runs")
            out += [Run.parse(r) for r in batch]
            if len(batch) < PAGE:
                break
        return out


def extract(data: bytes, directory: Path) -> None:
    """Replace `directory` with the artifact's kept files, so nothing of an interrupted extraction remains."""
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir(parents=True)
    root = directory.resolve()
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for entry in archive.namelist():
            out = (directory / entry).resolve()
            if not out.is_relative_to(root):
                raise Invalid(f"artifact entry {entry!r} lies outside its directory")
            if entry.endswith(KEPT_SUFFIXES):
                out.parent.mkdir(parents=True, exist_ok=True)
                out.write_bytes(archive.read(entry))
