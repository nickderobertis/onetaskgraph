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
