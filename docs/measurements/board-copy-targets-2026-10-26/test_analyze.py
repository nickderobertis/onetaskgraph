# llmlint: ignore-file[new_code_lands_in_a_project] The report's own recomputation and its tests, run by hand beside the measurement record whose command the report names; docs/measurements is no Nx project, and making it one is a change to the project graph and its four guarded selections, outside this measurement.
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
    (loopback / "github-plan-copy-points-unchanged.json").write_text(
        json.dumps(
            {
                "budget": "github-plan-copy-points-unchanged",
                "value": 1,
                "detail": json.dumps(
                    {"unscreened": sent, "unscreened, on a board of 20,000 items": sent}
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
    shutil.copytree(
        root / "bare-unchanged-20261028T050000",
        root / "superseded" / "bare-unchanged-20261028T070000",
    )
    later = retake(
        measured,
        "bare-unchanged",
        {1: 3},
        requests=[graphql(1, "SUB_ISSUES", 1.0, 2.0)],
        steps=[store_step("project-copy", 0.5, 3.0)],
    )
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
    expected = "| bare store: re-copy, one task's body changed | no counter read on both sides |"
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
            "store invocation without its",
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
    (pair,) = json.loads((root / "summary.json").read_text())["board_size_pairs"]
    assert pair["same"] is False
    assert pair["walk_pages"] == 1
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
        for recording, _ in analyze.SIZES
    }
    assert all(p.same and p.walk_pages == 0 for p in pairs)
