"""The analysis and the report's answers, from the committed logs and the plan's own budgets.

The plan is read through the real CLI configured in the host's ai-orchestrator checkout,
where the plan project lives; the committed summary must be exactly what the committed logs
and those budgets add up to, so a log, a budget or a price that changes is caught here.
"""

from __future__ import annotations

import asyncio
import dataclasses
import json
import os
import re
import shutil
from pathlib import Path

import pytest

from gaps import analysis, budgets, report
from gaps.budgets import Unit
from gaps.json_shape import encode
from tests.conftest import PYTHON, gaps_env, spawn

PLAN_CHECKOUT = Path.home() / "ai-orchestrator"
ONETASKGRAPH = PLAN_CHECKOUT / ".venv/bin/onetaskgraph"


def test_the_plan_states_sixteen_budgets_each_of_a_measured_unit_and_operation() -> None:
    """The plan states sixteen budgets each of a measured unit and operation."""
    found = asyncio.run(budgets.read(ONETASKGRAPH, PLAN_CHECKOUT))
    assert len(found) == 16
    assert {b.unit for b in found} == set(Unit)
    assert {b.operation for b in found} == set(analysis.BASELINE_RUNS)


def test_the_committed_summary_is_what_the_committed_logs_and_the_plan_add_up_to() -> None:
    """The committed summary is what the committed logs and the plan add up to."""
    summary = asyncio.run(report.build(PLAN_CHECKOUT, ONETASKGRAPH))
    committed = json.loads((analysis.LOGS / "gaps-summary.json").read_text())
    assert json.loads(json.dumps(encode(summary))) == committed


def test_every_candidate_states_its_baseline_its_figure_and_its_arithmetic() -> None:
    """Every candidate states its baseline its figure and its arithmetic."""
    summary = asyncio.run(report.build(PLAN_CHECKOUT, ONETASKGRAPH))
    lint = report.Lint("run-1", report.LintSummary(56, 50, 1), (report.Rule("r", "fail", ()),))
    answers = report.answers(summary, "a ruling", lint)
    assert len(answers.candidates) == 16
    for row in answers.candidates:
        assert set(row) == {"budget", "measure", "workload", "achievable", "limits", "consumed"}
        assert "Baseline, observed" in row["achievable"] and "Arithmetic:" in row["achievable"]
    assert answers.findings[-1] == "**Manager ruling, as received.** a ruling"
    assert (
        "llmlint run `run-1`" in answers.findings[-2]
        and "Still failing: r." in answers.findings[-2]
    )
    for candidate in summary.candidates:
        if candidate.budget.unit is Unit.POINTS:
            figure = int(candidate.arithmetic.split(" = ")[1].split(";")[0])
            assert figure == candidate.attainable


def test_the_report_command_writes_the_answers_and_refuses_an_unreadable_lint_record(
    tmp_path: Path,
) -> None:
    """The report command writes the answers and refuses an unreadable lint record."""
    out = tmp_path / "answers.json"
    command = [PYTHON, "-m", "gaps.report", "--answers", str(out)]
    done = spawn(command, gaps_env(tmp_path))
    assert done.returncode == 0, done.err
    assert len(json.loads(out.read_text())["candidates"]) == 16
    broken = tmp_path / "lint.json"
    broken.write_text('{"id": 1}')
    refused = spawn([*command, "--lint", str(broken)], gaps_env(tmp_path))
    assert refused.returncode == 1 and "lint.json" in refused.err


def last_ms(run: str, op: str, count: int) -> float:
    """The seconds the last `count` requests of `op` took in a baseline run, read raw."""
    path = analysis.BASELINE / run / "requests.jsonl"
    lines = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    return sum([r["ms"] for r in lines if r["op"] == op][-count:]) / 1000


def test_the_seconds_estimates_are_the_baseline_less_what_the_plans_remove() -> None:
    """The seconds estimates are the baseline less what the plans remove, recomputed raw."""
    summary = asyncio.run(report.build(PLAN_CHECKOUT, ONETASKGRAPH))
    by_id = {c.budget.id: c for c in summary.candidates}
    runs = json.loads((analysis.BASELINE / "summary.json").read_text())["runs"]

    bare = "tb-unchanged-span-20261024T095930"
    wall = round(sum(runs[bare]["split"].values()), 1)
    expected = round(wall - round(last_ms(bare, "ORIGIN_LOOKUP", 4), 2), 1)
    assert by_id["github-plan-copy-seconds-unchanged"].attainable == expected

    host = "host-approve-stock-20261024T100947"
    split = runs[host]["split"]
    wall = round(sum(split.values()), 1)
    screening = round(sum(v for k, v in split.items() if "screening" in k), 2)
    reads = round(last_ms(host, "ISSUE", 2) + last_ms(host, "SUB_ISSUES", 1), 2)
    parent = summary.prices.parent_listing_seconds
    means = [
        sum(v for k, v in run["split"].items() if "screening" in k)
        / sum(run["screening_calls"].values())
        for name, run in runs.items()
        if name in analysis.BASELINE_RUNS.values() and run.get("screening_calls")
    ]
    per_call = round(max(means), 2)
    assert summary.onevcs_call_seconds == per_call
    expected = round(wall - screening + 2 * per_call - reads + parent, 1)
    assert by_id["plan-approval-seconds"].attainable == expected


def test_the_consumed_points_count_charges_and_mutations_and_never_a_dry_runs_quote() -> None:
    """The consumed points count what was charged and every mutation, never a dry run's quote."""
    summary = asyncio.run(report.build(PLAN_CHECKOUT, ONETASKGRAPH))
    points = 0
    for run in sorted(analysis.LOGS.glob("probe-*/requests.jsonl")):
        for line in run.read_text().splitlines():
            record = json.loads(line)
            if record["kind"] == "rest" or record["op"] in ("dry run", "rateLimit"):
                continue
            points += record["cost"] if record["cost"] is not None else 1
    assert summary.consumed.points == points


def test_the_report_command_quotes_a_ruling_and_states_a_recorded_lint_run(tmp_path: Path) -> None:
    """The report command quotes a ruling and states a recorded lint run."""
    lint = tmp_path / "lint.json"
    rule = {"name": "a_rule", "outcome": "fail", "violations": []}
    lint.write_text(
        json.dumps(
            {"id": "run-9", "summary": {"total": 3, "passed": 2, "failed": 1}, "rules": [rule]}
        )
    )
    ruling = tmp_path / "ruling.md"
    ruling.write_text("Proceed,\nas planned.\n")
    out = tmp_path / "answers.json"
    command = [
        PYTHON,
        "-m",
        "gaps.report",
        "--answers",
        str(out),
        "--lint",
        str(lint),
        "--ruling",
        str(ruling),
    ]
    done = spawn(command, gaps_env(tmp_path))
    assert done.returncode == 0, done.err
    findings = json.loads(out.read_text())["findings"]
    assert findings[-1] == "**Manager ruling, as received.** Proceed, as planned."
    assert "`run-9`" in findings[-2] and "Still failing: a_rule." in findings[-2]


def test_the_answers_are_the_spike_report_templates_own_variables(tmp_path: Path) -> None:
    """The answers are the spike-report template's own variables, read from the template."""
    resolved = spawn(
        [PLAN_CHECKOUT / ".venv/bin/onepipeline", "template", "resolve", "spike-report", "--json"],
        dict(os.environ),
    )
    assert resolved.returncode == 0, resolved.err
    loader = tmp_path / "loader.json"
    loader.write_bytes(resolved.stdout)
    listed = spawn(
        [ONETASKGRAPH, "template", "variables", "--template-loader", loader, "--json"],
        dict(os.environ),
    )
    assert listed.returncode == 0, listed.err
    variables = {v["name"]: v for v in json.loads(listed.out)["variables"]}
    assert set(variables) == {f.name for f in dataclasses.fields(report.Answers)}
    keys = set(re.findall(r"`(\w+)` \(", variables["candidates"]["description"]))
    summary = asyncio.run(report.build(PLAN_CHECKOUT, ONETASKGRAPH))
    row = report.answers(summary, None, None).candidates[0]
    assert set(row) == keys


def test_the_reported_pauses_and_leftovers_are_what_the_live_runs_recorded() -> None:
    """The reported pauses and leftovers are what the committed live runs recorded."""
    summary = asyncio.run(report.build(PLAN_CHECKOUT, ONETASKGRAPH))
    pauses, recording = 0, 0
    for quota in sorted(analysis.LOGS.glob("probe-*/quota.json")):
        recorded = json.loads(quota.read_text())
        if "before" in recorded and "after" in recorded and "pauses" in recorded:
            recording += 1
            pauses += recorded["pauses"]
    assert (summary.consumed.pauses, summary.consumed.runs_recording_pauses) == (pauses, recording)
    last = sorted(analysis.LOGS.glob("probe-placement-*"))[-1] / "feasibility.json"
    left = json.loads(last.read_text())["cleanup"]["left"]
    assert summary.consumed.issues_left_by_the_last_placement == left
    row = report.answers(summary, None, None).candidates[0]
    assert f"{left} left recorded after the last placement probe" in row["consumed"]


def test_a_dry_run_price_disagreeing_with_the_charge_is_disclosed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A dry-run price disagreeing with what GitHub charged is stated by the analysis."""
    logs = tmp_path / "logs"
    shutil.copytree(analysis.LOGS, logs)
    latest = sorted(logs.glob("probe-alias-pricing-*"))[-1] / "feasibility.json"
    recorded = json.loads(latest.read_text())
    recorded["executed"][0]["cost"] += 1
    latest.write_text(json.dumps(recorded))
    monkeypatch.setattr(analysis, "LOGS", logs)
    summary = asyncio.run(report.build(PLAN_CHECKOUT, ONETASKGRAPH))
    assert not summary.prices.dry_run_agrees_with_charge
    assert "dry run and charge agree: False" in report.method(summary)


def test_a_clean_lint_run_is_reported_as_one_with_nothing_failing() -> None:
    """A lint run with no failing rule is stated as such."""
    summary = asyncio.run(report.build(PLAN_CHECKOUT, ONETASKGRAPH))
    clean = report.Lint("run-0", report.LintSummary(56, 56, 0), ())
    finding = report.answers(summary, None, clean).findings[-1]
    assert "56 rules passed and 0 failed of 56. None failed." in finding
