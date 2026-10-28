# llmlint: ignore-file[new_code_lands_in_a_project] The report's own recomputation and its tests, run by hand beside the measurement record whose command the report names; docs/measurements is no Nx project, and making it one is a change to the project graph and its four guarded selections, outside this measurement.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] The raw records these fixtures write are the proxy's, the runner's and the host audit's shapes, and every one is passed through the real harness/sanitize.write, so the analyzer is held to the sanitizer's output here; this suite sits outside every gate only because docs/measurements is no Nx project, as the new_code_lands_in_a_project note states.
"""The analyzer's parser and reconciliation, driven through its own command line.

    python3 -m pytest docs/measurements/board-copy-targets-2026-10-26/test_analyze.py

Every capture a case reads is produced the way a measurement produces it: raw run records, in
the shapes the proxy, the runner and the host audit write, reduced by `harness/sanitize.py`
itself. So a change to what the sanitizer writes that the analyzer does not read fails here. The
last cases reconcile the committed report with the committed captures.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE / "harness"))

import sanitize

import analyze

RESET = 1_800_000_000
OTHER_RESET = 1_799_999_000
BLOCKS = tuple(analyze.TABLES)


def graphql(
    seq: int, op: str, start: float, end: float, kind: str = "query", **extra: object
) -> dict:
    """One request as the proxy records it raw."""
    return {
        "seq": seq,
        "start": start,
        "end": end,
        "ms": round((end - start) * 1000, 1),
        "method": "POST",
        "host": extra.pop("host", "api.github.com"),
        "path": "/graphql",
        "kind": kind,
        "op": op,
        "req_bytes": 1,
        "resp_bytes": 1,
        "status": 200,
        "cost": None,
        "errors": [],
        "limits": {
            "x-ratelimit-resource": "graphql",
            "x-ratelimit-reset": str(RESET),
            "x-ratelimit-used": "100",
            "x-ratelimit-limit": "5000",
            "x-ratelimit-remaining": "4900",
        },
        **extra,
    }


def upload(seq: int, start: float, end: float) -> dict:
    return graphql(seq, "POST /user-attachments/assets", start, end, kind="rest") | {
        "host": "uploads.github.com",
        "path": "/user-attachments/assets?name=x",
        "limits": {},
    }


def tunnel(seq: int, start: float, end: float) -> dict:
    return graphql(seq, "CONNECT github.com", start, end, kind="tunnel") | {
        "method": "CONNECT",
        "host": "github.com",
        "path": "",
        "limits": {},
    }


def reading(at: float, used: int, reset: int = RESET) -> dict:
    return {"at": at, "used": used, "remaining": 5000 - used, "reset": reset}


def buckets(remaining: int = 4900) -> dict:
    bucket = {
        "limit": 5000,
        "used": 5000 - remaining,
        "remaining": remaining,
        "reset": RESET,
    }
    return {
        "core": bucket,
        "graphql": bucket,
        "graphql (GET /rate_limit)": bucket,
        "graphql (rateLimit)": bucket
        | {
            "readings": [remaining] * 3,
            "used_readings": [5000 - remaining] * 3,
            "reset_readings": ["2027-01-15T08:00:00Z"] * 3,
        },
    }


def store_step(name: str, start: float, end: float) -> dict:
    verb = name.split("-")[0]
    return {
        "step": name,
        "argv": ["/opt/onetaskgraph", verb, "copy", "notes:P", "--to", "board"],
        "start": start,
        "end": end,
        "returncode": 0,
    }


def take(
    root: Path,
    name: str,
    requests: list[dict],
    steps: list[dict],
    host: list[dict] | None = None,
    screen: list[dict] | None = None,
    burn: tuple[list[dict], list[dict]] | None = None,
    events: list[dict] | None = None,
) -> Path:
    """Write one run's raw records, as the runner leaves them, and sanitize them into `root`."""
    raw = root.parent / "raw" / name
    raw.mkdir(parents=True)

    def lines(file: str, records: list[dict]) -> None:
        (raw / file).write_text("".join(json.dumps(r) + "\n" for r in records))

    lines("proxy.jsonl", requests)
    lines("steps.jsonl", steps)
    if host is not None:
        lines("host.jsonl", host)
    lines("screen.jsonl", screen or [])
    before, after = burn or (
        [reading(0.0, 100), reading(2.0, 102)],
        [reading(20.0, 120), reading(22.0, 122)],
    )
    (raw / "burn-before.json").write_text(json.dumps(before))
    (raw / "burn-after.json").write_text(json.dumps(after))
    (raw / "quota-before.json").write_text(
        json.dumps({"at": 0.0, "buckets": buckets()})
    )
    (raw / "quota-after.json").write_text(
        json.dumps({"at": 30.0, "buckets": buckets()})
    )
    gate = {
        "event": "reading",
        "at": 0.0,
        "buckets": buckets(),
        "verdicts": {"core": {"proceed": True}, "graphql": {"proceed": True}},
    }
    (raw / "quota-events.json").write_text(json.dumps(events or [gate]))
    (raw / "versions.json").write_text(json.dumps({"store": "onetaskgraph 0.12.7"}))
    (raw / "conditions.json").write_text(
        json.dumps({"sources.notes.visibility": {"value": "private", "layer": "file"}})
    )
    (raw / "workload.json").write_text(
        json.dumps(
            {
                "project": [10, 10],
                "task t1": [5, 5],
                "document": [7, 7],
                "image": [3, 3],
            }
        )
    )
    (raw / "cache-before.json").write_text(json.dumps({"files": 0, "bytes": 0}))
    (raw / "cache-after.json").write_text(json.dumps({"files": 2, "bytes": 9}))
    return sanitize.write(raw, root / name)


def host_events(start: float = 1.0) -> list[dict]:
    return [
        {"kind": "interpreter", "name": "start", "start": start, "pid": 7},
        {
            "kind": "step",
            "name": "plan_review.review_key",
            "start": 1.5,
            "end": 2.0,
            "pid": 7,
        },
        {
            "kind": "store",
            "argv": ["project", "copy", "authoring:P", "--to", "plans"],
            "start": 2.5,
            "end": 5.0,
            "returncode": 0,
            "pid": 7,
        },
        {"kind": "interpreter", "name": "exit", "start": 5.5, "pid": 7},
    ]


def captures(root: Path) -> Path:
    """Seven rows: a bare first copy of two GraphQL queries, an aliased create, an upload and a
    refused create; every other row one query inside one copy; the host rows timed by the host's
    audit."""
    prices: dict[str, int] = {}
    for scenario in (s.name for s in analyze.ROWS):
        name = f"{scenario}-20261028T050000"
        if scenario == "bare-first":
            requests = [
                graphql(1, "ORIGIN_LOOKUPS", 1.0, 2.0),
                graphql(2, "CREATE_ISSUES", 2.0, 4.0, kind="mutation", mutations=8),
                upload(3, 4.0, 5.0),
                graphql(4, "CREATED_BOARD_ITEMS", 6.0, 7.0),
                graphql(
                    5,
                    "CREATE_ISSUE",
                    7.0,
                    7.0,
                    kind="mutation",
                    host="refused",
                    mutations=1,
                ),
            ]
            prices[f"{name}/1"] = 1
            prices[f"{name}/4"] = 2
            screen = [
                {
                    "kind": "screen",
                    "name": "inspect",
                    "start": 5.0,
                    "end": 5.4,
                    "returncode": 0,
                }
            ]
            take(
                root,
                name,
                requests,
                [
                    store_step("project-copy", 0.5, 5.5),
                    store_step("document-copy", 6.0, 8.0),
                ],
                screen=screen,
            )
        elif scenario.startswith("host"):
            prices[f"{name}/1"] = 1
            take(
                root,
                name,
                [graphql(1, "SUB_ISSUES", 3.0, 4.0)],
                [store_step("copy-plan", 0.0, 6.0)],
                host=host_events(),
            )
        else:
            prices[f"{name}/1"] = 1
            take(
                root,
                name,
                [graphql(1, "SUB_ISSUES", 1.0, 2.0)],
                [store_step("project-copy", 0.5, 3.0)],
            )
    (root / "superseded").mkdir()
    (root / "pricing.json").write_text(
        json.dumps({"requests": prices, "mutation_rises": {}})
    )
    loopback = root / "loopback"
    loopback.mkdir()
    sent = {"requests by operation": {"reading a project's tasks": 1}, "points": 1}
    for budget in analyze.LOOPBACK_POINTS:
        (loopback / f"{budget}.json").write_text(
            json.dumps(
                {
                    "budget": budget,
                    "value": 1,
                    "detail": json.dumps(
                        {name: sent for name in analyze.CONFIGURATIONS}
                        | {"host configuration, private beside public": {"points": 1}}
                    ),
                }
            )
        )
    return root


def report(path: Path) -> Path:
    blocks = "".join(
        f"<!-- analyze:begin {name} -->\nstale\n<!-- analyze:end {name} -->\n\n"
        for name in BLOCKS
    )
    path.write_text("# A report\n\n" + blocks)
    return path


@pytest.fixture
def measured(tmp_path: Path) -> tuple[Path, Path]:
    return captures(tmp_path / "captures"), report(tmp_path / "report.md")


def run(measured: tuple[Path, Path], *flags: str) -> int:
    root, path = measured
    return analyze.main(["--captures", str(root), "--report", str(path), *flags])


def summary(measured: tuple[Path, Path]) -> dict:
    return {
        row["scenario"]: row
        for row in json.loads((measured[0] / "summary.json").read_text())["rows"]
    }


def retake(
    measured: tuple[Path, Path],
    scenario: str,
    prices: dict[int, int] | None = None,
    **raw: object,
) -> Path:
    """Replace one row's capture with a later one taken from `raw`'s records."""
    root = measured[0]
    for old in root.glob(f"{scenario}-*"):
        shutil.rmtree(old)
    name = f"{scenario}-20261028T060000"
    path = take(root, name, **raw)
    pricing = json.loads((root / "pricing.json").read_text())
    pricing["requests"] |= {
        f"{name}/{seq}": value for seq, value in (prices or {}).items()
    }
    (root / "pricing.json").write_text(json.dumps(pricing))
    return path


def test_a_report_whose_tables_are_stale_is_refused_naming_each_and_rewritten_by_write(
    measured, capsys
):
    assert run(measured) == 1
    error = capsys.readouterr().err
    assert "targets" in error and "writes" in error and "--write" in error
    assert run(measured, "--write") == 0
    assert run(measured) == 0
    assert "stale" not in measured[1].read_text()


def test_one_table_edited_by_hand_is_the_one_named(measured, capsys):
    run(measured, "--write")
    path = measured[1]
    path.write_text(path.read_text().replace("**within**", "**met**", 1))
    assert run(measured) == 1
    assert "differs from its captures in: targets;" in capsys.readouterr().err


def test_a_report_missing_a_table_or_naming_an_unknown_one_is_refused_even_by_write(
    measured, capsys
):
    path = measured[1]
    text = path.read_text().replace(
        "<!-- analyze:begin burn -->\nstale\n<!-- analyze:end burn -->\n", ""
    )
    path.write_text(
        text + "<!-- analyze:begin nonsense -->\n<!-- analyze:end nonsense -->\n"
    )
    assert run(measured, "--write") == 1
    error = capsys.readouterr().err
    assert (
        "burn (missing from the report)" in error
        and "nonsense (no such table)" in error
    )
    assert run(measured) == 1


def test_an_empty_block_is_filled_by_write(measured):
    path = measured[1]
    path.write_text(
        path.read_text().replace(
            "<!-- analyze:begin over -->\nstale\n", "<!-- analyze:begin over -->\n"
        )
    )
    assert run(measured, "--write") == 0
    expected = "<!-- analyze:begin over -->\nNo points or writes figure is over its target.\n<!-- analyze:end over -->"
    assert expected in path.read_text()
    assert run(measured) == 0


def test_the_latest_capture_of_a_scenario_is_its_row_and_a_superseded_one_never_is(
    measured,
):
    root = measured[0]
    earlier = root / "bare-unchanged-20261028T050000"
    # A superseded capture newer than both is never read; of the two live ones, the later wins.
    shutil.copytree(earlier, root / "superseded" / "bare-unchanged-20261028T090000")
    later = take(
        root,
        "bare-unchanged-20261028T070000",
        [graphql(1, "SUB_ISSUES", 1.0, 2.0)],
        [store_step("project-copy", 0.5, 3.0)],
    )
    pricing = json.loads((root / "pricing.json").read_text())
    pricing["requests"][f"{later.name}/1"] = 3
    (root / "pricing.json").write_text(json.dumps(pricing))
    assert earlier.exists()
    run(measured, "--write")
    row = summary(measured)["bare-unchanged"]
    assert row["capture"] == later.name
    assert row["points"]["figure"] == 3


def test_points_are_each_querys_own_price_plus_one_per_mutation_request_never_a_refused_one(
    measured,
):
    run(measured, "--write")
    first = summary(measured)["bare-first"]
    # Two queries priced 1 and 2, one mutation request at 1; the refused create is not sent.
    assert first["points"]["figure"] == 4
    assert first["points"]["at_five"] == 8
    assert first["writes"] == {
        "requests": 4,
        "issues_created": 8,
        "mutation_requests": 1,
        "mutations": 8,
        "uploads": 1,
        "writes": 9,
    }
    assert first["refused"] == 1


def test_a_query_without_a_dry_run_price_stops_the_analysis(measured):
    root = measured[0]
    prices = json.loads((root / "pricing.json").read_text())
    del prices["requests"]["bare-first-20261028T050000/4"]
    (root / "pricing.json").write_text(json.dumps(prices))
    with pytest.raises(
        SystemExit, match="no dry-run price for bare-first-20261028T050000/4"
    ):
        run(measured)


@pytest.mark.parametrize("bad", [-1, True, 1.5])
def test_a_price_that_is_not_a_whole_number_of_points_is_refused(measured, bad):
    root = measured[0]
    prices = json.loads((root / "pricing.json").read_text())
    prices["requests"]["bare-first-20261028T050000/4"] = bad
    (root / "pricing.json").write_text(json.dumps(prices))
    with pytest.raises(SystemExit, match="is not a whole number of points"):
        run(measured)


@pytest.mark.parametrize(
    ("field", "value", "why"),
    [
        ("end", 0.1, "an end before its start"),
        ("start", "soon", "not a finite number"),
        ("kind", "carrier-pigeon", "no known kind"),
        ("mutations", -2, "not a whole number"),
        ("ms", -5.0, "without its sequence, duration or limits"),
        ("seq", "first", "without its sequence, duration or limits"),
        ("limits", None, "without its sequence, duration or limits"),
        ("host", 7, "without its host or path"),
        ("path", None, "without its host or path"),
    ],
)
def test_a_capture_record_the_sanitizer_could_not_have_written_is_refused_by_line(
    measured, field, value, why
):
    path = measured[0] / "bare-changed1-20261028T050000" / "requests.jsonl"
    record = json.loads(path.read_text().splitlines()[0]) | {field: value}
    path.write_text(json.dumps(record) + "\n")
    with pytest.raises(SystemExit, match=f"requests.jsonl:1: .*{why}"):
        run(measured)


def test_a_missing_scenario_stops_the_analysis_naming_it(measured):
    shutil.rmtree(measured[0] / "host-approve-20261028T050000")
    with pytest.raises(SystemExit, match="no capture of host-approve"):
        run(measured)


def test_the_background_and_residual_come_from_the_counter_both_idle_samples_read(
    measured,
):
    # The other counter is read before the row only, so it is never the one compared.
    retake(
        measured,
        "bare-changed1",
        {1: 1},
        requests=[graphql(1, "SUB_ISSUES", 4.0, 5.0)],
        steps=[store_step("project-copy", 3.0, 6.0)],
        burn=(
            [reading(0.0, 100), reading(1.0, 900, OTHER_RESET), reading(2.0, 104)],
            [reading(12.0, 130), reading(14.0, 134)],
        ),
    )
    run(measured, "--write")
    points = summary(measured)["bare-changed1"]["points"]
    assert points["counter"] == RESET
    assert (points["before_used"], points["after_used"], points["delta"]) == (
        104,
        130,
        26,
    )
    # 4 + 4 points over 2 + 2 idle seconds: 2 a second, over the 10 s between the two reads.
    assert points["background_rate"] == 2.0
    assert points["background"] == 20.0
    assert points["residual"] == 26 - 1 - 20.0


def test_a_row_whose_idle_samples_share_no_counter_reports_its_priced_figure_alone(
    measured,
):
    retake(
        measured,
        "bare-changed1",
        {1: 1},
        requests=[graphql(1, "SUB_ISSUES", 4.0, 5.0)],
        steps=[store_step("project-copy", 3.0, 6.0)],
        burn=([reading(0.0, 100)], [reading(12.0, 900, OTHER_RESET)]),
    )
    run(measured, "--write")
    points = summary(measured)["bare-changed1"]["points"]
    assert (
        points["figure"] == 1
        and points["counter"] is None
        and points["residual"] is None
    )
    # Each column in its place: no reading, rise, rate or background; the priced points; no residual.
    expected = "| bare store: re-copy, one task's body changed | no counter read on both sides | — | — | — | 1 | — |"
    assert expected in measured[1].read_text()


def test_every_rows_seconds_shares_sum_to_its_measured_total(measured):
    run(measured, "--write")
    rows = summary(measured)
    for row in rows.values():
        assert round(sum(row["attribution"].values()), 2) == round(row["seconds"], 2), (
            row["scenario"]
        )
    first = rows["bare-first"]["attribution"]
    # The gap between the two timed commands is the harness's, not a share of the row.
    assert rows["bare-first"]["seconds"] == 7.0
    assert first["network round trips: GitHub GraphQL"] == 4.0
    assert first["network round trips: GitHub REST and uploads"] == 1.0
    assert first["write screening: onevcs boundary inspect"] == 0.4
    host = rows["host-first"]["attribution"]
    assert host["recipe launch: just, plan-store.sh and uv run"] == 1.0
    assert host["interpreter and SDK start-up"] == 0.5
    assert host["host step: plan_review.review_key"] == 0.5
    assert host["store invocation: project copy"] == 1.5
    assert host["network round trips: GitHub GraphQL"] == 1.0
    assert host["host Python outside the named steps"] == 1.0
    assert host["interpreter exit and recipe teardown"] == 0.5


def test_a_tunnel_a_subprocess_and_a_nested_step_each_take_their_own_share(measured):
    events = host_events() + [
        {
            "kind": "subprocess",
            "program": "git",
            "start": 1.6,
            "end": 1.8,
            "returncode": 0,
            "pid": 7,
        },
        {
            "kind": "step",
            "name": "plan_review.unreviewed",
            "start": 1.4,
            "end": 2.2,
            "pid": 7,
        },
    ]
    retake(
        measured,
        "host-changed1",
        {1: 1},
        requests=[graphql(1, "SUB_ISSUES", 3.0, 4.0), tunnel(2, 4.0, 4.5)],
        steps=[store_step("copy-plan", 0.0, 6.0)],
        host=events,
    )
    run(measured, "--write")
    shares = summary(measured)["host-changed1"]["attribution"]
    assert shares["host subprocess: git"] == 0.2
    # The step that started last is the innermost: review_key inside unreviewed.
    assert shares["host step: plan_review.review_key"] == 0.3
    assert shares["host step: plan_review.unreviewed"] == 0.3
    assert (
        shares[
            "network round trips: tunnelled HTTPS (the image upload's storage and its check)"
        ]
        == 0.5
    )
    assert round(sum(shares.values()), 2) == 6.0


def test_a_request_outside_the_copy_is_the_hosts_and_one_outside_every_invocation_is_named(
    measured,
):
    listing = {
        "kind": "store",
        "argv": ["task", "list", "--project", "P"],
        "start": 1.6,
        "end": 1.9,
        "returncode": 0,
        "pid": 7,
    }
    retake(
        measured,
        "host-unchanged",
        {1: 1, 2: 1, 3: 1},
        requests=[
            graphql(1, "SUB_ISSUES", 3.0, 4.0),
            graphql(2, "ISSUE", 1.7, 1.8),
            graphql(3, "ISSUE", 5.2, 5.3),
        ],
        steps=[store_step("copy-plan", 0.0, 6.0)],
        host=host_events() + [listing],
    )
    run(measured, "--write")
    owners = summary(measured)["host-unchanged"]["owners"]
    assert owners["store | project copy | SUB_ISSUES"]["points"] == 1
    assert owners["host | task list | ISSUE"]["points"] == 1
    assert owners["host | (no store invocation) | ISSUE"]["points"] == 1


def test_walk_pages_are_replaced_by_the_reference_boards_pages(measured):
    retake(
        measured,
        "bare-unchanged",
        {1: 1, 2: 2},
        requests=[graphql(1, "SUB_ISSUES", 1.0, 2.0), graphql(2, "BOARD", 2.0, 3.0)],
        steps=[store_step("project-copy", 0.5, 4.0)],
    )
    run(measured, "--write")
    row = summary(measured)["bare-unchanged"]
    assert row["walk_pages"] == {"BOARD": 1, "SEARCH_ISSUES": 0}
    # 3.5 s measured, less the 1 s page, plus 20 pages of 6.95 s.
    expected = "| bare store: unchanged re-copy | 1 BOARD + 0 SEARCH_ISSUES | **141.5 s** | 3 |"
    assert expected in measured[1].read_text()


def test_a_paused_gate_is_reported_with_its_pauses(measured):
    low = {
        "event": "reading",
        "at": 0.0,
        "buckets": buckets(1000),
        "verdicts": {"core": {"proceed": True}, "graphql": {"proceed": False}},
    }
    pause = {"event": "pause", "at": 0.0, "seconds": 300.0, "buckets": ["graphql"]}
    passed = low | {
        "verdicts": {"core": {"proceed": True}, "graphql": {"proceed": True}}
    }
    retake(
        measured,
        "host-approve",
        {1: 1},
        requests=[graphql(1, "SUB_ISSUES", 3.0, 4.0)],
        steps=[store_step("approve-design", 0.0, 6.0)],
        host=host_events(),
        events=[low, pause, passed],
    )
    run(measured, "--write")
    assert "| proceed (≥ 40%) | 1 pause(s), 300 s |" in measured[1].read_text()


def test_rounded_shares_still_sum_to_the_rounded_total():
    shares = {"a": 1.004, "b": 2.004, "c": 3.004, "d": 0.001}
    out = analyze.rounded(shares, sum(shares.values()))
    assert round(sum(out.values()), 2) == round(sum(shares.values()), 2)
    assert any(key.startswith("1 further causes under 0.005 s each (d)") for key in out)


def test_a_figure_over_its_target_is_marked_over(measured):
    many = [graphql(n, "ISSUE", 3.0, 3.1) for n in range(1, 7)]
    many.append(graphql(7, "UPDATE_ISSUE", 3.2, 3.3, kind="mutation", mutations=1))
    retake(
        measured,
        "host-approve",
        dict.fromkeys(range(1, 7), 1),
        requests=many,
        steps=[store_step("approve-design", 0.0, 6.0)],
        host=host_events(),
    )
    run(measured, "--write")
    row = summary(measured)["host-approve"]
    assert row["points"]["figure"] == 7
    assert row["verdicts"]["points"] == "over"
    assert (
        "Over its target: host `just approve-design`: points."
        in measured[1].read_text()
    )
    assert summary(measured)["host-changed1"]["verdicts"]["points"] == "within"


def test_a_row_within_its_points_target_that_a_five_point_mutation_would_put_over_is_named(
    measured,
):
    # Four queries and one mutation request: 5 points, the approval's target; 9 at five a mutation.
    reads = [graphql(n, "ISSUE", 3.0, 3.1) for n in range(1, 5)]
    reads.append(graphql(5, "UPDATE_ISSUE", 3.2, 3.3, kind="mutation", mutations=1))
    retake(
        measured,
        "host-approve",
        dict.fromkeys(range(1, 5), 1),
        requests=reads,
        steps=[store_step("approve-design", 0.0, 6.0)],
        host=host_events(),
    )
    run(measured, "--write")
    row = summary(measured)["host-approve"]
    assert (row["points"]["figure"], row["points"]["at_five"]) == (5, 9)
    assert row["verdicts"]["points"] == "within"
    text = measured[1].read_text()
    assert "| 9 — **would flip to over** |" in text
    # An unchanged re-copy sends no mutation, so its verdict cannot move.
    assert "| 1 — no flip |" in text


def test_a_gate_whose_last_reading_refused_is_reported_paused(measured):
    low = {
        "event": "reading",
        "at": 0.0,
        "buckets": buckets(1000),
        "verdicts": {"core": {"proceed": True}, "graphql": {"proceed": False}},
    }
    retake(
        measured,
        "host-approve",
        {1: 1},
        requests=[graphql(1, "SUB_ISSUES", 3.0, 4.0)],
        steps=[store_step("approve-design", 0.0, 6.0)],
        host=host_events(),
        events=[low],
    )
    run(measured, "--write")
    text = measured[1].read_text()
    assert "| paused | none |" in text


@pytest.mark.parametrize(
    ("file", "line", "why"),
    [
        ("requests.jsonl", "{not json", "requests.jsonl:1: not JSON"),
        ("requests.jsonl", "[1, 2]", "requests.jsonl:1: not a JSON object"),
        ("steps.jsonl", "", "steps.jsonl: no timed step"),
        (
            "steps.jsonl",
            '{"argv": "copy", "start": 0, "end": 1, "returncode": 0}',
            "without its command line",
        ),
        (
            "screen.jsonl",
            '{"name": "guess", "start": 0, "end": 1}',
            "neither an inspect nor a check",
        ),
        (
            "host.jsonl",
            '{"kind": "store", "start": 0, "end": 1}',
            "store invocation without its command line",
        ),
        (
            "host.jsonl",
            '{"kind": "store", "argv": [1], "start": 0, "end": 1}',
            "command line is not a list of words",
        ),
        (
            "host.jsonl",
            '{"kind": "subprocess", "start": 0, "end": 1}',
            "subprocess without its program",
        ),
        ("host.jsonl", '{"kind": "step", "start": 0, "end": 1}', "without its name"),
        ("host.jsonl", '{"kind": "seance", "start": 0, "end": 1}', "no known kind"),
    ],
)
def test_a_record_file_the_sanitizer_could_not_have_written_is_refused(
    measured, file, line, why
):
    (measured[0] / "host-first-20261028T050000" / file).write_text(line + "\n")
    with pytest.raises(SystemExit, match=why):
        run(measured)


@pytest.mark.parametrize(
    ("file", "value", "why"),
    [
        (
            "quota.json",
            {"quota-before": {"buckets": {}}},
            "does not carry every bucket",
        ),
        ("quota.json", "not an object", "not a JSON object"),
        (
            "conditions.json",
            {"sources.x.plugin": "local-md"},
            "without its value and layer",
        ),
        ("versions.json", {"store": 12}, "a version that is not a string"),
        ("cache-after.json", {"files": -1, "bytes": 0}, "without its files and bytes"),
        ("workload.json", {"project": [10]}, "is not its bytes and characters"),
    ],
)
def test_a_capture_document_the_sanitizer_could_not_have_written_is_refused(
    measured, file, value, why
):
    (measured[0] / "bare-unchanged-20261028T050000" / file).write_text(
        json.dumps(value)
    )
    with pytest.raises(SystemExit, match=why):
        run(measured)


def probe() -> dict:
    return {
        "idle_background": {"readings": [reading(0.0, 10), reading(3.0, 13)]},
        "onevcs_inspect": {
            "inspects": 3,
            "before": [{"core_used": 1, "graphql_used": 5}],
            "after": [{"core_used": 4, "graphql_used": 5}],
        },
    }


def test_the_probes_give_the_background_rate_and_what_one_inspect_spends(measured):
    (measured[0] / "probes").mkdir()
    (measured[0] / "probes" / "background-and-onevcs.json").write_text(
        json.dumps(probe())
    )
    run(measured, "--write")
    text = measured[1].read_text()
    assert (
        f"| idle background, counter resetting {RESET} | 2 reads over 3.0 s | 3 points | 1.00 points/s |"
        in text
    )
    assert "| 1 REST request and 0 GraphQL points per inspect |" in text


@pytest.mark.parametrize(
    ("change", "why"),
    [
        (lambda p: p.pop("onevcs_inspect"), "without its `before` readings"),
        (
            lambda p: p["onevcs_inspect"].update(inspects=0),
            "positive count of inspects",
        ),
        (
            lambda p: p["idle_background"].update(
                readings=[{"at": "now", "used": 1, "reset": 1}]
            ),
            "without timed readings",
        ),
        (
            lambda p: p["idle_background"]["readings"].reverse(),
            "probe readings out of the order they were taken",
        ),
    ],
)
def test_a_malformed_probe_is_refused(measured, change, why):
    malformed = probe()
    change(malformed)
    (measured[0] / "probes").mkdir()
    (measured[0] / "probes" / "background-and-onevcs.json").write_text(
        json.dumps(malformed)
    )
    with pytest.raises(SystemExit, match=why):
        run(measured)


def test_a_pricing_file_whose_prices_are_not_an_object_is_refused(measured):
    (measured[0] / "pricing.json").write_text(json.dumps({"requests": [1, 2]}))
    with pytest.raises(
        SystemExit, match="`requests` or `mutation_rises` is not an object"
    ):
        run(measured)


def test_a_loopback_recording_whose_operations_are_not_counts_is_refused(measured):
    path = measured[0] / "loopback" / "github-plan-copy-points-unchanged.json"
    bad = {"unscreened": {"requests by operation": {"reading": "one"}, "points": 1}}
    path.write_text(
        json.dumps(
            {"budget": "github-plan-copy-points-unchanged", "detail": json.dumps(bad)}
        )
    )
    with pytest.raises(SystemExit, match="requests by operation that are not counts"):
        run(measured)


def test_the_board_size_pairs_compare_requests_by_operation_at_both_sizes(measured):
    root = measured[0]
    path = root / "loopback" / "github-plan-copy-points-unchanged.json"
    record = json.loads(path.read_text())
    detail = json.loads(record["detail"])
    detail["unscreened, on a board of 20,000 items"] = {
        "requests by operation": {
            "reading a project's tasks": 1,
            "reading a page of the board": 1,
        },
        "points": 3,
    }
    record["detail"] = json.dumps(detail)
    path.write_text(json.dumps(record))
    run(measured, "--write")
    pairs = json.loads((root / "summary.json").read_text())["board_size_pairs"]
    (pair,) = [
        p
        for p in pairs
        if p["recording"] == "unscreened"
        and p["budget"] == "github-plan-copy-points-unchanged"
    ]
    assert pair["same"] is False
    assert pair["walk_pages"] == [0, 1]
    assert "**differ**" in measured[1].read_text()


def test_the_committed_report_matches_the_committed_captures(tmp_path):
    """The reconciliation itself, over what is committed: run on a copy, so the committed
    summary is not rewritten by a test run."""
    root = tmp_path / "captures"
    shutil.copytree(analyze.CAPTURES, root)
    copy = tmp_path / "report.md"
    shutil.copy(analyze.REPORT, copy)
    assert analyze.main(["--captures", str(root), "--report", str(copy)]) == 0
    committed = json.loads((analyze.CAPTURES / "summary.json").read_text())
    assert json.loads((root / "summary.json").read_text()) == committed


def test_the_committed_loopback_recordings_still_carry_every_pair_the_report_compares():
    """The loopback journey's recording keys are its own; if they move, the board-size table
    would quietly shrink, so every budget and configuration is required here."""
    pairs = analyze.size_pairs(analyze.loopback(analyze.CAPTURES))
    assert {(p.budget, p.recording) for p in pairs} == {
        (f"github-plan-copy-points-{scenario}", recording)
        for scenario in ("first", "changed", "unchanged")
        for recording in (pairing.small for pairing in analyze.SIZES)
    }
    assert all(p.same and p.walk_pages == (0, 0) for p in pairs)


def test_mutation_rises_that_are_not_whole_numbers_are_refused(measured):
    path = measured[0] / "pricing.json"
    pricing = json.loads(path.read_text()) | {
        "mutation_rises": {"bare-first-20261028T050000": [1, True]}
    }
    path.write_text(json.dumps(pricing))
    with pytest.raises(
        SystemExit,
        match="mutation rises of bare-first-20261028T050000 are not whole numbers",
    ):
        run(measured)


def test_a_step_whose_exit_status_is_a_boolean_is_refused(measured):
    path = measured[0] / "host-first-20261028T050000" / "steps.jsonl"
    path.write_text(
        '{"argv": ["copy-plan"], "start": 0, "end": 1, "returncode": false}\n'
    )
    with pytest.raises(SystemExit, match="without its command line or exit status"):
        run(measured)


def quota_with(measured: tuple[Path, Path], change) -> None:
    path = measured[0] / "bare-unchanged-20261028T050000" / "quota.json"
    q = json.loads(path.read_text())
    change(q)
    path.write_text(json.dumps(q))


@pytest.mark.parametrize(
    ("change", "why"),
    [
        (lambda q: q.update({"quota-after": None}), "`quota-after` is not an object"),
        (
            lambda q: q["quota-before"]["buckets"]["core"].pop("limit"),
            "bucket core without its limit",
        ),
        (
            lambda q: q["quota-before"]["buckets"]["graphql (rateLimit)"].update(
                readings=["x"]
            ),
            "readings that are not whole",
        ),
        (
            lambda q: q["quota-before"]["buckets"]["graphql"].update(limit=0),
            "a GraphQL limit of 0",
        ),
        (
            lambda q: q.update({"burn-after": [{"at": 1.0, "used": -3, "reset": 1}]}),
            "`burn-after` is not a list of timed",
        ),
        (
            lambda q: q.update({"gate": {"event": "reading"}}),
            "the gate's events are not a list",
        ),
        (
            lambda q: q["gate"].append({"event": "guess"}),
            "neither a reading nor a pause",
        ),
        (
            lambda q: q["gate"].append(
                {"event": "reading", "verdicts": {"graphql": "yes"}}
            ),
            "a gate reading whose verdicts are not yes or no",
        ),
        (
            lambda q: q["gate"].append({"event": "reading", "verdicts": {}}),
            "a gate reading without its verdicts",
        ),
        (
            lambda q: q["gate"].append(
                {"event": "pause", "seconds": 5, "verdicts": [1]}
            ),
            "a gate pause whose verdicts are not yes or no",
        ),
        (
            lambda q: q["burn-before"].reverse(),
            "`burn-before` readings out of the order they were taken",
        ),
        (
            lambda q: q["gate"].append({"event": "pause", "seconds": -5}),
            "a gate pause without its seconds",
        ),
    ],
)
def test_a_quota_capture_the_sanitizer_could_not_have_written_is_refused(
    measured, change, why
):
    quota_with(measured, change)
    with pytest.raises(SystemExit, match=why):
        run(measured)


@pytest.mark.parametrize(
    ("record", "why"),
    [
        (
            {"budget": "github-plan-copy-points-unchanged", "detail": "{not json"},
            "a detail that is not JSON",
        ),
        ({"detail": "{}"}, "without its budget or its detail"),
        (
            {"budget": "github-plan-copy-points-first", "detail": "{}"},
            "a recording of 'github-plan-copy-points-first' filed as 'github-plan-copy-points-unchanged'",
        ),
        (
            {
                "budget": "github-plan-copy-points-unchanged",
                "detail": {"unscreened": {"requests by operation": {}, "points": -1}},
            },
            "points that are not a whole number",
        ),
    ],
)
def test_a_loopback_recording_the_journey_could_not_have_written_is_refused(
    measured, record, why
):
    (measured[0] / "loopback" / "github-plan-copy-points-unchanged.json").write_text(
        json.dumps(record)
    )
    with pytest.raises(SystemExit, match=why):
        run(measured)


@pytest.mark.parametrize(
    "outcome",
    [{"deleted": -1, "only": "all"}, {"deleted": 2, "only": 3}, {"only": "bare"}],
)
def test_a_cleanup_outcome_without_its_count_and_scope_is_refused(measured, outcome):
    root = measured[0]
    take(root, "cleanup-20261028T070000", [], [store_step("cleanup", 0.0, 1.0)])
    (root / "cleanup-20261028T070000" / "cleanup.json").write_text(json.dumps(outcome))
    with pytest.raises(SystemExit, match="without its deleted count and its scope"):
        run(measured)


def test_a_cleanup_is_reported_with_what_it_deleted(measured):
    root = measured[0]
    take(root, "cleanup-20261028T070000", [], [store_step("cleanup", 0.0, 1.0)])
    (root / "cleanup-20261028T070000" / "cleanup.json").write_text(
        json.dumps({"deleted": 10, "only": "bare"})
    )
    run(measured, "--write")
    assert (
        "| `cleanup-20261028T070000` | 0 | 0 | 0 | 0 | 10 (bare) | 0 |"
        in measured[1].read_text()
    )


def test_a_mutation_recorded_without_its_count_is_one_write(measured):
    retake(
        measured,
        "bare-changed1",
        {1: 1},
        requests=[
            graphql(1, "SUB_ISSUES", 1.0, 2.0),
            graphql(2, "UPDATE_ISSUE", 2.0, 2.5, kind="mutation"),
        ],
        steps=[store_step("project-copy", 0.5, 3.0)],
    )
    run(measured, "--write")
    assert summary(measured)["bare-changed1"]["writes"]["writes"] == 1


def test_search_pages_and_board_pages_are_each_replaced_by_their_own_reference_pages(
    measured,
):
    retake(
        measured,
        "bare-unchanged",
        {1: 2, 2: 5},
        requests=[graphql(1, "BOARD", 1.0, 2.0), graphql(2, "SEARCH_ISSUES", 2.0, 4.0)],
        steps=[store_step("project-copy", 0.5, 5.0)],
    )
    run(measured, "--write")
    # 4.5 s measured, less 3 s of pages, plus 20 x 6.95 s and 10 x 6.79 s.
    expected = "| bare store: unchanged re-copy | 1 BOARD + 1 SEARCH_ISSUES | **208.4 s** | 7 |"
    assert expected in measured[1].read_text()


def test_a_first_copy_over_its_writes_target_is_marked_over(measured):
    creates = [
        graphql(n, "CREATE_ISSUES", float(n), n + 0.5, kind="mutation", mutations=11)
        for n in range(1, 5)
    ]
    retake(
        measured,
        "host-first",
        {},
        requests=creates,
        steps=[store_step("copy-plan", 0.0, 6.0)],
        host=host_events(),
    )
    run(measured, "--write")
    row = summary(measured)["host-first"]
    assert row["writes"]["writes"] == 44
    assert row["verdicts"]["writes"] == "over"
    assert (
        "Over its target: host `just copy-plan`: first copy: writes."
        in measured[1].read_text()
    )


@pytest.mark.parametrize(
    ("conditions", "why"),
    [
        (
            {"sources.x.cache.enabled": {"value": "false", "layer": "file"}},
            "holds a value it cannot hold",
        ),
        (
            {"sources.x.visibility": {"value": "secret", "layer": "file"}},
            "holds a value it cannot hold",
        ),
        (
            {"page_size": {"value": "50", "layer": "default"}},
            "is not one this measurement declares",
        ),
    ],
)
def test_a_condition_the_store_could_not_have_reported_is_refused(
    measured, conditions, why
):
    (measured[0] / "bare-unchanged-20261028T050000" / "conditions.json").write_text(
        json.dumps(conditions)
    )
    with pytest.raises(SystemExit, match=why):
        run(measured)


def test_a_capture_without_its_request_log_is_refused(measured):
    (measured[0] / "bare-unchanged-20261028T050000" / "requests.jsonl").unlink()
    with pytest.raises(SystemExit, match="no request log"):
        run(measured)


def test_a_capture_document_that_is_not_json_is_refused(measured):
    (measured[0] / "bare-unchanged-20261028T050000" / "versions.json").write_text(
        "{not json"
    )
    with pytest.raises(SystemExit, match="versions.json: not JSON"):
        run(measured)


def test_a_counter_read_once_on_each_side_gives_no_background_rate(measured):
    retake(
        measured,
        "bare-changed1",
        {1: 1},
        requests=[graphql(1, "SUB_ISSUES", 4.0, 5.0)],
        steps=[store_step("project-copy", 3.0, 6.0)],
        burn=([reading(2.0, 100)], [reading(12.0, 104)]),
    )
    run(measured, "--write")
    points = summary(measured)["bare-changed1"]["points"]
    assert (
        points["delta"],
        points["background_rate"],
        points["background"],
        points["residual"],
    ) == (4, 0.0, 0.0, 3.0)


def test_host_events_without_an_interpreter_start_or_exit_time_from_the_step_itself(
    measured,
):
    events = [e for e in host_events() if e["kind"] != "interpreter"]
    retake(
        measured,
        "host-unchanged",
        {1: 1},
        requests=[graphql(1, "SUB_ISSUES", 3.0, 4.0)],
        steps=[store_step("copy-plan", 0.0, 6.0)],
        host=events,
    )
    run(measured, "--write")
    shares = summary(measured)["host-unchanged"]["attribution"]
    # With no interpreter events the recipe's own launch and exit are unseen: everything before the
    # first host event is start-up, everything after the last is the host's own Python.
    assert "recipe launch: just, plan-store.sh and uv run" not in shares
    assert shares["interpreter and SDK start-up"] == 1.5
    assert shares["host Python outside the named steps"] == 1.5
    assert round(sum(shares.values()), 2) == 6.0


def analyzer(*args: str) -> subprocess.CompletedProcess:
    """The analyzer run as the report says to run it: its own executable, a fresh interpreter."""
    return subprocess.run(
        [sys.executable, str(HERE / "analyze.py"), *args],
        capture_output=True,
        text=True,
        check=False,
    )


def test_the_executable_reconciles_quietly_and_exits_one_on_drift(measured):
    root, path = measured
    stale = analyzer("--captures", str(root), "--report", str(path))
    assert stale.returncode == 1 and stale.stdout == ""
    assert "differs from its captures in: targets" in stale.stderr
    written = analyzer("--captures", str(root), "--report", str(path), "--write")
    assert (written.returncode, written.stdout, written.stderr) == (0, "", "")
    checked = analyzer("--captures", str(root), "--report", str(path))
    assert (checked.returncode, checked.stdout, checked.stderr) == (0, "", "")


def test_the_executable_refuses_an_unknown_argument_with_its_usage(measured):
    refused = analyzer("--captures", str(measured[0]), "--rewrite")
    assert refused.returncode == 2
    assert (
        "usage: analyze.py" in refused.stderr
        and "unrecognized arguments: --rewrite" in refused.stderr
    )


def test_the_executable_names_a_damaged_capture_and_exits_non_zero(measured):
    root, path = measured
    (root / "bare-unchanged-20261028T050000" / "requests.jsonl").write_text(
        "{not json\n"
    )
    damaged = analyzer("--captures", str(root), "--report", str(path))
    assert damaged.returncode == 1
    assert (
        "requests.jsonl:1: not JSON" in damaged.stderr
        and "take the capture again" in damaged.stderr
    )


@pytest.mark.parametrize(
    ("drop", "why"),
    [
        ("screened, on a board of 20,000 items", "has no 'screened' on both boards"),
        (
            "host configuration, public source",
            "has no 'host configuration, public source' on both",
        ),
    ],
)
def test_a_points_recording_missing_a_configuration_is_refused(measured, drop, why):
    path = measured[0] / "loopback" / "github-plan-copy-points-unchanged.json"
    record = json.loads(path.read_text())
    detail = json.loads(record["detail"])
    del detail[drop]
    path.write_text(json.dumps(record | {"detail": json.dumps(detail)}))
    with pytest.raises(SystemExit, match=why):
        run(measured)


def test_a_points_recording_whose_configuration_carries_no_requests_is_refused(
    measured,
):
    path = measured[0] / "loopback" / "github-plan-copy-points-unchanged.json"
    record = json.loads(path.read_text())
    detail = json.loads(record["detail"]) | {"screened": {"points": 1}}
    path.write_text(json.dumps(record | {"detail": json.dumps(detail)}))
    with pytest.raises(SystemExit, match="screened: no requests by operation"):
        run(measured)


def test_a_missing_loopback_recording_is_refused_naming_it(measured):
    (measured[0] / "loopback" / "github-plan-copy-points-first.json").unlink()
    with pytest.raises(
        SystemExit, match="no loopback recording github-plan-copy-points-first"
    ):
        run(measured)


def test_overlapping_steps_are_refused(measured):
    retake(
        measured,
        "bare-unchanged",
        {1: 1},
        requests=[graphql(1, "SUB_ISSUES", 1.0, 2.0)],
        steps=[
            store_step("project-copy", 0.5, 3.0),
            store_step("document-copy", 2.0, 4.0),
        ],
    )
    with pytest.raises(
        SystemExit,
        match="steps.jsonl:2: a step that starts before the one before it ended",
    ):
        run(measured)


def test_a_record_of_a_kind_the_sanitizer_wrote_as_other_is_counted_as_what_it_is(
    measured,
):
    # The sanitizer writes any kind it does not know as `other`: such a request is a request but
    # no write, a host event is no cause of its own, and a screening call is timed but neither
    # an inspect nor a check.
    events = host_events() + [
        {"kind": "séance", "name": "x", "start": 1.6, "end": 1.7, "pid": 7}
    ]
    retake(
        measured,
        "host-changed1",
        {1: 1, 2: 1},
        requests=[
            graphql(1, "SUB_ISSUES", 3.0, 4.0),
            graphql(2, "SUB_ISSUES", 4.0, 4.2, kind="carrier"),
        ],
        steps=[store_step("copy-plan", 0.0, 6.0)],
        host=events,
        screen=[
            {
                "kind": "screen",
                "name": "audit",
                "start": 2.6,
                "end": 2.9,
                "returncode": 0,
            }
        ],
    )
    run(measured, "--write")
    row = summary(measured)["host-changed1"]
    assert row["writes"]["requests"] == 2 and row["writes"]["writes"] == 0
    assert row["points"]["figure"] == 2
    assert row["screening"] == {"inspect": 0, "check": 0, "seconds": 0.3}
    assert row["attribution"]["write screening: onevcs boundary other"] == 0.3
    assert not any("séance" in cause for cause in row["attribution"])
    assert round(sum(row["attribution"].values()), 2) == 6.0


@pytest.mark.parametrize("returncode", [1, 4])
def test_a_capture_whose_copy_failed_or_wrote_part_is_no_row(measured, returncode):
    step = store_step("project-copy", 0.5, 3.0) | {"returncode": returncode}
    retake(
        measured,
        "bare-unchanged",
        {1: 1},
        requests=[graphql(1, "SUB_ISSUES", 1.0, 2.0)],
        steps=[step],
    )
    with pytest.raises(
        SystemExit,
        match=f"steps.jsonl:1: a step that exited {returncode}, so this capture is not a row",
    ):
        run(measured)
