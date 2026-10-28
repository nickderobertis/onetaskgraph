"""The analyzer's parser and reconciliation, driven through its own command line.

    python3 -m pytest docs/measurements/board-copy-targets-2026-10-26/test_analyze.py

Most cases build a small captures directory in the shape `harness/sanitize.py` writes and run
`analyze.main` over it; the last reconciles the committed report with the committed captures.
"""

from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import analyze  # noqa: E402

RESET = 1_800_000_000
OTHER_RESET = 1_799_999_000


def request(seq: int, op: str, start: float, end: float, kind: str = "query", **extra: object) -> dict:
    graphql = kind in ("query", "mutation")
    return {
        "seq": seq,
        "start": start,
        "end": end,
        "ms": round((end - start) * 1000, 1),
        "method": "POST",
        "host": extra.pop("host", "api.github.com"),
        "path": "/graphql" if graphql else extra.pop("path", "/user-attachments/assets"),
        "kind": kind,
        "op": op,
        "req_bytes": 1,
        "resp_bytes": 1,
        "status": 200,
        "cost": None,
        "errors": [],
        "limits": {"resource": "graphql", "reset": RESET} if graphql else {},
        **extra,
    }


def reading(at: float, used: int, reset: int = RESET) -> dict:
    return {"at": at, "used": used, "remaining": 5000 - used, "reset": reset}


def bucket(used: int) -> dict:
    return {"limit": 5000, "used": used, "remaining": 5000 - used, "reset": RESET}


def quota(before: list[dict], after: list[dict]) -> dict:
    rate_limit = bucket(100) | {"readings": [4900, 4900, 4900], "used_readings": [100, 100, 100]}
    buckets = {
        "core": bucket(10),
        "graphql": bucket(100),
        "graphql (GET /rate_limit)": bucket(90),
        "graphql (rateLimit)": rate_limit,
    }
    return {
        "burn-before": before,
        "burn-after": after,
        "quota-before": {"at": 0.0, "buckets": buckets},
        "quota-after": {"at": 0.0, "buckets": buckets},
        "gate": [{"event": "reading", "at": 0.0, "buckets": buckets, "verdicts": {"core": True, "graphql": True}}],
    }


def write_capture(root: Path, name: str, requests: list[dict], steps: list[dict], host: list[dict] | None = None,
                  screen: list[dict] | None = None, burn: tuple[list[dict], list[dict]] | None = None) -> Path:
    path = root / name
    path.mkdir(parents=True)
    (path / "requests.jsonl").write_text("".join(json.dumps(r) + "\n" for r in requests))
    (path / "steps.jsonl").write_text("".join(json.dumps(s) + "\n" for s in steps))
    if host is not None:
        (path / "host.jsonl").write_text("".join(json.dumps(h) + "\n" for h in host))
    (path / "screen.jsonl").write_text("".join(json.dumps(s) + "\n" for s in screen or []))
    before, after = burn or ([reading(0.0, 100), reading(2.0, 102)], [reading(20.0, 120), reading(22.0, 122)])
    (path / "quota.json").write_text(json.dumps(quota(before, after)))
    (path / "versions.json").write_text(json.dumps({"store": "onetaskgraph 0.12.7"}))
    (path / "conditions.json").write_text(
        json.dumps({"sources.notes.visibility": {"value": "private", "layer": "file"}})
    )
    (path / "workload.json").write_text(json.dumps({"project": [10, 10], "task t1": [5, 5], "document": [7, 7], "image": [3, 3]}))
    (path / "cache-before.json").write_text(json.dumps({"files": 0, "bytes": 0}))
    (path / "cache-after.json").write_text(json.dumps({"files": 2, "bytes": 9}))
    return path


def step(name: str, start: float, end: float) -> dict:
    verb = name.split("-")[0]
    return {"step": name, "program": "onetaskgraph", "argv": [verb, "copy", "notes:<id>"], "start": start, "end": end, "returncode": 0}


def captures(root: Path) -> Path:
    """Seven rows: a bare first copy of two GraphQL queries, an aliased create and an upload;
    every other row one query inside one copy; the host rows timed by the host's audit."""
    prices: dict[str, int] = {}
    for scenario, _, _ in analyze.ROWS:
        name = f"{scenario}-20261028T050000"
        if scenario == "bare-first":
            requests = [
                request(1, "ORIGIN_LOOKUPS", 1.0, 2.0),
                request(2, "CREATE_ISSUES", 2.0, 4.0, kind="mutation", mutations=8),
                request(3, "POST /user-attachments/assets", 4.0, 5.0, kind="rest"),
                request(4, "CREATED_BOARD_ITEMS", 6.0, 7.0),
                request(5, "CREATE_ISSUE", 7.0, 7.5, kind="mutation", host="refused", mutations=1),
            ]
            prices[f"{name}/1"] = 1
            prices[f"{name}/4"] = 2
            write_capture(root, name, requests, [step("project-copy", 0.5, 5.5), step("document-copy", 6.0, 8.0)],
                          screen=[{"kind": "screen", "name": "inspect", "start": 5.0, "end": 5.4, "returncode": 0}])
        elif scenario.startswith("host"):
            requests = [request(1, "SUB_ISSUES", 3.0, 4.0)]
            prices[f"{name}/1"] = 1
            host = [
                {"kind": "interpreter", "name": "start", "start": 1.0, "end": 1.0, "pid": 7},
                {"kind": "step", "name": "plan_review.review_key", "start": 1.5, "end": 2.0, "pid": 7},
                {"kind": "store", "argv": ["project", "copy", "authoring:<id>"], "start": 2.5, "end": 5.0, "returncode": 0, "pid": 7},
                {"kind": "interpreter", "name": "exit", "start": 5.5, "end": 5.5, "pid": 7},
            ]
            write_capture(root, name, requests, [step("copy-plan", 0.0, 6.0)], host=host)
        else:
            requests = [request(1, "SUB_ISSUES", 1.0, 2.0)]
            prices[f"{name}/1"] = 1
            write_capture(root, name, requests, [step("project-copy", 0.5, 3.0)])
    (root / "superseded").mkdir()
    (root / "pricing.json").write_text(json.dumps({"requests": prices, "mutation_rises": {}}))
    loopback = root / "loopback"
    loopback.mkdir()
    sent = {"requests by operation": {"reading a project's tasks": 1}, "points": 1}
    (loopback / "github-plan-copy-points-unchanged.json").write_text(
        json.dumps({"budget": "github-plan-copy-points-unchanged", "value": 1,
                    "detail": json.dumps({"unscreened": sent, "unscreened, on a board of 20,000 items": sent})})
    )
    return root


def report(path: Path) -> Path:
    blocks = "".join(f"<!-- analyze:begin {name} -->\nstale\n<!-- analyze:end {name} -->\n\n" for name in (
        "targets", "over", "detail", "owners", "seconds", "burn", "quota", "conditions", "workload",
        "board-size", "reference-size", "writes", "spend", "probes"))
    path.write_text("# A report\n\n" + blocks)
    return path


@pytest.fixture
def measured(tmp_path: Path) -> tuple[Path, Path]:
    return captures(tmp_path / "captures"), report(tmp_path / "report.md")


def run(measured: tuple[Path, Path], *flags: str) -> int:
    root, path = measured
    return analyze.main(["--captures", str(root), "--report", str(path), *flags])


def summary(measured: tuple[Path, Path]) -> dict:
    return {row["scenario"]: row for row in json.loads((measured[0] / "summary.json").read_text())["rows"]}


def test_a_report_whose_tables_are_stale_is_refused_naming_each_and_rewritten_by_write(measured, capsys):
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


def test_a_report_missing_a_table_is_refused(measured, capsys):
    path = measured[1]
    path.write_text(path.read_text().replace("<!-- analyze:begin burn -->\nstale\n<!-- analyze:end burn -->\n", ""))
    assert run(measured, "--write") == 0
    assert run(measured) == 1
    assert "burn (missing from the report)" in capsys.readouterr().err


def test_the_latest_capture_of_a_scenario_is_its_row_and_a_superseded_one_never_is(measured):
    root = measured[0]
    shutil.copytree(root / "bare-unchanged-20261028T050000", root / "superseded" / "bare-unchanged-20261028T060000")
    later = root / "bare-unchanged-20261028T070000"
    shutil.copytree(root / "bare-unchanged-20261028T050000", later)
    prices = json.loads((root / "pricing.json").read_text())
    prices["requests"][f"{later.name}/1"] = 3
    (root / "pricing.json").write_text(json.dumps(prices))
    run(measured, "--write")
    row = summary(measured)["bare-unchanged"]
    assert row["capture"] == later.name
    assert row["points"]["figure"] == 3


def test_points_are_each_querys_own_price_plus_one_per_mutation_request_never_a_refused_one(measured):
    run(measured, "--write")
    first = summary(measured)["bare-first"]
    # Two queries priced 1 and 2, one mutation request at 1; the refused create is not sent.
    assert first["points"]["figure"] == 4
    assert first["points"]["at_five"] == 8
    assert first["writes"] == {"requests": 4, "issues_created": 8, "mutation_requests": 1, "mutations": 8, "uploads": 1, "writes": 9}
    assert first["refused"] == 1


def test_a_query_without_a_dry_run_price_stops_the_analysis(measured):
    root = measured[0]
    prices = json.loads((root / "pricing.json").read_text())
    del prices["requests"]["bare-first-20261028T050000/4"]
    (root / "pricing.json").write_text(json.dumps(prices))
    with pytest.raises(SystemExit, match="no dry-run price for bare-first-20261028T050000/4"):
        run(measured)


def test_a_missing_scenario_stops_the_analysis_naming_it(measured):
    shutil.rmtree(measured[0] / "host-approve-20261028T050000")
    with pytest.raises(SystemExit, match="no capture of host-approve"):
        run(measured)


def test_the_background_and_residual_come_from_the_counter_both_idle_samples_read(measured):
    root = measured[0]
    path = root / "bare-changed1-20261028T050000"
    data = json.loads((path / "quota.json").read_text())
    # The other counter is read before the row only, so it is never the one compared.
    data["burn-before"] = [reading(0.0, 100), reading(1.0, 900, OTHER_RESET), reading(2.0, 104)]
    data["burn-after"] = [reading(12.0, 130), reading(14.0, 134)]
    (path / "quota.json").write_text(json.dumps(data))
    run(measured, "--write")
    points = summary(measured)["bare-changed1"]["points"]
    assert points["counter"] == RESET
    assert (points["before_used"], points["after_used"], points["delta"]) == (104, 130, 26)
    # 4 + 4 points over 2 + 2 idle seconds: 2 a second, over the 10 s between the two reads.
    assert points["background_rate"] == 2.0
    assert points["background"] == 20.0
    assert points["residual"] == 26 - 1 - 20.0


def test_every_rows_seconds_shares_sum_to_its_measured_total(measured):
    run(measured, "--write")
    rows = summary(measured)
    for row in rows.values():
        assert round(sum(row["attribution"].values()), 2) == round(row["seconds"], 2), row["scenario"]
    first = rows["bare-first"]["attribution"]
    # The gap between the two timed commands is the harness's, not a share of the row.
    assert rows["bare-first"]["seconds"] == 7.0
    assert first["network round trips: GitHub GraphQL"] == 4.0
    assert first["write screening: onevcs boundary inspect"] == 0.4
    host = rows["host-first"]["attribution"]
    assert host["recipe launch: just, plan-store.sh and uv run"] == 1.0
    assert host["interpreter and SDK start-up"] == 0.5
    assert host["host step: plan_review.review_key"] == 0.5
    assert host["store invocation: project copy"] == 1.5
    assert host["network round trips: GitHub GraphQL"] == 1.0
    assert host["interpreter exit and recipe teardown"] == 0.5


def test_rounded_shares_still_sum_to_the_rounded_total():
    shares = {"a": 1.004, "b": 2.004, "c": 3.004, "d": 0.001}
    out = analyze.rounded(shares, sum(shares.values()))
    assert round(sum(out.values()), 2) == round(sum(shares.values()), 2)
    assert any(key.startswith("1 further causes under 0.005 s each (d)") for key in out)


def test_a_figure_over_its_target_is_marked_over_and_a_mutation_price_flip_is_named(measured):
    root = measured[0]
    path = root / "host-approve-20261028T050000"
    many = [request(n, "ISSUE", 3.0, 3.1) for n in range(1, 7)]
    many.append(request(7, "UPDATE_ISSUE", 3.2, 3.3, kind="mutation", mutations=1))
    (path / "requests.jsonl").write_text("".join(json.dumps(r) + "\n" for r in many))
    prices = json.loads((root / "pricing.json").read_text())
    prices["requests"] |= {f"{path.name}/{n}": 1 for n in range(1, 7)}
    (root / "pricing.json").write_text(json.dumps(prices))
    run(measured, "--write")
    row = summary(measured)["host-approve"]
    assert row["points"]["figure"] == 7
    assert row["verdicts"]["points"] == "over"
    text = measured[1].read_text()
    assert "Over its target: host `just approve-design`: points." in text
    unchanged = summary(measured)["host-changed1"]
    assert unchanged["verdicts"]["points"] == "within"


def test_the_board_size_pairs_compare_requests_by_operation_at_both_sizes(measured):
    root = measured[0]
    path = root / "loopback" / "github-plan-copy-points-unchanged.json"
    record = json.loads(path.read_text())
    detail = json.loads(record["detail"])
    detail["unscreened, on a board of 20,000 items"] = {"requests by operation": {"reading a project's tasks": 1, "reading a page of the board": 1}, "points": 3}
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
    assert json.loads((root / "summary.json").read_text()) == json.loads((analyze.CAPTURES / "summary.json").read_text())


def test_an_empty_block_is_filled_by_write(measured):
    path = measured[1]
    path.write_text(path.read_text().replace("<!-- analyze:begin over -->\nstale\n", "<!-- analyze:begin over -->\n"))
    assert run(measured, "--write") == 0
    assert "<!-- analyze:begin over -->\nNo points or writes figure is over its target.\n<!-- analyze:end over -->" in path.read_text()
    assert run(measured) == 0
