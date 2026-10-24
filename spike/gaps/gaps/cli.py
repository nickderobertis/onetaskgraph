"""One measurement of one scenario: `python -m gaps <scenario> [--build stock|head|closure]`.

Live, on the test board only, quota-gated, needing `GH_PROJECTS_TOKEN`:
`probe-alias-pricing`, `probe-placement`. Offline, needing no credential: `reach-public`,
`reach-recopy`, `reach-recopy-unmarked`, `reach-private-item`, `reach-private-ref`,
`reach-unresolvable-ref`, `screen-union`, `proxy-refusal`.

Raw run directories go under `$XDG_CACHE_HOME/board-copy-gaps-2026-10-24/runs/` (default
`~/.cache`); the sanitized form is written to `spike/gaps/logs/<scenario>-<build>-<time>/`
(`SPIKE_GAPS_LOGS` names another directory) and its path printed on standard output, the only
line a successful run prints. `SPIKE_UPSTREAM` points the proxy at a loopback stand-in for
GitHub instead of GitHub itself; the tests use it, and nothing else may.

Exit statuses: 0 — the measurement ran and its result is recorded (a reachability case's own
copy may have failed: that is its result, in `result.json`); 1 — the measurement could not be
taken or broke a guarantee the harness owes (GitHub or onevcs failed, probe issues were left
behind, the union disagreed with the per-item verdicts, the proxy let a write through, or the
sanitized output would carry identifying text); 2 — the invocation or the environment is
wrong (an unknown scenario, a missing credential or binary, a cache path that is not absolute).
Every diagnostic goes to standard error.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import shutil
import sys
import time
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from urllib.parse import urlsplit

from gaps import allowance, board, placement, pricing, reach, refusal, union
from gaps.github import Client, GitHubError, RateLimit, RequestRecord, Token
from gaps.harness import sanitize
from gaps.json_shape import Json, ShapeError, load_lines
from gaps.proc import ProcessFailed
from gaps.relay import relay

PACKAGE_ROOT = Path(__file__).resolve().parents[1]
REPO = PACKAGE_ROOT.parents[1]
LOGS = PACKAGE_ROOT / "logs"
CACHE_NAME = "board-copy-gaps-2026-10-24"
USAGE = 2
FAILED = 1


class Scenario(StrEnum):
    """Every scenario this command measures."""

    PRICING = "probe-alias-pricing"
    PLACEMENT = "probe-placement"
    REACH_PUBLIC = reach.Case.PUBLIC.value
    REACH_RECOPY = reach.Case.RECOPY.value
    REACH_RECOPY_UNMARKED = reach.Case.RECOPY_UNMARKED.value
    REACH_PRIVATE_ITEM = reach.Case.PRIVATE_ITEM.value
    REACH_PRIVATE_REF = reach.Case.PRIVATE_REF.value
    REACH_UNRESOLVABLE_REF = reach.Case.UNRESOLVABLE_REF.value
    SCREEN_UNION = "screen-union"
    PROXY_REFUSAL = "proxy-refusal"


class Build(StrEnum):
    """Which onetaskgraph the offline cases run."""

    STOCK = "stock"
    HEAD = "head"
    CLOSURE = "closure"


class UsageError(Exception):
    """The invocation or its environment is wrong."""


class MeasurementFailed(Exception):
    """The measurement could not be taken, or broke a guarantee the harness owes."""


@dataclass(frozen=True)
class Paths:
    """Where this run reads and writes, from a validated environment."""

    cache: Path
    logs: Path
    onevcs: Path
    binaries: dict[Build, Path]
    upstream: str | None

    @property
    def runs(self) -> Path:
        """Raw run directories."""
        return self.cache / "runs"

    @property
    def quota_log(self) -> Path:
        """Every quota reading and pause."""
        return self.cache / "allowance.jsonl"


def absolute(name: str, default: Path) -> Path:
    """An environment path, refused when relative."""
    value = os.environ.get(name)
    path = Path(value) if value else default
    if not path.is_absolute():
        raise UsageError(f"{name} must be an absolute path, not {value!r}")
    return path


def paths() -> Paths:
    """This run's paths."""
    cache = absolute("XDG_CACHE_HOME", Path.home() / ".cache") / CACHE_NAME
    host = Path.home() / "ai-orchestrator/.venv/bin"
    upstream = os.environ.get("SPIKE_UPSTREAM")
    if upstream is not None:
        parts = urlsplit(upstream)
        if parts.scheme != "http" or parts.hostname not in ("127.0.0.1", "localhost"):
            raise UsageError(f"SPIKE_UPSTREAM may name only a loopback stand-in, not {upstream}")
        try:
            port = parts.port
        except ValueError as error:
            raise UsageError(f"SPIKE_UPSTREAM names no usable port: {upstream}") from error
        if port is None:
            raise UsageError(f"SPIKE_UPSTREAM names no port: {upstream}")
    return Paths(
        cache=cache,
        logs=absolute("SPIKE_GAPS_LOGS", LOGS),
        upstream=upstream,
        onevcs=absolute("SPIKE_ONEVCS", host / "onevcs"),
        binaries={
            Build.STOCK: absolute("SPIKE_STOCK_BINARY", host / "onetaskgraph"),
            Build.HEAD: absolute("SPIKE_HEAD_BINARY", REPO / "target/debug/onetaskgraph"),
            Build.CLOSURE: absolute(
                "SPIKE_CLOSURE_BINARY", cache / "prototype/target/debug/onetaskgraph"
            ),
        },
    )


def executable(path: Path, hint: str) -> Path:
    """`path`, refused when it is not an executable file."""
    if not (path.is_file() and os.access(path, os.X_OK)):
        raise UsageError(f"no executable at {path}; {hint}")
    return path


def token() -> Token:
    """The test board's credential, refused when absent or malformed."""
    value = os.environ.get("GH_PROJECTS_TOKEN", "")
    if not value or any(c.isspace() or not c.isprintable() for c in value):
        raise UsageError(
            "GH_PROJECTS_TOKEN is unset or malformed; export the test board's credential"
        )
    return Token(value)


def last_headers(runs: Path) -> RateLimit | None:
    """The GraphQL headers of the last answer the latest earlier live run recorded."""
    logs = sorted(runs.glob("probe-*/requests.jsonl"), key=lambda p: p.stat().st_mtime)
    for log in reversed(logs):
        records = [RequestRecord.parse(line, str(log)) for line in load_lines(log)]
        graphql = [r for r in records if r.limits is not None and r.limits.resource == "graphql"]
        if graphql:
            return max(graphql, key=lambda r: r.end).limits
    return None


def publish(run: Path, out: Path, files: dict[str, Json]) -> Path:
    """Write the sanitized files, refusing any that would carry an id, a URL or a title."""
    out.mkdir(parents=True, exist_ok=True)
    for name, value in files.items():
        rendered = json.dumps(value, indent=1, sort_keys=True).replace(str(run), "<run>")
        (out / name).write_text(rendered + "\n")
    if (run / "requests.jsonl").exists():
        shutil.copy(run / "requests.jsonl", out / "requests.jsonl")
    found = sanitize.leaks(out)
    if found:
        shutil.rmtree(out)
        raise MeasurementFailed(f"the sanitized output would carry identifying text: {found[:5]}")
    return out


async def live(scenario: Scenario, where: Paths, run: Path, out: Path) -> Path:
    """One quota-gated live probe through the guarded proxy.

    Whatever fails after the first request, the requests the run sent are published beside the
    failure, with whatever quota readings it took: a failed run still spent the allowance.
    """
    credential = token()
    scratch = board.scratch_board()
    log = run / "requests.jsonl"
    readings: dict[str, Json] = {}
    try:
        async with relay(run, allowed=(), upstream=where.upstream) as reader:
            client = Client(reader.endpoint, credential, log=log)
            before = await allowance.gate(
                client, where.quota_log, run.name, last_headers(where.runs)
            )
            readings["before"] = before.reading.to_json()
            readings["pauses"] = before.pauses
            context = await client.graphql(
                "CREATION_CONTEXT", board.ids_query(), board.ids_variables(scratch)
            )
            ids = board.board_ids(context)
        # The read-only pricing probe is allowed no write; the placement probe the test board,
        # the issues it creates and an interrupted run's recorded leftovers.
        state = placement.State(where.cache / "state" / "probe-issues.json")
        writes = (ids.repository, ids.project) if scenario is Scenario.PLACEMENT else ()
        leftovers = tuple(state.ids) if scenario is Scenario.PLACEMENT else ()
        async with relay(run, writes, upstream=where.upstream, issues=leftovers) as writer:
            client = Client(writer.endpoint, credential, log=log)
            try:
                match scenario:
                    case Scenario.PRICING:
                        result = (await pricing.probe(client, scratch, ids.repository)).to_json()
                    case _:
                        result = (await placement.probe(client, ids, state)).to_json()
            finally:
                readings["after"] = (await allowance.read(client, None)).to_json()
    except (GitHubError, ShapeError, placement.LeftBehind) as error:
        failed: dict[str, Json] = {"failed.json": {"error": str(error)}}
        if readings:
            failed["quota.json"] = readings
        publish(run, out, failed)
        raise MeasurementFailed(f"{scenario.value} failed: {error}") from error
    return publish(run, out, {"feasibility.json": result, "quota.json": readings})


async def offline(scenario: Scenario, build: Build, where: Paths, run: Path, out: Path) -> Path:
    """One synthetic case."""
    if scenario is Scenario.PROXY_REFUSAL:
        refused = await refusal.run(run)
        if not refused.all_as_expected:
            raise MeasurementFailed("the proxy did not refuse as expected; read result.json")
        return publish(run, out, {"result.json": refused.to_json()})
    hint = "build it (cargo build -p onetaskgraph, or spike/gaps/prototype/build.sh)"
    exe = executable(where.binaries[build], hint)
    onevcs = executable(where.onevcs, "set SPIKE_ONEVCS to an onevcs executable")
    if scenario is Scenario.SCREEN_UNION:
        result = await union.run(run, exe, onevcs)
        published = publish(run, out, {"result.json": result.to_json()})
        if not result.commands:
            raise MeasurementFailed(f"the copy screened nothing to replay; read {published}")
        if not result.all_agree:
            raise MeasurementFailed(
                f"the union disagreed with the per-item verdicts; read {published}"
            )
        return published
    case = reach.Case(scenario.value)
    return publish(run, out, {"result.json": (await reach.run(run, case, exe, onevcs)).to_json()})


async def measure(scenario: Scenario, build: Build) -> Path:
    """Run one scenario and publish its sanitized form."""
    where = paths()
    stamp = time.strftime("%Y%m%dT%H%M%S", time.gmtime())
    name = f"{scenario.value}-{build.value}-{stamp}"
    run = where.runs / name
    run.mkdir(parents=True)
    if scenario in (Scenario.PRICING, Scenario.PLACEMENT):
        return await live(scenario, where, run, where.logs / name)
    return await offline(scenario, build, where, run, where.logs / name)


def main() -> None:
    """Parse the invocation, measure, and exit with the documented status."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("scenario", choices=[s.value for s in Scenario])
    parser.add_argument("--build", choices=[b.value for b in Build], default=Build.HEAD.value)
    args = parser.parse_args()
    try:
        out = asyncio.run(measure(Scenario(args.scenario), Build(args.build)))
    except UsageError as error:
        sys.stderr.write(f"gaps: {error}\n")
        sys.exit(USAGE)
    except (
        MeasurementFailed,
        GitHubError,
        ShapeError,
        ProcessFailed,
        placement.LeftBehind,
    ) as error:
        sys.stderr.write(f"gaps: {error}\n")
        sys.exit(FAILED)
    sys.stdout.write(f"{out}\n")
