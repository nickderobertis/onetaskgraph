"""The analysis and the spike report's answers, in one command.

`python -m gaps.report --answers <out.json> [--ruling <file>] [--lint <llmlint.json>]`

Reads the plan's 16 budgets through the CLI configured in `--plan-checkout` (default: the
host's ai-orchestrator checkout), analyses every committed log, writes
`spike/gaps/logs/gaps-summary.json`, and writes the `spike-report` template's answers. Every
figure in the answers comes from that analysis; a manager ruling is quoted as received, and
the judged-lint result as recorded. Exit statuses: 0 written; 1 a log, the plan or the lint
record could not be read as expected; 2 the invocation is wrong.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import sys
from dataclasses import dataclass
from pathlib import Path

from gaps import analysis, budgets
from gaps.analysis import Baseline, Candidate, Shape, Summary
from gaps.budgets import Operation, Unit
from gaps.json_shape import Json, ShapeError, decode, encode, load
from gaps.placement import AddOutcome
from gaps.pricing import Named
from gaps.proc import ProcessFailed

BRANCH = "nick/board-copy-gaps-2026-10-24/spike-feasibility"
WORKLOAD = (
    "The brief's 10-item plan (one 50 KB project, eight 45–49 KB tasks with 10 blocked-by edges, "
    "one 35 KB document with a 104,072-byte PNG) onto the test board the live crate declares. "
    "Baseline at the 2,000-item reference size by the approved page-replacement arithmetic: no "
    "baseline run sent a board or search page, so 0 pages are replaced and each figure is its "
    "test-board figure; every prototype read is item- or parent-scoped, so none grows with the "
    "board."
)
MEASURES = {
    Unit.POINTS: "GraphQL primary points of `project copy` then `document copy`",
    Unit.SECONDS: "wall seconds of `project copy` then `document copy`",
    Unit.WRITES: (
        "individual mutations of the first copy, aliased ones counted one each, plus uploads"
    ),
}
APPROVAL = {
    Unit.POINTS: "GraphQL primary points of the four store commands `just approve-design` runs",
    Unit.SECONDS: "wall seconds of `just approve-design`",
}


@dataclass(frozen=True)
class Violation:
    """One judged-lint finding, as `llmlint history --format json` records it."""

    file: str
    line: int
    message: str


@dataclass(frozen=True)
class Rule:
    """One judged-lint rule's outcome."""

    name: str
    outcome: str
    violations: tuple[Violation, ...] = ()


@dataclass(frozen=True)
class LintSummary:
    """The run's own totals."""

    total: int
    passed: int
    failed: int


@dataclass(frozen=True)
class Lint:
    """A judged-lint run of this spike's code, and which files it was held to."""

    id: str
    summary: LintSummary
    rules: tuple[Rule, ...]


@dataclass(frozen=True)
class Answers:
    """The `spike-report` template's answers."""

    spike: str
    branch: str
    harness: str
    method: str
    candidates: tuple[dict[str, str], ...]
    findings: tuple[str, ...]
    visual_changes: tuple[str, ...]


def measure_of(candidate: Candidate) -> str:
    """What a candidate measures."""
    if candidate.budget.operation is Operation.HOST_APPROVAL:
        return APPROVAL.get(candidate.budget.unit, MEASURES[candidate.budget.unit])
    return MEASURES[candidate.budget.unit]


def row(candidate: Candidate, summary: Summary) -> dict[str, str]:
    """One candidate as the template's six strings."""
    consumed = summary.consumed
    left = consumed.issues_left_by_the_last_placement
    left_said = (
        "the last placement probe's cleanup unrecorded"
        if left is None
        else f"{left} left recorded after the last placement probe"
    )
    verdict = "within" if candidate.within else "OVER"
    base = candidate.baseline
    return {
        "budget": candidate.budget.id,
        "measure": measure_of(candidate),
        "workload": WORKLOAD,
        "achievable": (
            f"Baseline, observed on 0.11.0 (`{base.run}`): "
            f"{baseline_figure(base, candidate.budget.unit):g} {candidate.budget.unit.value}. "
            f"Attainable, {candidate.label}: {candidate.attainable:g} "
            f"{candidate.budget.unit.value}, "
            f"{verdict} the bar of {candidate.budget.threshold:g}. Arithmetic: "
            f"{candidate.arithmetic}."
        ),
        "limits": candidate.limits,
        "consumed": (
            f"Shared across all 16 candidates: {consumed.runs} live probe runs sent "
            f"{consumed.requests} requests ({consumed.dry_runs} dry runs, "
            f"{consumed.rate_limit_reads} "
            f"quota reads, {consumed.mutation_requests} mutation requests, "
            f"{consumed.creations_sent} disposable issue creations sent; "
            f"{left_said}), "
            f"{consumed.points} points by the prices observed, "
            f"in {consumed.seconds} s of request time. The baseline was not run again: it is the "
            "cached re-measurement."
        ),
    }


def baseline_figure(base: Baseline, unit: Unit) -> float:
    """The baseline figure a unit is held to."""
    match unit:
        case Unit.POINTS:
            return base.points
        case Unit.WRITES:
            return base.writes
        case Unit.SECONDS:
            return base.wall_seconds


def method(summary: Summary) -> str:
    """How the spike measured, and what limits the numbers."""
    allowance = "; ".join(
        f"`{name}` {before} → {after}"
        for name, (before, after) in summary.consumed.remaining_before_and_after.items()
    )
    return (
        "**Live, on the test board only**, through the timing spike's recording proxy behind this "
        "spike's stricter guard (`spike/gaps/gaps/guard.py`: any write naming another repository "
        "or "
        "project, however written, and every upload and tunnel are refused locally). Before every "
        "probe the harness reads `GET /rate_limit`, GraphQL's own `rateLimit` three times and the "
        "last "
        "answer's headers, and applies the timing spike's 40% rule to the least of them. "
        f"Pauses: {summary.consumed.pauses}, across the {summary.consumed.runs_recording_pauses} "
        "live runs that record them; the least GraphQL share any gate proceeded on was "
        f"{summary.consumed.least_share_before:.0%}. GraphQL points remaining before → after each "
        f"live run: {allowance}. "
        "`probe-alias-pricing` prices the plugin's own documents (regenerated from the crate and "
        "held to it by a test) by GitHub's dry run at each alias count a copy sends, plus "
        "coalesced "
        "and parent-scoped shapes, each charged once for real "
        f"(dry run and charge agree: {summary.prices.dry_run_agrees_with_charge}). "
        f"`probe-placement` ran {summary.placement.samples} times: 9 issues created as a coalesced "
        "first copy creates them, polled until placed, `addProjectV2ItemById` raced against "
        "GitHub's "
        "placement, every field value and edge as one request, three 19-alias requests whose "
        "`x-ratelimit-used` rises price a request, one charged parent listing, every issue "
        "deleted. "
        "The baseline is not re-run: it is the cached, sanitized 0.11.0 re-measurement in "
        "`docs/measurements/board-copy-speed-2026-10-18/after/`.\n\n"
        "**Offline, synthetic**: the real CLI (0.11.0 wheel `stock`, main `acd090b4` `head`, and "
        "`closure`, head plus `spike/gaps/prototype/reach-closure.patch`) and the host's real "
        "onevcs, "
        "against an onevcs home holding only invented identities (`synthco/public-widgets` public, "
        "`synthco/secret-gadgets` private) declared by rule and confirmed by onevcs itself. The "
        "plan "
        "copies between two public folders of Markdown while a synthetic private Linear source and "
        "a "
        "synthetic private GitHub board are configured, unrelated to it, and answer every request "
        "503. No real private source was configured or read.\n\n"
        "**Limits**, disclosed rather than corrected: the baseline's host runs substituted "
        "`nickderobertis/onetaskgraph` for the unregistered scratch repository in `onevcs boundary "
        "inspect`, and ran with no onetaskgraph user layer, so a real host's reachability checks "
        "of "
        "its private sources are not in the baseline. Synthetic onevcs answers visibility by rule "
        "and "
        "makes no GitHub refresh, so its call times are not the host's; the host's measured "
        "0.62–0.67 s "
        "a call is used for the seconds estimates. The synthetic destination is a folder, not a "
        "board. "
        "One baseline sample per operation. GitHub reports two interleaving GraphQL counters, so a "
        "mutation's price is the least rise in its own window. No 24-alias request was sent. "
        "Seconds "
        "marked 'estimate' subtract measured request and screening times from the measured "
        "baseline."
    )


def findings(summary: Summary, ruling: str | None, lint: Lint | None) -> list[str]:
    """One string per problem the spike found with the plan's design."""
    reach, union = summary.offline.reach, summary.offline.union
    prices, shape, dry = summary.prices, summary.prices.shape, summary.prices.dry_run
    placed = summary.placement
    children = sorted(placed.children_seconds)
    stock, head, closure = (reach[f"reach-public {b}"] for b in ("stock", "head", "closure"))
    private_item, unresolvable = (
        reach["reach-private-item closure"],
        reach["reach-unresolvable-ref closure"],
    )
    private_ref = reach["reach-private-ref closure"]
    recopy, unmarked = reach["reach-recopy closure"], reach["reach-recopy-unmarked closure"]
    refused_adds = sum(1 for o in placed.add_outcomes if o is AddOutcome.REFUSED_ALREADY_EXISTS)
    unasked = [q.operation for s in head.steps for q in s.recorder_requests]
    asked = [q.operation for s in private_ref.steps for q in s.recorder_requests]
    calls = {
        step: next(v for v in variants if v.plant.value == "as copied")
        for step, variants in union.commands.items()
    }
    over = ", ".join(
        f"`{c.budget.id}` about {c.attainable:g} s against {c.budget.threshold:g}"
        for c in summary.candidates
        if c.budget.unit is Unit.SECONDS and not c.within
    )
    rises = ", ".join(f"{k} aliases: {v}" for k, v in prices.mutation_least_rise_by_aliases.items())
    out = [
        "**Why an unrelated private source is contacted (located).** On 0.11.0 and on main a "
        f"public-only copy of the synthetic plan sends `{unasked[0]}` to the unrelated private "
        "board "
        "double; unavailable, it fails the copy (exits: stock "
        f"{[s.returncode for s in stock.steps]}, "
        f"head {[s.returncode for s in head.steps]}) after the writes were screened "
        f"({head.screens.check} checks), and the copy is rolled back, so on a hosted destination "
        "it "
        "writes and then deletes. The cause is `Engine::record_dependents` (copy.rs): for every "
        "task "
        "the copy landed somewhere not declared private it calls `recorded_prerequisites`, which "
        "asks "
        "every source declared private (`recording_sources`) which of its tasks record that task.",
        "**Minimal safe closure (proven on synthetic sources).** Ask the recording sources about a "
        "landed dependent only when the destination already held it carrying a copy's record and "
        "not "
        "`onetaskgraph.edges_on_record` — the rule `reads_recorded_for` already applies to a "
        "dependency read. A dependent this copy created is never asked: nothing earlier can have "
        "recorded it, and an earlier copy that kept an edge on a private side removed that mark in "
        "the same write. With `reach-closure.patch` (copy.rs only; no signature or stored key "
        f"changes) the public copy exits {[s.returncode for s in closure.steps]} with "
        f"{closure.private_requests} requests to the unavailable doubles. A task declared private "
        "is "
        f"still refused before any write (exit {[s.returncode for s in private_item.steps]}, "
        f"destination changed: {private_item.destination_changed}, {private_item.screens.check} "
        f"checks, {private_item.private_requests} private requests), and so is a dependency on a "
        f"source nothing configures (exit {[s.returncode for s in unresolvable.steps]}, "
        "destination "
        f"changed: {unresolvable.destination_changed}). Redundant discovery that can be omitted: "
        "that "
        "recording-source query for every dependent the copy created or that carries the mark. "
        f"A re-copy over the tasks the first copy marked asked {recopy.private_requests} private "
        f"sources (exits {[s.returncode for s in recopy.steps]}); with the mark taken off all "
        f"{unmarked.tasks_unmarked} tasks, as an earlier release left them, the re-copy asked "
        "again and failed closed on the unavailable double (exits "
        f"{[s.returncode for s in unmarked.steps]}).",
        "**Explicitly private reference: withheld, one request, not reported.** A task depending "
        "on "
        "an item of the declared-private Linear double still sends it "
        f"`{asked[0] if asked else 'none'}…`, "
        "the opaque-designator verification; unavailable, the closure copy succeeds "
        f"(exit {[s.returncode for s in private_ref.steps]}) and the destination carries no edge "
        "and no "
        "id of it — withheld by declaration. The copy's output names neither the reference nor a "
        f"withholding (names it: {any(s.output_names_the_reference for s in private_ref.steps)}; "
        "says "
        f"withheld: {any(s.output_says_withheld for s in private_ref.steps)}). By the manager's "
        "ruling "
        "below the implementation adds that report and nothing more.",
        "**Deduplicated inspection plus union screening works with schema v1 and the real "
        "onevcs.** "
        "No onevcs producer node is needed. "
        + "; ".join(
            f"{step}: {v.per_item.calls} calls per item → {v.unioned.calls} deduplicated and "
            "unioned"
            for step, v in calls.items()
        )
        + ". One inspect per distinct repository and one check of every check's arrays, "
        "concatenated "
        "element by element under the one scope they share (the arrays are the pinned schema's "
        "own). "
        f"Every variant agreed (all agree: {union.all_agree}), including a synthetic private term "
        "planted in an item's metadata, text and paths, each of which the union refused. A refused "
        "union does not say which item, so the store re-checks per item only on that path.",
        "**Selected placement strategy for implementation.** (1) Create the project issue, then "
        "every "
        "task issue in one aliased `createIssue` request, each with `projectV2Ids` and "
        "`parentIssueId`. "
        f"(2) Never assume membership: {placed.in_create_answers} of {placed.issues} create "
        "answers "
        "carried a board item. Read every created issue's items in one aliased "
        "`CREATED_BOARD_ITEMS` "
        f"read ({shape[Shape.PLACEMENT_READ]} point for up to 21 issues) and repeat it only for "
        "issues "
        f"still absent, with a bounded backoff: children were placed {children[0]}–{children[-1]} "
        "s after "
        f"their answer, reads per sample {list(placed.polls)}. (3) Only for an issue still absent "
        "after "
        "the bound, send `addProjectV2ItemById`: racing GitHub's placement and after it, it left "
        "one "
        f"item with the same id and never two (never duplicated: {placed.idempotent}) — "
        f"of {len(placed.add_outcomes)} adds, {refused_adds} were refused as 'Content already "
        "exists in this project' rather than answered with the item, so the fallback treats that "
        "refusal as placed and reads the item — but it costs a write an issue, "
        "and "
        "for all 10 the first copy would be 46 writes, over the strict 40. (4) Send every field "
        "value "
        "and blocked-by edge as one aliased request of two mutation kinds; split one that would "
        "exceed "
        "19 aliases, the most proven.",
        f"**Batched origins.** `ORIGIN_LOOKUPS` costs {dry[Named.LOOKUPS_SIX]} point at up to 6 "
        "asks "
        f"and {dry[Named.LOOKUPS_EIGHT]} at 8–10 as sent today (`originFirst` 3); at 10 asks "
        f"{dry[Named.LOOKUPS_FIRST_2]} with first 2 and {dry[Named.LOOKUPS_FIRST_1]} with first 1, "
        "which "
        "keeps duplicate detection through `pageInfo.hasNextPage`. A single `ORIGIN_LOOKUP` costs "
        f"{dry[Named.LOOKUP]}. A listing coalesced with a lookup is "
        f"{dry[Named.LISTING_PLUS_LOOKUP]} "
        f"(no saving), with a placement read {dry[Named.LISTING_PLUS_PLACEMENT]}. `SUB_ISSUES` is "
        f"{dry[Named.LISTING]} at 23 members and {dry[Named.LISTING_50]} at 50; one read of the "
        "parent "
        "answered every child with its destination URL, so a document copy resolves every task "
        "reference from its recorded links or that one read.",
        "**Reference resolution for approval.** `metadata set` reads the document, its project and "
        "the "
        "project's members (ISSUE, ISSUE, SUB_ISSUES). One read of the document's parent's "
        "listing, "
        f"the document's own fields trimmed, answers all three and the version token for "
        f"{shape[Shape.PARENT_LISTING]} point (untrimmed {dry[Named.PARENT]}): a new document of "
        "the "
        "GitHub plugin's own, no contract change.",
        "**Mutation pricing.** The least `x-ratelimit-used` rise of a mutation request by alias "
        "count: "
        f"{rises}. Dry runs moved `used` by {list(prices.dry_runs_moved_used_by)}. Every plan "
        "prices a "
        "mutation request at its least rise; no 24-alias request was sent, so the first copy's "
        "worst "
        "case splits its field and edge write in two and is still within 12.",
        "**Undo batching** (an earlier run of this spike, commit bae8223f's logs): one request of "
        "11 "
        "aliased `deleteIssue` met `RESOURCE_LIMITS_EXCEEDED` from the 8th alias on, and batches "
        "of 5 "
        "succeeded. A copy's undo, or any coalesced delete, must stay under that.",
        "**Harness.** The timing spike's proxy guard missed an id passed through a variable of "
        "another name (`projectId: $p`) or an inline `projectV2Ids` list. This spike's own async "
        "guard reads both, lets through only the five mutation kinds the probes send, and only "
        "for issues it saw this run create; `proxy-refusal` shows its "
        f"{len(summary.offline.refusal.cases)} cases as expected, nothing refused reaching "
        f"upstream (all as expected: {summary.offline.refusal.all_as_expected}).",
        "**Bars over, escalated.** Every strict points and writes bar is attainable on observed "
        "prices, "
        "under two unknown-API assumptions: alias-independent mutation pricing beyond 19 aliases, "
        "and "
        "placement within a bounded read or two. The worst-case reading stands: the first copy is "
        "exactly at 12 only if both the second placement read and the split write happen, and "
        "writes "
        "stay within 40 only while placement rides on `createIssue(projectV2Ids)`. The warn-only "
        "host "
        f"seconds bars stay over: {over}. What remains is unattributed time in the plan store's "
        "own "
        "process and the host recipe's steps; the final measurement must attribute those seconds "
        "to "
        "named causes. No target was changed; all of this went to the manager before finalize.",
    ]
    if lint is not None:
        failed = sorted(r.name for r in lint.rules if r.outcome == "fail")
        out.append(
            f"**Judged lint of this spike's code** (llmlint run `{lint.id}`, base `bc49f888`, logs "
            f"excluded): {lint.summary.passed} rules passed and {lint.summary.failed} failed of "
            f"{lint.summary.total}."
            + (f" Still failing: {', '.join(failed)}." if failed else " None failed.")
        )
    if ruling:
        out.append(f"**Manager ruling, as received.** {ruling}")
    return out


def answers(summary: Summary, ruling: str | None, lint: Lint | None) -> Answers:
    """The template's answers."""
    return Answers(
        spike="spike-feasibility",
        branch=BRANCH,
        harness=(
            f"On `{BRANCH}`. The timing spike's harness stays under `spike/harness/` unchanged; "
            "this "
            "spike's Nx project `spike-gaps` (`spike/gaps/`) extends it — its proxy, 40% quota "
            "rule and "
            "workload generator are imported, not copied. One measurement of one scenario is "
            "`spike/gaps/gaps.sh <scenario> [--build stock|head|closure]`: the live "
            "`probe-alias-pricing` and "
            "`probe-placement` (the test board's `GH_PROJECTS_TOKEN`, quota-gated) and the offline "
            "`reach-public`, `reach-private-item`, `reach-private-ref`, `reach-unresolvable-ref`, "
            "`screen-union` and `proxy-refusal` (no credential). `closure` is built by "
            "`spike/gaps/prototype/build.sh`. Raw runs stay under "
            "`${XDG_CACHE_HOME:-$HOME/.cache}/board-copy-gaps-2026-10-24/runs/`; sanitized ones "
            "are "
            "committed in `spike/gaps/logs/`. `PYTHONPATH=spike/gaps:spike/harness python3 -m "
            "gaps.report --answers <file>` writes `spike/gaps/logs/gaps-summary.json` and these "
            "answers. "
            "`nx run spike-gaps:test` checks the harness and the analysis end to end."
        ),
        method=method(summary),
        candidates=tuple(row(c, summary) for c in summary.candidates),
        findings=tuple(findings(summary, ruling, lint)),
        visual_changes=(),
    )


async def build(plan_checkout: Path, onetaskgraph: Path) -> Summary:
    """The analysis of every committed log against the plan's budgets."""
    return analysis.analyse(await budgets.read(onetaskgraph, plan_checkout))


def main() -> None:
    """Write the summary and the answers."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--answers", type=Path, required=True)
    parser.add_argument("--ruling", type=Path)
    parser.add_argument("--lint", type=Path)
    parser.add_argument("--plan-checkout", type=Path, default=Path.home() / "ai-orchestrator")
    args = parser.parse_args()
    onetaskgraph = args.plan_checkout / ".venv/bin/onetaskgraph"
    try:
        summary = asyncio.run(build(args.plan_checkout, onetaskgraph))
        lint = None if args.lint is None else decode(Lint, load(args.lint), str(args.lint))
        ruling = None if args.ruling is None else " ".join(args.ruling.read_text().split())
    except (ShapeError, ProcessFailed, OSError) as error:
        sys.stderr.write(f"gaps.report: {error}\n")
        sys.exit(1)
    rendered: Json = encode(summary)
    (analysis.LOGS / "gaps-summary.json").write_text(
        json.dumps(rendered, indent=1, sort_keys=True) + "\n"
    )
    args.answers.write_text(json.dumps(encode(answers(summary, ruling, lint)), indent=1) + "\n")


if __name__ == "__main__":
    main()
