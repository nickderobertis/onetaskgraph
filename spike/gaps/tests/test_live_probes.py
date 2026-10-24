"""The live probes end to end, against the loopback stand-in for GitHub.

Each test drives the real `gaps` entry point as a subprocess, or the probe functions through
the real guarded proxy, with only GitHub itself replaced: every request crosses the relay and
its guard exactly as a live run's does.
"""

from __future__ import annotations

import asyncio
import json
import os
import time
from pathlib import Path

import pytest

from gaps import allowance, board, cli, placement, proc
from gaps.github import Client, GitHubError, Kind, RateLimit, RequestRecord, Token
from gaps.json_shape import ShapeError, decode, load, load_lines, obj
from gaps.placement import AddOutcome, PlacementResult
from gaps.pricing import Named, PricingResult
from gaps.relay import relay
from tests import fake_github
from tests.conftest import PYTHON, gaps_env, spawn


def run_gaps(tmp_path: Path, *args: str, **env: str) -> proc.Completed:
    """`python -m gaps <args>` with this test's cache and logs."""
    return spawn([PYTHON, "-m", "gaps", *args], gaps_env(tmp_path, **env), timeout=300)


def test_a_placement_probe_runs_end_to_end_and_deletes_every_issue_it_created(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """A placement probe runs end to end and deletes every issue it created."""
    stand_in, url = github
    done = run_gaps(tmp_path, "probe-placement", SPIKE_UPSTREAM=url, GH_PROJECTS_TOKEN="t0ken")
    assert done.returncode == 0, done.err
    out = Path(done.out.strip())
    assert done.out.count("\n") == 1 and out.parent == tmp_path / "logs"
    result = decode(PlacementResult, load(out / "feasibility.json"), "feasibility")
    assert result.placement.placed == result.placement.issues == 9
    assert result.placement.polls == 2
    assert result.board_items_in_create_answers == 0
    assert result.children_with_parent == 8
    assert result.idempotence.one_item_each_after_settling
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
    assert done.returncode == 0, done.err
    result = decode(PricingResult, load(Path(done.out.strip()) / "feasibility.json"), "p")
    priced = {p.variant for p in result.dry_runs} | {p.variant for p in result.executed}
    assert {n.value for n in Named} <= priced
    assert {p.variant for p in result.executed} <= {p.variant for p in result.dry_runs}


@pytest.mark.parametrize(
    ("env", "said"),
    [
        ({}, "GH_PROJECTS_TOKEN"),
        ({"GH_PROJECTS_TOKEN": "t", "SPIKE_UPSTREAM": "https://api.github.com"}, "loopback"),
        ({"GH_PROJECTS_TOKEN": "t", "XDG_CACHE_HOME": "relative/cache"}, "absolute"),
        ({"GH_PROJECTS_TOKEN": "t\x01ken"}, "GH_PROJECTS_TOKEN"),
    ],
)
def test_a_wrong_environment_exits_2_naming_what_is_wrong(
    tmp_path: Path, env: dict[str, str], said: str
) -> None:
    """A wrong environment exits 2 naming what is wrong."""
    done = run_gaps(tmp_path, "probe-placement", **env)
    assert done.returncode == 2
    assert said in done.err and done.out == ""


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
    leftovers = tuple(state.ids)
    async with relay(tmp_path, (ids.repository, ids.project), url, leftovers) as proxy:
        client = Client(proxy.endpoint, Token("t0ken"))
        try:
            return await placement.probe(client, ids, state), state, client
        except (GitHubError, ShapeError, placement.LeftBehind) as error:
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
    assert result.idempotence.one_item_each_after_settling
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
            log = tmp_path / "allowance.jsonl"
            gated = await allowance.gate(client, log, "test", None, recover)
            assert gated.pauses == 1
            return gated.reading

    reading = asyncio.run(gate())
    assert len(pauses) == 1 and pauses[0] >= allowance.MIN_PAUSE
    assert reading.least.remaining / reading.least.limit >= 0.4
    log = tmp_path / "allowance.jsonl"
    events = [obj(line, "entry").get("event") for line in load_lines(log)]
    assert events == ["reading", "pause", "reading"]


def test_a_failed_probe_publishes_its_log_and_quota_beside_the_failure_and_exits_1(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """A failed probe publishes its log and quota beside the failure and exits 1."""
    stand_in, url = github
    stand_in.fail_deletes = True
    done = run_gaps(tmp_path, "probe-placement", SPIKE_UPSTREAM=url, GH_PROJECTS_TOKEN="t0ken")
    assert done.returncode == 1 and "probe issue(s) left" in done.err
    [run] = (tmp_path / "logs").iterdir()
    assert {p.name for p in run.iterdir()} == {"failed.json", "quota.json", "requests.jsonl"}
    assert "left" in json.loads((run / "failed.json").read_text())["error"]


def test_output_that_would_carry_identifying_text_is_refused_and_removed(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """Output that would carry identifying text is refused and removed."""
    stand_in, url = github
    stand_in.create_error = "see https://github.com/someone/somewhere/issues/7"
    done = run_gaps(tmp_path, "probe-placement", SPIKE_UPSTREAM=url, GH_PROJECTS_TOKEN="t0ken")
    assert done.returncode == 1 and "identifying text" in done.err
    assert not (tmp_path / "logs").exists() or not any((tmp_path / "logs").iterdir())


def test_the_launcher_reads_the_credential_file_and_keeps_an_exported_credential(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """The launcher reads the credential file and keeps an exported credential."""
    stand_in, url = github
    token_file = tmp_path / "secrets.env"
    token_file.write_text('GH_PROJECTS_TOKEN="from-file"\n')
    launcher = [str(Path(__file__).resolve().parents[1] / "gaps.sh"), "probe-alias-pricing"]
    base = gaps_env(tmp_path, SPIKE_UPSTREAM=url, SPIKE_TOKEN_ENV=str(token_file))
    first = spawn(launcher, base)
    assert first.returncode == 0, first.err
    assert set(stand_in.tokens) == {"Bearer from-file"}
    stand_in.tokens.clear()
    exported = base | {"GH_PROJECTS_TOKEN": "exported"}
    second = spawn(launcher, exported)
    assert second.returncode == 0, second.err
    assert set(stand_in.tokens) == {"Bearer exported"}


def test_a_create_answered_in_part_fails_and_every_issue_it_did_create_is_deleted(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """A create answered in part fails, and every issue it did create is deleted."""
    stand_in, url = github
    stand_in.answer_creates = 4
    result, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, GitHubError) and "not created" in str(result)
    assert stand_in.issues == {} and state.ids == []


def test_cleanup_deletes_only_this_probes_issues_and_forgets_those_already_gone(
    tmp_path: Path, github: tuple[fake_github.Board, str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """Cleanup deletes only this probe's issues and forgets those already gone."""
    stand_in, url = github
    monkeypatch.setattr(placement, "RACE_SETTLE", 0.1)
    stand_in.issues["I_foreign_title"] = fake_github.Issue("I_foreign_title", "a person's", None)
    foreign = fake_github.Issue("I_foreign_repo", "bcg-probe x", None, repository="R_other")
    stand_in.issues["I_foreign_repo"] = foreign
    seeded = ["I_foreign_title", "I_foreign_repo", "I_gone"]
    (tmp_path / "state.json").write_text(json.dumps(seeded))
    result, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, placement.LeftBehind)
    assert set(stand_in.issues) == {"I_foreign_title", "I_foreign_repo"}
    assert sorted(state.ids) == ["I_foreign_repo", "I_foreign_title"]


def test_an_unreadable_ownership_read_deletes_nothing_and_keeps_every_recorded_id(
    tmp_path: Path, github: tuple[fake_github.Board, str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """An unreadable ownership read deletes nothing and keeps every recorded id."""
    stand_in, url = github
    monkeypatch.setattr(placement, "RACE_SETTLE", 0.1)
    stand_in.fail_ownership_read = True
    result, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, GitHubError) and "FORBIDDEN" in str(result)
    assert len(stand_in.issues) == 11 and len(state.ids) == 11


def test_a_reading_is_the_least_of_its_figures_and_ignores_headers_of_a_closed_window(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """A reading is the least of its figures, and ignores the headers of a closed window."""
    stand_in, url = github
    stand_in.rest_remaining = 4900

    async def read(headers: RateLimit | None) -> allowance.Reading:
        """One reading through the real relay."""
        async with relay(tmp_path, (), upstream=url) as proxy:
            return await allowance.read(Client(proxy.endpoint, Token("t0ken")), headers)

    own = asyncio.run(read(None))
    assert own.rest_endpoint.remaining == 4900
    assert own.least.remaining == min(b.remaining for b in own.own) < 4900
    lower = asyncio.run(
        read(RateLimit(limit=5000, remaining=10, used=4990, reset=4102444800, resource="graphql"))
    )
    assert lower.least.remaining == 10
    closed = asyncio.run(
        read(RateLimit(limit=5000, remaining=10, used=4990, reset=1, resource="graphql"))
    )
    assert closed.least.remaining > 10


@pytest.mark.parametrize(
    ("framing", "status"),
    [
        (fake_github.Chunking.WHOLE, 0),
        (fake_github.Chunking.CUT_SHORT, 1),
        (fake_github.Chunking.NO_FINAL_LINE, 1),
        (fake_github.Chunking.LONGER_THAN_SENT, 1),
    ],
    ids=lambda v: v.name if isinstance(v, fake_github.Chunking) else str(v),
)
def test_a_chunked_answer_is_read_whole_and_one_cut_short_is_refused(
    tmp_path: Path,
    github: tuple[fake_github.Board, str],
    framing: fake_github.Chunking,
    status: int,
) -> None:
    """A chunked answer is read whole, and one cut short is refused rather than half read."""
    stand_in, url = github
    stand_in.chunked = framing
    done = run_gaps(tmp_path, "probe-alias-pricing", SPIKE_UPSTREAM=url, GH_PROJECTS_TOKEN="t0ken")
    assert done.returncode == status, done.err
    if status:
        assert "HTTP 502" in done.err
        [run] = (tmp_path / "logs").iterdir()
        assert (run / "failed.json").exists()
        statuses = [obj(r, "r").get("status") for r in load_lines(run / "requests.jsonl")]
        assert 502 in statuses


def test_a_create_answered_malformed_in_part_still_deletes_every_issue_it_named(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """A create answered malformed in part still deletes every issue its answer named."""
    stand_in, url = github
    stand_in.malformed_create = 0
    result, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, ShapeError)
    assert len(stand_in.issues) == 1 and state.ids == []


@pytest.mark.parametrize(
    ("burned", "resets_in", "pauses"),
    [(10, 60, 0), (990, 3600, 1)],
    ids=["burn resets the bucket first", "burn exhausts it first"],
)
def test_under_forty_percent_the_burn_rate_decides_whether_to_pause(
    tmp_path: Path,
    github: tuple[fake_github.Board, str],
    burned: int,
    resets_in: int,
    pauses: int,
) -> None:
    """Under 40%, the timing spike's rule proceeds only when the burn resets the bucket first."""
    stand_in, url = github
    now = time.time()
    stand_in.reset = int(now) + resets_in
    stand_in.remaining = stand_in.used + 1000  # 20% of 5,000
    earlier = allowance.Bucket(5000, stand_in.used - burned, 1000 + burned, stand_in.reset)
    previous = allowance.Reading(now - 60, earlier, (earlier,), None)
    log = tmp_path / "allowance.jsonl"
    log.write_text(json.dumps({"event": "reading", "reading": previous.to_json()}) + "\n")
    slept: list[float] = []

    async def recover(seconds: float) -> None:
        """Pause, and let the bucket recover meanwhile."""
        slept.append(seconds)
        stand_in.remaining = stand_in.used + 4000

    async def gate() -> allowance.Reading:
        """The gate through the real relay."""
        async with relay(tmp_path, (), upstream=url) as proxy:
            client = Client(proxy.endpoint, Token("t0ken"))
            gated = await allowance.gate(client, log, "test", None, recover)
            assert gated.pauses == pauses
            return gated.reading

    asyncio.run(gate())
    assert len(slept) == pauses


def test_the_gate_reads_the_last_graphql_headers_of_the_latest_run_that_recorded_any(
    tmp_path: Path,
) -> None:
    """The gate reads the last GraphQL headers of the latest run that recorded any."""
    runs = tmp_path / "runs"

    def run(name: str, records: list[RequestRecord], age: float) -> None:
        directory = runs / name
        directory.mkdir(parents=True)
        log = directory / "requests.jsonl"
        log.write_text("".join(json.dumps(r.to_json()) + "\n" for r in records))
        os.utime(log, (time.time() - age, time.time() - age))

    def record(end: float, remaining: int, resource: str) -> RequestRecord:
        limits = RateLimit(5000, remaining, 5000 - remaining, 4102444800, resource)
        return RequestRecord(1, Kind.QUERY, "q", end - 1, end, 200, None, (), limits, 1)

    run("probe-a", [record(10, 3000, "graphql"), record(20, 2500, "graphql")], age=300)
    run("probe-b", [record(30, 4900, "core")], age=100)
    run("other-c", [record(40, 1, "graphql")], age=50)
    chosen = cli.last_headers(runs)
    assert chosen is not None and chosen.remaining == 2500
    assert cli.last_headers(tmp_path / "none") is None


def test_a_delete_answered_null_keeps_the_issue_recorded_for_the_next_probe(
    tmp_path: Path, github: tuple[fake_github.Board, str], monkeypatch: pytest.MonkeyPatch
) -> None:
    """A delete answered null, with no error, keeps the issue recorded for the next probe."""
    stand_in, url = github
    monkeypatch.setattr(placement, "RACE_SETTLE", 0.1)
    stand_in.null_deletes = True
    result, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, placement.LeftBehind)
    assert len(state.ids) == 11


def test_a_request_lost_in_transport_is_still_recorded(tmp_path: Path) -> None:
    """A request whose connection closes before any answer is still recorded, then refused."""

    async def lose() -> tuple[GitHubError | None, list[RequestRecord]]:
        """Send one request to a server that reads it and closes without answering."""

        async def close(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
            await reader.readuntil(b"\r\n\r\n")
            writer.close()

        server = await asyncio.start_server(close, "127.0.0.1", 0)
        port = server.sockets[0].getsockname()[1]
        log = tmp_path / "requests.jsonl"
        client = Client(f"http://127.0.0.1:{port}/graphql", Token("t0ken"), log=log)
        try:
            await client.graphql("lost", "query { viewer { login } }", {})
        except GitHubError as error:
            return error, client.records
        finally:
            server.close()
        return None, client.records

    error, records = asyncio.run(lose())
    assert error is not None and "no answer" in str(error)
    assert [(r.op, r.status) for r in records] == [("lost", 0)]
    logged = [obj(line, "r").get("status") for line in load_lines(tmp_path / "requests.jsonl")]
    assert logged == [0]


@pytest.mark.parametrize("odd", list(fake_github.OddAdd), ids=lambda o: o.name)
def test_an_unexpected_add_is_recorded_as_such_and_every_issue_still_deleted(
    tmp_path: Path,
    github: tuple[fake_github.Board, str],
    monkeypatch: pytest.MonkeyPatch,
    odd: fake_github.OddAdd,
) -> None:
    """An add refused otherwise, answering another item, or malformed is recorded, not fatal."""
    stand_in, url = github
    monkeypatch.setattr(placement, "RACE_SETTLE", 0.1)
    stand_in.odd_add = odd
    result, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, PlacementResult)
    assert result.idempotence.racing_outcomes[1] is AddOutcome.OTHER
    assert not result.idempotence.one_item_each_after_settling
    assert stand_in.issues == {} and state.ids == []


def test_a_rest_request_lost_in_transport_is_still_recorded(tmp_path: Path) -> None:
    """A REST request whose connection closes before any answer is still recorded."""

    async def lose() -> list[RequestRecord]:
        """Send one REST request to a server that closes without answering."""

        async def close(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
            await reader.readuntil(b"\r\n\r\n")
            writer.close()

        server = await asyncio.start_server(close, "127.0.0.1", 0)
        port = server.sockets[0].getsockname()[1]
        client = Client(f"http://127.0.0.1:{port}/graphql", Token("t0ken"))
        with pytest.raises(GitHubError, match="no answer"):
            await client.rest_get("GET /rate_limit", "/rate_limit")
        server.close()
        return client.records

    records = asyncio.run(lose())
    assert [(r.kind, r.status) for r in records] == [(Kind.REST, 0)]


def test_a_cleanup_failing_while_a_probe_fails_reports_both(
    tmp_path: Path, github: tuple[fake_github.Board, str]
) -> None:
    """A cleanup that leaves issues while handling a probe's own failure says so, citing it."""
    stand_in, url = github
    stand_in.answer_creates = 4
    stand_in.fail_deletes = True
    result, state, _ = asyncio.run(probe(url, tmp_path))
    assert isinstance(result, placement.LeftBehind) and "not created" in str(result)
    assert isinstance(result.__cause__, GitHubError)
    assert len(state.ids) == 4 and len(stand_in.issues) == 4
