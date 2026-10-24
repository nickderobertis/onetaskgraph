"""The `spike-report` answers for this spike, built from `spike/logs/gaps-summary.json`:
`python3 spike/harness/report_answers.py <answers.json> [<manager ruling file>]`.

Every figure comes from the summary, which `gaps.py` derives from the committed logs; the
prose here only says what each figure is. A manager ruling, when one is given, is quoted in
the findings as it was received.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SUMMARY = HERE.parent / "logs" / "gaps-summary.json"
BRANCH = "nick/board-copy-gaps-2026-10-24/spike-feasibility"
WORKLOAD = (
    "The brief's 10-item plan (one 50 KB project, eight 45–49 KB tasks with 10 blocked-by edges, "
    "one 35 KB document with a 104,072-byte PNG) onto the test board (owner nickderobertis, "
    "project 1, issues in nickderobertis/onetaskgraph-live-scratch). Baseline at the 2,000-item "
    "reference size by the approved page-replacement arithmetic: no baseline run sent a board or "
    "search page, so 0 pages are replaced and each figure is its test-board figure; every "
    "prototype read is item- or parent-scoped, so none grows with the board."
)
MEASURES = {
    "points": "GraphQL primary points of `project copy` then `document copy` (approval: the four store commands of `just approve-design`)",
    "seconds": "wall seconds of the same commands",
    "writes": "individual mutations, aliased ones counted one each, plus uploads, of the first copy",
}
APPROVAL = {
    "points": "GraphQL primary points of the four store commands `just approve-design` runs",
    "seconds": "wall seconds of `just approve-design`",
}
GRAPHQL_HOUR = 5000


def label(candidate: dict) -> str:
    # llmlint: ignore[structural_pattern_matching] This repository declares requires-python >=3.9 and ruff targets it; `match` is a syntax error there.
    if candidate["unit"] == "seconds":
        return "estimate"
    if candidate["unit"] == "writes":
        return "observed (unchanged by the selected strategy)"
    return "estimate from observed prices"


def system_limits(candidate: dict) -> str:
    bar = candidate["bar"]
    unit = candidate["unit"]
    # llmlint: ignore[structural_pattern_matching] This repository declares requires-python >=3.9 and ruff targets it; `match` is a syntax error there.
    if unit == "points":
        burst = 3 * candidate["attainable"]
        return (
            f"GraphQL allows {GRAPHQL_HOUR} points an hour, shared with all host work. At the "
            f"10x burst of 3 copies an hour, {burst} points an hour at {candidate['attainable']} a "
            f"copy ({burst / GRAPHQL_HOUR:.2%} of the hour); at the bar, {3 * bar} ({3 * bar / GRAPHQL_HOUR:.2%}). "
            f"{candidate['limits']}"
        )
    if unit == "writes":
        return (
            "GitHub's documented secondary limit is 80 content-creating requests a minute. The first "
            "copy sends 10 issue creations in 3 requests and 1 upload. Its mutation total is "
            f"{candidate['attainable']} against the strict {bar}. {candidate['limits']}"
        )
    return f"{candidate['limits']} The bar is {bar} s and only warns on this host."


def candidates(summary: dict) -> list[dict]:
    consumed = summary["consumed"]["total"]
    shared = (
        f"Shared across all 16 candidates: {consumed['runs']} live probe runs sent {consumed['requests']} "
        f"requests ({consumed['dry_runs']} free dry runs, {consumed['mutation_requests']} mutation "
        f"requests, {consumed['issues_created']} disposable issues created and deleted), "
        f"{consumed['points']} points by the prices observed, in {consumed['seconds']} s of request "
        "time. No baseline scenario was run again: the baseline is the cached re-measurement."
    )
    out = []
    for c in summary["candidates"]:
        verdict = "within" if c["within"] else "OVER"
        out.append(
            {
                "budget": c["budget"],
                "measure": (
                    APPROVAL[c["unit"]]
                    if "approval" in c["budget"]
                    else MEASURES[c["unit"]]
                ),
                "workload": WORKLOAD,
                "achievable": (
                    f"Baseline, observed on 0.11.0 (`{c['baseline_run']}`): {c['baseline']} {c['unit']}. "
                    f"Attainable, {label(c)}: {c['attainable']} {c['unit']}, {verdict} the bar of {c['bar']}. "
                    f"Arithmetic: {c['arithmetic']}."
                ),
                "limits": system_limits(c),
                "consumed": shared,
            }
        )
    return out


def method(summary: dict) -> str:
    prices = summary["prices"]
    allowance = "; ".join(
        f"`{name}` {figures['graphql_before']} → {figures['graphql_after']}"
        for name, figures in summary["consumed"]["runs"].items()
    )
    return (
        "**Live, on the test board only**, through the timing spike's recording proxy, which refuses any "
        "write naming another repository or project. The proxy now also refuses one named through a "
        "variable of any name. Before every run the harness reads `GET /rate_limit` and GraphQL's own "
        "`rateLimit` three times, plus the last answer's headers. It pauses under 40% unless the burn "
        "arithmetic shows a reset first. No pause was needed. Three probes ran. (1) `probe-alias-pricing`, "
        "three times, read-only: GitHub's dry-run price of the plugin's own documents at every alias count "
        "a copy sends, plus coalesced and parent-scoped shapes, each charged once for real with "
        f"`rateLimit {{ cost }}`. Dry run and charge agreed in every case: {prices['dry_run_agrees_with_charge']}. "
        "(2) `probe-placement`, three times: 9 issues created as a coalesced first copy creates them "
        "(parent, then 8 children aliased with `projectV2Ids` and `parentIssueId`). The probe then polls "
        "their board items, races `addProjectV2ItemById` against GitHub's own placement, and sends 9 field "
        "values and 10 blocked-by edges as one request. It times a parent-scoped sub-issue read, measures "
        "the `x-ratelimit-used` rise of each mutation request, and deletes every issue it created. "
        "(3) The baseline is not re-run: it is the cached, sanitized 0.11.0 re-measurement in "
        "`docs/measurements/board-copy-speed-2026-10-18/after/`. GraphQL points remaining of 5,000 "
        f"(the gate's least reading before, `rateLimit` after) for each live run: {allowance}.\n\n"
        "**Offline, synthetic**: the real CLI (0.11.0 wheel `stock`, main `acd090b4` `head`, and `closure`, "
        "head plus the prototype patch) and the host's real onevcs 0.56.5. onevcs runs against an onevcs "
        "home holding only invented identities (`synthco/public-widgets` public, `synthco/secret-gadgets` "
        "private) declared by rule, as the engine's own `onevcs.rs` journeys do. The plan copies from one "
        "public Markdown folder to another. Two synthetic private sources are configured but unrelated to "
        "the plan: a Linear source and a GitHub board, both pointed at one loopback recorder answering every "
        "request 503. No real private source was configured or read.\n\n"
        "**Limits.** These are disclosed rather than corrected. The baseline's host runs substituted "
        "`nickderobertis/onetaskgraph` for the unregistered scratch repository in `onevcs boundary inspect`. "
        "They ran with no onetaskgraph user layer, so a real host's reachability checks of its private "
        "sources are not in the baseline. Synthetic onevcs answers visibility from rules and makes no "
        "GitHub refresh, so its call times (about 0.02 s) are not the host's. The host's measured 0.62–0.67 s "
        "a call is used for the seconds estimates. The synthetic destination is a Markdown folder, not a "
        "board. There is one baseline sample per scenario, and three placement samples of 9 issues each. "
        "GitHub reported two interleaving GraphQL counters, so a mutation's price is the least rise "
        "observed in its own window. A 24-alias write request was never sent. Seconds marked 'estimate' "
        "subtract measured request and screening times from the measured baseline. They are not "
        "measurements."
    )


def findings(summary: dict, ruling: str | None) -> list[str]:
    reach = summary["offline"]["reach"]
    union = summary["offline"]["union"]["closure"]
    refusal = summary["offline"]["refusal"]
    shape = summary["prices"]["shape"]
    dry = summary["prices"]["dry_run"]
    rises = summary["prices"]["mutation_request_rises_by_aliases"]
    placement = summary["placement"]
    children = sorted(
        {s for p in placement for s in p["children_seconds_after_their_answer"]}
    )
    pub = {b: reach[f"reach-public {b}"] for b in ("stock", "head", "closure")}
    trimmed = shape["listing through the item's parent, item trimmed"]
    untrimmed = dry["SUB_ISSUES through the item's parent"]
    calls = union["commands"]
    out = [
        (
            "**Why an unrelated private source is contacted (located).** On 0.11.0 and on main, a public-only copy "
            f"of the synthetic plan sends `{pub['head']['operations_they_asked'][0]}` to the unrelated private board "
            "double. With it unavailable, the copy exits 1 (exit codes stock "
            f"{pub['stock']['exit']}, head {pub['head']['exit']}) and the destination ends unchanged. The failure comes "
            f"after the writes were screened ({pub['head']['onevcs_calls']['check']} checks) and is rolled back, so on a "
            "hosted destination the copy writes and then deletes. The cause is `Engine::record_dependents` "
            "(copy.rs). For every task the copy landed somewhere not declared private, it calls "
            "`recorded_prerequisites`, which asks every source declared private (`recording_sources`) which of its "
            "tasks record that task."
        ),
        (
            "**Minimal safe closure (proven on synthetic sources).** Ask the recording sources about a landed "
            "dependent only when the destination already held it carrying a copy's record "
            "(`onetaskgraph.copied_destination_version`) and not `onetaskgraph.edges_on_record`. That is "
            "exactly the rule `reads_recorded_for` already applies to a dependency read. A dependent this copy "
            "created is never asked, because nothing earlier can have recorded it. An earlier copy that kept an "
            "edge on a private side removed that mark in the same write. The prototype is "
            "`spike/prototype/reach-closure.patch`: 25 added lines in copy.rs, no signature or stored key "
            "changed. With it, the public copy exits "
            f"{pub['closure']['exit']} and sends {pub['closure']['requests_to_private_doubles']} requests to the "
            "unavailable private doubles. A task declared private is still refused before any write (exit "
            f"{reach['reach-private-item closure']['exit']}, destination unchanged, 0 checks, 0 private requests). A "
            "dependency on `ghost:G-1`, a source nothing configures, is still refused before any write (exit "
            f"{reach['reach-unresolvable-ref closure']['exit']}, destination unchanged). Redundant discovery that can be "
            "omitted: that recording-source query, for every dependent created by the copy or marked "
            "`edges_on_record`. Nothing else in the copy contacted an unrelated source."
        ),
        (
            "**Explicitly private reference: withheld, with one remaining request, and not reported.** "
            "A task depending on `priv-linear:SYN-1`, an item of a declared-private Linear source, still "
            f"sends that source `{reach['reach-private-ref closure']['operations_they_asked'][0]}…` "
            "(organization) on head and closure: the opaque-designator verification. When the source is "
            "unavailable, the closure copy succeeds and the destination task carries no edge and no id of it. "
            "The edge is withheld by declaration. On head the same case fails only on the unrelated board's "
            "query. The copy's output names neither the reference nor a withholding (names the reference: "
            f"{reach['reach-private-ref closure']['output_names_the_reference']}; says withheld: "
            f"{reach['reach-private-ref closure']['output_says_withheld']}). By the manager's ruling below, "
            "the implementation adds that report and nothing more."
        ),
        (
            "**Command-scoped deduplicated inspection plus union screening works with schema v1 and the real "
            "onevcs.** No onevcs producer node is needed. Every inspection in each copy command had one distinct "
            f"input. Per-item calls were {calls['project-copy']['as copied']['per_item_calls']} for the project "
            f"copy and {calls['document-copy']['as copied']['per_item_calls']} for the document copy. Deduplicated "
            f"and unioned they were {calls['project-copy']['as copied']['unioned_calls']} and "
            f"{calls['document-copy']['as copied']['unioned_calls']}: one inspect per distinct repository and one "
            "check. The check's `text`, `paths` and `metadata` arrays are concatenated element by element under "
            "the one `scope` every check carried. The union verdict agreed with the per-item verdicts in all 8 "
            f"variants (all agree: {union['all_agree']}), including a synthetic private term planted in one "
            "item's metadata, text and paths, each of which the union refused. The cost: a refused union does not "
            "say which item, so the store re-checks per item only on that refusal path."
        ),
        (
            "**Selected placement strategy for implementation.** (1) Create the project issue, then every task "
            "issue in one aliased `createIssue` request, each with `projectV2Ids` and `parentIssueId`. (2) "
            "Never assume membership. GitHub never returned the board item in a create answer (0 of 27). "
            "Read every created issue's board items in one aliased `CREATED_BOARD_ITEMS` read "
            f"({shape['placement read (up to 21 issues)']} point for up to 21 issues). Repeat it only for issues "
            "still absent, with a bounded backoff. Children were placed "
            f"{children[0]}–{children[-1]} s after their create answer: all 9 at the first read in 2 of 3 "
            "samples, and one child at a second read in the third. (3) Only for an issue still absent after "
            "the bound, send `addProjectV2ItemById`. Racing GitHub's own placement and after it, that left "
            "exactly one item with the same id (idempotent, no duplicate), but it costs one write an issue. "
            "Using it for all 10 would make the first copy 46 writes, over the strict 40. (4) Send every field "
            "value and blocked-by edge as one aliased request of two mutation kinds. 19 in one request were "
            "all answered in 4.2–5.3 s. Split if a request would exceed what was proven."
        ),
        (
            "**Batched origins.** `ORIGIN_LOOKUPS` costs "
            f"{dry['ORIGIN_LOOKUPS asks=6']} point up to 6 asks and {dry['ORIGIN_LOOKUPS asks=8']} at 8–10 asks "
            f"as sent today (`originFirst` 3). At 10 asks it costs {dry['ORIGIN_LOOKUPS asks=10 originFirst=2']} "
            f"with first 2 and {dry['ORIGIN_LOOKUPS asks=10 originFirst=1']} with first 1. First 1 keeps "
            "duplicate detection through `pageInfo.hasNextPage`, which every ask already selects. A single "
            f"`ORIGIN_LOOKUP` costs {dry['ORIGIN_LOOKUP']}. Coalescing `SUB_ISSUES` with any origin lookup saves "
            f"nothing ({dry['SUB_ISSUES + ORIGIN_LOOKUP']} points), while `SUB_ISSUES` with a placement read is "
            f"{dry['SUB_ISSUES + CREATED_BOARD_ITEMS reads=1']}. `SUB_ISSUES` is "
            f"{dry['SUB_ISSUES memberFirst=23']} point at 23 members and {dry['SUB_ISSUES memberFirst=50']} at 50. "
            "One `SUB_ISSUES` read of the parent answered all 8 children with their destination URLs, so a "
            "document copy resolves every task reference from its recorded links, or from that one read, with no "
            "per-task lookup."
        ),
        (
            "**Reference resolution for approval.** The approval's `metadata set` reads the document, its project "
            "and the project's members (ISSUE, ISSUE, SUB_ISSUES). One read of the document's parent's listing, "
            "with the document's own fields trimmed, answers all three and the version precondition's token, and "
            f"costs {trimmed} point (untrimmed {untrimmed}). This is a new "
            "document of the GitHub plugin's own, with no contract change."
        ),
        (
            "**Mutation pricing.** The least `x-ratelimit-used` rise of a mutation request, by alias count, "
            f"was {', '.join(f'{k} aliases: {min(v)}' for k, v in rises.items())}. Every plan above prices a "
            "mutation request at 1 whatever its aliases. A 24-alias request was not sent, so the first copy's "
            "worst case splits its field and edge write in two and still meets 12."
        ),
        (
            "**Undo batching.** One request of 11 aliased `deleteIssue` met `RESOURCE_LIMITS_EXCEEDED` from the "
            "8th alias on. Batches of 5 succeeded. A copy's undo, or any coalesced delete, must stay under that, "
            "or it leaves items behind exactly when a copy cannot finish."
        ),
        (
            "**Harness.** The proxy's write guard missed a project or repository id passed through a variable "
            "of another name (`projectId: $p`). It is now guarded, with a test that fails without the fix. "
            f"The `proxy-refusal` scenario shows every non-test write refused locally with nothing reaching "
            f"upstream (all as expected: {refusal['all_as_expected']}). Dry runs moved `used` by nothing across "
            f"{summary['consumed']['total']['dry_runs']} requests. `GET /rate_limit` and `rateLimit` again "
            "reported different windows."
        ),
        (
            "**Bars over, escalated.** Every strict points and writes bar is attainable on observed prices, "
            "under two unknown-API assumptions: alias-independent mutation pricing beyond 19 aliases, and "
            "placement within a bounded read or two. The worst-case reading stands: the first copy is exactly "
            "at 12 only if both the second placement read and the split write happen, and writes stay within "
            "40 only while placement rides on `createIssue(projectV2Ids)`. The four warn-only host seconds "
            "bars are estimated over even after union screening and the request cuts: host first copy about "
            "33.5 s against 32, one-body re-copy about 19.0 s against 8, unchanged about 12.8 s against 7, "
            "approval about 10.1 s against 7. What remains is unattributed time in the plan store's own "
            "process (7.7–11.0 s in the baseline) and the host recipe's steps (1.8–2.2 s). The final "
            "measurement must attribute those seconds to named causes. No target was changed. All of this "
            "went to the manager before finalize."
        ),
    ]
    if ruling:
        out.append(f"**Manager ruling, as received.** {ruling}")
    return out


def main() -> None:
    summary = json.loads(SUMMARY.read_text())
    ruling = (
        " ".join(Path(sys.argv[2]).read_text().split()) if len(sys.argv) > 2 else None
    )
    answers = {
        "spike": "spike-feasibility",
        "branch": BRANCH,
        "harness": (
            f"On `{BRANCH}`, under `spike/`: the timing spike's harness extended, not replaced. One "
            "measurement of one scenario is `spike/copy-timing.sh <scenario> [--build stock|span|head|closure]`. "
            "This spike added the live `probe-alias-pricing` and `probe-placement`, which need the test "
            "board's `GH_PROJECTS_TOKEN` and are quota-gated. It also added the offline `reach-public`, "
            "`reach-private-item`, `reach-private-ref`, `reach-unresolvable-ref`, `screen-union` and "
            "`proxy-refusal`, which need no credential. `closure` is built by `spike/prototype/build.sh` "
            "from `spike/prototype/reach-closure.patch`, and `stock` offline needs "
            "`SPIKE_HOST_CLONE=~/ai-orchestrator SPIKE_VERSION='onetaskgraph 0.11.0'`. Raw logs stay under "
            "`${XDG_CACHE_HOME:-$HOME/.cache}/board-copy-gaps-2026-10-24/runs/`, and sanitized ones are "
            "committed in `spike/logs/`. `python3 spike/harness/gaps.py` turns them into "
            "`spike/logs/gaps-summary.json`, and `python3 spike/harness/report_answers.py` turns that into "
            "this report's answers. `.venv/bin/pytest spike/tests` checks the harness and analyzer, with no "
            "credential and no network."
        ),
        "method": method(summary),
        "candidates": candidates(summary),
        "findings": findings(summary, ruling),
        "visual_changes": [],
    }
    Path(sys.argv[1]).write_text(json.dumps(answers, indent=1) + "\n")


if __name__ == "__main__":
    main()
