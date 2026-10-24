"""The offline scenarios end to end: `python -m gaps` driving the real CLI and the real onevcs.

`head` is this checkout's `target/debug/onetaskgraph` (Nx builds it before this target), and
`closure` the prototype `spike/gaps/prototype/build.sh` builds into the user's cache (the
project's `build` target, which `test` depends on).
"""

from __future__ import annotations

import asyncio
import os
from pathlib import Path

import pytest

from gaps import cli, proc, reach, refusal, union
from gaps.boundary import Verdict
from gaps.json_shape import decode, load
from gaps.reach import ReachResult
from gaps.refusal import RefusalResult
from gaps.union import Plant, UnionResult
from tests.conftest import PYTHON, REPO, gaps_env, spawn

CACHE = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache")
BINARIES = {
    "SPIKE_HEAD_BINARY": str(REPO / "target/debug/onetaskgraph"),
    "SPIKE_CLOSURE_BINARY": str(
        CACHE / "board-copy-gaps-2026-10-24/prototype/target/debug/onetaskgraph"
    ),
    "SPIKE_ONEVCS": str(Path.home() / "ai-orchestrator/.venv/bin/onevcs"),
}


def run_case(tmp_path: Path, scenario: str, build: str, **env: str) -> proc.Completed:
    """One offline scenario through the entry point."""
    return spawn(
        [PYTHON, "-m", "gaps", scenario, "--build", build], gaps_env(tmp_path, **(BINARIES | env))
    )


def result(done: proc.Completed) -> Path:
    """The published result of a run that must have succeeded."""
    assert done.returncode == 0, done.err
    return Path(done.out.strip()) / "result.json"


def test_on_head_an_unrelated_unavailable_private_board_fails_the_public_copy(
    tmp_path: Path,
) -> None:
    """On head an unrelated unavailable private board fails the public copy."""
    reach = decode(ReachResult, load(result(run_case(tmp_path, "reach-public", "head"))), "r")
    assert [s.returncode for s in reach.steps] == [1]
    assert [q.operation for q in reach.steps[0].recorder_requests] == ["query <search>"]
    assert not reach.destination_changed
    assert reach.screens.check > 0


def test_with_the_closure_the_public_copy_succeeds_without_asking_the_private_doubles(
    tmp_path: Path,
) -> None:
    """With the closure the public copy succeeds without asking the private doubles."""
    reach = decode(ReachResult, load(result(run_case(tmp_path, "reach-public", "closure"))), "r")
    assert [s.returncode for s in reach.steps] == [0, 0]
    assert reach.private_requests == 0
    assert reach.destination_changed and reach.destination_files == 10


@pytest.mark.parametrize("scenario", ["reach-private-item", "reach-unresolvable-ref"])
def test_with_the_closure_a_private_item_or_an_unresolvable_reference_is_refused_before_any_write(
    tmp_path: Path, scenario: str
) -> None:
    """With the closure a private item or an unresolvable reference is refused before any write."""
    reach = decode(ReachResult, load(result(run_case(tmp_path, scenario, "closure"))), "r")
    assert [s.returncode for s in reach.steps] == [1]
    assert not reach.destination_changed and reach.destination_files == 0
    assert reach.screens.check == 0 and reach.private_requests == 0


def test_with_the_closure_a_declared_private_reference_is_withheld_and_not_reported(
    tmp_path: Path,
) -> None:
    """With the closure a declared private reference is withheld and not reported."""
    reach = decode(
        ReachResult, load(result(run_case(tmp_path, "reach-private-ref", "closure"))), "r"
    )
    assert [s.returncode for s in reach.steps] == [0, 0]
    assert [q.operation for s in reach.steps for q in s.recorder_requests] == ["query {"]
    assert not any(s.output_names_the_reference or s.output_says_withheld for s in reach.steps)
    assert reach.destination_files == 10 and not reach.destination_names_private


def test_union_screening_agrees_with_per_item_screening_in_every_variant(tmp_path: Path) -> None:
    """Union screening agrees with per item screening in every variant."""
    union = decode(UnionResult, load(result(run_case(tmp_path, "screen-union", "closure"))), "u")
    assert union.all_agree
    assert set(union.commands) == {"project-copy", "document-copy"}
    for variants in union.commands.values():
        verdicts = {v.plant: v.unioned.verdict.name for v in variants}
        assert verdicts[Plant.NONE] == "PASS"
        assert {verdicts[p] for p in Plant if p is not Plant.NONE} == {"REFUSE"}
        assert all(v.unioned.calls == 2 for v in variants)


def test_the_proxy_refuses_every_write_to_another_destination(tmp_path: Path) -> None:
    """The proxy refuses every write to another destination."""
    refusal = decode(RefusalResult, load(result(run_case(tmp_path, "proxy-refusal", "head"))), "r")
    assert refusal.all_as_expected
    assert [o.reached_upstream for o in refusal.cases].count(1) == 6


def test_a_missing_binary_exits_2_naming_it(tmp_path: Path) -> None:
    """A missing binary exits 2 naming it."""
    done = run_case(tmp_path, "reach-public", "head", SPIKE_HEAD_BINARY=str(tmp_path / "absent"))
    assert done.returncode == 2 and "no executable" in done.err


def test_with_the_closure_a_recopy_over_marked_tasks_asks_no_private_source(
    tmp_path: Path,
) -> None:
    """With the closure, a re-copy over marked tasks asks no private source."""
    reach = decode(ReachResult, load(result(run_case(tmp_path, "reach-recopy", "closure"))), "r")
    assert [s.returncode for s in reach.steps] == [0, 0, 0, 0]
    assert reach.private_requests == 0 and reach.tasks_unmarked == 0


def test_with_the_closure_a_recopy_over_unmarked_tasks_asks_and_fails_closed(
    tmp_path: Path,
) -> None:
    """With the closure, a re-copy over unmarked tasks asks the private sources and fails closed."""
    done = run_case(tmp_path, "reach-recopy-unmarked", "closure")
    reach = decode(ReachResult, load(result(done)), "r")
    assert reach.tasks_unmarked == 8
    assert [s.returncode for s in reach.steps] == [0, 0, 1]
    assert [q.operation for q in reach.steps[2].recorder_requests] == ["query <search>"]


def test_a_copy_that_screens_nothing_fails_the_union_experiment_with_exit_1(tmp_path: Path) -> None:
    """A copy that screens nothing gives the union nothing to replay, and exits 1."""
    stub = tmp_path / "onetaskgraph"
    stub.write_text(
        "#!/bin/sh\n[ \"$1\" = --version ] && echo 'onetaskgraph 0.0.0' && exit 0\nexit 1\n"
    )
    stub.chmod(0o755)
    done = run_case(tmp_path, "screen-union", "closure", SPIKE_CLOSURE_BINARY=str(stub))
    assert done.returncode == 1 and "screened nothing" in done.err


def test_a_disagreeing_union_or_an_unexpected_refusal_fails_its_scenario(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A disagreeing union, or a refusal not as expected, fails its scenario.

    Asserted by handing `offline` each experiment's failing result rather than end to end:
    producing one for real needs a guard that lets a refused write through, or an onevcs whose
    union disagrees with itself, and both are what these experiments exist to catch.
    """
    where = cli.paths()
    copy = reach.ReachResult(
        reach.Case.PUBLIC, "", "", (), 0, False, 0, False, 0, reach.Screens(0, 0, 0, 0.0, ())
    )
    way = union.Way(1, 0, 1, Verdict.PASS, (), 0.0)
    refused = union.Way(1, 0, 1, Verdict.REFUSE, (), 0.0)
    disagreeing = union.UnionResult(
        copy, {"project-copy": (union.Variant(union.Plant.NONE, way, refused),)}
    )
    unexpected = refusal.RefusalResult((refusal.Outcome("a write", 200, 403, 1),))

    async def fake_union(*_: object) -> union.UnionResult:
        return disagreeing

    async def fake_refusal(*_: object) -> refusal.RefusalResult:
        return unexpected

    monkeypatch.setattr(union, "run", fake_union)
    monkeypatch.setattr(refusal, "run", fake_refusal)
    for scenario in (cli.Scenario.SCREEN_UNION, cli.Scenario.PROXY_REFUSAL):
        with pytest.raises(cli.MeasurementFailed):
            asyncio.run(cli.offline(scenario, cli.Build.HEAD, where, tmp_path, tmp_path / scenario))
