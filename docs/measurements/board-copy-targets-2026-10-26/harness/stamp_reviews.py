# llmlint: ignore-file[new_code_lands_in_a_project] This is the measurement record beside the captures it produced: run by hand against GitHub through measure.sh, never by a target. docs/measurements is no Nx project (nor is coalesced-members-dry-run/dry_run.py beside it), and making it one would put a live-GitHub harness inside affected selection.
# llmlint: ignore-file[boundary_inputs_validated] Every input here is this harness's own raw log, its own state folder or the plan it generated moments earlier; the committed captures are validated where they are read, by analyze.py, and what leaves the raw logs is held by the sanitizer's allowlist and leak scan.
# llmlint: ignore-file[changed_behavior_has_e2e] Its proof is the live run itself, whose captures are committed and reconciled by test_analyze.py; the GitHub-facing parts can be driven only against GitHub's test board, which measure.sh does, and the sanitizer produces every capture those tests read.
"""Stamp a generated plan with the review records `copy-plan` requires, without a judged turn.

Run with ai-orchestrator's own environment from its checkout, the `authoring` source
pointed at the generated folder: `uv run python stamp_reviews.py authoring:<plan>`. Each
key is the one ai-orchestrator itself computes (`plan_review.review_key` and `plan_key`),
so `copy-plan`'s pre-flight reads the plan as reviewed. Nothing judged it: the plan is
seeded noise, and only the copy's cost is being measured.
"""

from __future__ import annotations

import sys

from orchestrator import plan_review, plan_store


def main() -> None:
    if len(sys.argv) != 2 or not sys.argv[1].startswith("authoring:"):
        sys.exit(
            "stamp_reviews.py: name the generated plan's project, as authoring:<plan>"
        )
    project = sys.argv[1]
    by = next(iter(plan_review.By))
    records = plan_store.read_tasks(project)
    bar = plan_review.bar_fingerprint()
    # Only a task whose recorded review no longer matches its content is stamped again, as a
    # review of that one task would record it; every other record is left byte for byte.
    for task in plan_review.unreviewed(records, bar):
        plan_review.write_record(project, task, plan_review.review_key(task, bar), by)
    records = plan_store.read_tasks(project)
    plan = plan_store.read_plan(project, records)
    key = plan_review.plan_key(
        plan,
        plan_review.plan_bar_fingerprint(),
        plan_review.plan_answers(plan_store.project_record(project)),
        plan_review.owned_by_node(records),
    )
    plan_review.write_plan_record(project, key, by)


if __name__ == "__main__":
    main()
