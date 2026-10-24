"""The live probes end to end, against the loopback stand-in for GitHub.

Each test drives the real `gaps` entry point as a subprocess, or the probe functions through
the real guarded proxy, with only GitHub itself replaced: every request crosses the relay and
its guard exactly as a live run's does.
"""

from __future__ import annotations

import asyncio
import json
import subprocess
from pathlib import Path

import pytest

from gaps import allowance, board, placement
from gaps.github import Client, GitHubError, Token
from gaps.json_shape import decode, load, load_lines, obj
from gaps.placement import AddOutcome, PlacementResult
from gaps.pricing import Named, PricingResult
from gaps.relay import relay
from tests import fake_github
from tests.conftest import PYTHON, gaps_env


def run_gaps(tmp_path: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
    """`python -m gaps <args>` with this test's cache and logs."""
    return subprocess.run(
        [PYTHON, "-m", "gaps", *args],
        env=gaps_env(tmp_path, **env),
        capture_output=True,
        text=True,
        check=False,
        timeout=300,
    )


def test_a_placement_probe_runs_end_to_end_and_deletes_every_issue_it_created(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """A placement probe runs end to end and deletes every issue it created."""
    stand_in, url = github
    done = run_gaps(tmp_path, "probe-placement", SPIKE_UPSTREAM=url, GH_PROJECTS_TOKEN="t0ken")
    assert done.returncode == 0, done.stderr
    out = Path(done.stdout.strip())
    assert done.stdout.count("\n") == 1 and out.parent == tmp_path / "logs"
    result = decode(PlacementResult, load(out / "feasibility.json"), "feasibility")
    assert result.placement.placed == result.placement.issues == 9
    assert result.placement.polls == 2
    assert result.board_items_in_create_answers == 0
    assert result.children_with_parent == 8
    assert result.idempotence.never_duplicated
    assert result.coalesced_write_answered == result.coalesced_write_aliases == 19
    assert result.reference_resolution_children == 8
    assert result.cleanup.deleted == 11 and result.cleanup.left == 0
    assert stand_in.issues == {}
    ops = [obj(line, "record").get("op") for line in load_lines(out / "requests.jsonl")]
    assert ops.count("ProbeNineteen") == placement.REPEATS
    assert {"before", "after"} <= set(json.loads((out / "quota.json").read_text()))


def test_a_pricing_probe_prices_every_shape_the_analysis_names(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """A pricing probe prices every shape the analysis names."""
    _, url = github
    done = run_gaps(tmp_path, "probe-alias-pricing", SPIKE_UPSTREAM=url, GH_PROJECTS_TOKEN="t0ken")
    assert done.returncode == 0, done.stderr
    result = decode(PricingResult, load(Path(done.stdout.strip()) / "feasibility.json"), "p")
    priced = {p.variant for p in result.dry_runs} | {p.variant for p in result.executed}
    assert {n.value for n in Named} <= priced
    assert {p.variant for p in result.executed} <= {p.variant for p in result.dry_runs}


@pytest.mark.parametrize(
    ("env", "said"),
    [
        ({}, "GH_PROJECTS_TOKEN"),
        ({"GH_PROJECTS_TOKEN": "t", "SPIKE_UPSTREAM": "https://api.github.com"}, "loopback"),
        ({"GH_PROJECTS_TOKEN": "t", "XDG_CACHE_HOME": "relative/cache"}, "absolute"),
    ],
)
def test_a_wrong_environment_exits_2_naming_what_is_wrong(
    tmp_path: Path, env: dict[str, str], said: str
) -> None:
    """A wrong environment exits 2 naming what is wrong."""
    done = run_gaps(tmp_path, "probe-placement", **env)
    assert done.returncode == 2
    assert said in done.stderr and done.stdout == ""


def test_an_unknown_scenario_exits_2(tmp_path: Path) -> None:
    """An unknown scenario exits 2."""
    assert run_gaps(tmp_path, "probe-everything").returncode == 2


async def probe(
    url: str, tmp_path: Path
) -> tuple[PlacementResult | Exception, placement.State, Client]:
    """The placement probe through the real relay, its failure answered rather than raised."""
    state = placement.State(tmp_path / "state.json")
    ids = board.BoardIds(
        board.RepositoryId(fake_github.REPOSITORY),
        board.ProjectId(fake_github.PROJECT),
        board.FieldId(fake_github.FIELD),
        board.OptionId(fake_github.OPTION),
    )
    async with relay(tmp_path, (ids.repository, ids.project), upstream=url) as proxy:
        client = Client(proxy.endpoint, Token("t0ken"))
        try:
            return await placement.probe(client, ids, state), state, client
        except (GitHubError, placement.LeftBehind) as error:
            return error, state, client


def test_an_add_refused_as_already_present_is_recorded_not_failed(
    tmp_path: Path, github: tuple[fake_github.Board, str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """An add refused as already present is recorded not failed."""
    stand_in, url = github
    stand_in.refuse_present_adds = True
    monkeypatch.setattr(placement, "RACE_SETTLE", 0.1)
    result, _, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, PlacementResult)
    assert result.idempotence.placed_outcome is AddOutcome.REFUSED_ALREADY_EXISTS
    assert result.idempotence.never_duplicated
    assert stand_in.issues == {}


def test_issues_never_placed_fail_the_probe_and_are_still_deleted(
    tmp_path: Path, github: tuple[fake_github.Board, str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """Issues never placed fail the probe and are still deleted."""
    stand_in, url = github
    stand_in.placement_reads = 10**6
    monkeypatch.setattr(placement, "PLACEMENT_TIMEOUT", 1.0)
    result, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, GitHubError) and "never placed" in str(result)
    assert stand_in.issues == {} and state.ids == []


def test_a_refused_delete_leaves_the_ids_recorded_and_the_next_probe_deletes_them(
    tmp_path: Path, github: tuple[fake_github.Board, str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """A refused delete leaves the ids recorded and the next probe deletes them."""
    stand_in, url = github
    monkeypatch.setattr(placement, "RACE_SETTLE", 0.1)
    stand_in.fail_deletes = True
    result, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, placement.LeftBehind)
    assert len(state.ids) == 11 and len(stand_in.issues) == 11
    stand_in.fail_deletes = False
    again, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(again, PlacementResult)
    assert again.cleanup.deleted == 22 and stand_in.issues == {} and state.ids == []


def test_the_gate_pauses_under_forty_percent_and_proceeds_once_the_bucket_recovers(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """The gate pauses under forty percent and proceeds once the bucket recovers."""
    stand_in, url = github
    stand_in.remaining = stand_in.used + 1000  # 20% of 5,000
    pauses: list[float] = []

    async def recover(seconds: float) -> None:
        """Pause, and let the bucket recover meanwhile."""
        pauses.append(seconds)
        stand_in.remaining = stand_in.used + 4000

    async def gate() -> allowance.Reading:
        """The gate through the real relay."""
        async with relay(tmp_path, (), upstream=url) as proxy:
            client = Client(proxy.endpoint, Token("t0ken"))
            return await allowance.gate(client, tmp_path / "allowance.jsonl", "test", None, recover)

    reading = asyncio.run(gate())
    assert len(pauses) == 1 and pauses[0] >= allowance.MIN_PAUSE
    assert reading.least.remaining / reading.least.limit >= 0.4
    log = tmp_path / "allowance.jsonl"
    events = [obj(line, "entry").get("event") for line in load_lines(log)]
    assert events == ["reading", "pause", "reading"]
