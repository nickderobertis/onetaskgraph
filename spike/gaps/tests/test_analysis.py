"""The analysis and the report's answers, from the committed logs and the plan's own budgets.

The plan is read through the real CLI configured in the host's ai-orchestrator checkout,
where the plan project lives; the committed summary must be exactly what the committed logs
and those budgets add up to, so a log, a budget or a price that changes is caught here.
"""

from __future__ import annotations

import asyncio
import json
import subprocess
from pathlib import Path

from gaps import analysis, budgets, report
from gaps.budgets import Unit
from gaps.json_shape import encode
from tests.conftest import PYTHON, gaps_env

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
    done = subprocess.run(
        command, env=gaps_env(tmp_path), capture_output=True, text=True, check=False
    )
    assert done.returncode == 0, done.stderr
    assert len(json.loads(out.read_text())["candidates"]) == 16
    broken = tmp_path / "lint.json"
    broken.write_text('{"id": 1}')
    refused = subprocess.run(
        [*command, "--lint", str(broken)],
        env=gaps_env(tmp_path),
        capture_output=True,
        text=True,
        check=False,
    )
    assert refused.returncode == 1 and "lint.json" in refused.stderr


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
    expected = round(wall - screening + 2 * 0.67 - reads + parent, 1)
    assert by_id["plan-approval-seconds"].attainable == expected
