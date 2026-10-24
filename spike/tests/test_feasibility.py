"""The feasibility spike's own checks: the document surgery the pricing probe sends, the union
screening's arithmetic, the synthetic plans' one change per case, the reachability recorder,
and that the committed analysis is what the committed logs add up to.

Run: `.venv/bin/pytest spike/tests` (no credential, no network).
"""

from __future__ import annotations

import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

import pytest

HARNESS = Path(__file__).resolve().parent.parent / "harness"
sys.path.insert(0, str(HARNESS))

import feasibility
import gaps
import reach
import union

DOCS = feasibility.DOCS


def test_a_merged_query_shares_its_variables_and_keeps_each_fragment_once():
    merged = feasibility.merge(DOCS["SUB_ISSUES"], DOCS["ORIGIN_LOOKUPS"])
    defs, body, fragments = feasibility.split(merged)
    names = [d.split(":", 1)[0] for d in defs]
    assert len(names) == len(set(names))
    assert {"$id", "$memberFirst", "$filter0", "$ask9", "$relationshipEvents"} <= set(
        names
    )
    assert "node(id:$id)" in body
    assert "o9:repositoryOwner" in body
    assert merged.count("fragment BoardIssue on Issue") == 1
    assert set(fragments) >= {"BoardIssue", "Related"}


def test_a_variable_declared_two_ways_is_refused():
    with pytest.raises(ValueError, match=r"\$id is declared two ways"):
        feasibility.merge("query($id:ID!){a}", "query($id:String!){b}")


def test_the_parent_scoped_listing_asks_the_parents_members_and_trims_the_item():
    nested = feasibility.through_parent(DOCS["SUB_ISSUES"], trimmed=True)
    _, body, _ = feasibility.split(nested)
    assert body.count("{") == body.count("}")
    assert "node(id:$id){__typename id" in body
    assert "parent{__typename ...BoardIssue blockedBy" in body
    assert "subIssues(first:$memberFirst" in body.split("parent{", 1)[1]


def test_a_union_concatenates_element_by_element_under_one_scope():
    checks = [
        {
            "destination": "public",
            "text": ["a"],
            "paths": [],
            "metadata": ["t1"],
            "scope": None,
        },
        {
            "destination": "public",
            "text": ["b", "c"],
            "paths": ["p"],
            "metadata": ["t2"],
            "scope": None,
        },
    ]
    assert union.union_of(checks) == {
        "destination": "public",
        "text": ["a", "b", "c"],
        "paths": ["p"],
        "metadata": ["t1", "t2"],
    }
    planted = union.planted(checks, "metadata", "o/n")
    assert planted[1]["metadata"] == ["t2 see o/n for details"]
    assert checks[1]["metadata"] == ["t2"]


def test_a_union_of_checks_under_different_scopes_is_refused():
    with pytest.raises(ValueError, match="2 different scopes"):
        union.union_of([{"scope": ["x"]}, {"scope": ["y"]}])


@pytest.mark.parametrize(
    ("case", "expected"),
    [
        ("reach-public", None),
        ("reach-private-item", 'classification: "private"'),
        ("reach-private-ref", '"id": "priv-linear:SYN-1"'),
        ("reach-unresolvable-ref", '"id": "ghost:G-1"'),
    ],
)
def test_each_case_makes_its_one_change_to_a_plan_naming_only_the_synthetic_repository(
    tmp_path, case, expected
):
    reach.write_plan(tmp_path, case)
    texts = {
        p.relative_to(tmp_path).as_posix(): p.read_text()
        for p in tmp_path.rglob("*.md")
    }
    assert len(texts) == 10
    assert all("onetaskgraph-live-scratch" not in t for t in texts.values())
    assert all(
        "github.com/synthco/public-widgets" in t
        for k, t in texts.items()
        if not k.startswith("documents/")
    )
    changed = [k for k, t in texts.items() if expected and expected in t]
    assert changed == ([] if expected is None else [f"tasks/{reach.PLAN}/t1.md"])


def test_the_recorder_answers_503_and_names_what_was_asked():
    recorder = reach.Recorder()
    try:
        request = urllib.request.Request(
            f"{recorder.url}/graphql",
            data=json.dumps(
                {"query": "query($q: String!) { search(query: $q) { nodes { id } } }"}
            ).encode(),
            headers={"Content-Type": "application/json"},
        )
        with pytest.raises(urllib.error.HTTPError) as answered:
            urllib.request.urlopen(request, timeout=10)
        assert answered.value.code == 503
    finally:
        recorder.stop()
    assert [r["operation"] for r in recorder.requests] == ["query <search>"]


def test_a_plans_points_are_the_sum_of_its_priced_shapes():
    shape = {"read": 1, "lookup": 2, "write": 1}
    figure = gaps.plan(
        shape, [("list", "read", 2), ("find", "lookup", 1), ("write", "write", 1)]
    )
    assert figure == {"points": 5, "arithmetic": "list 2×1 + find 1×2 + write 1×1 = 5"}


def test_a_mutations_rise_is_read_against_the_last_request_of_its_own_window(
    tmp_path, monkeypatch
):
    run = tmp_path / "probe-placement-stock-1"
    run.mkdir()
    records = [
        {
            "start": 1,
            "op": "ProbeIds",
            "kind": "query",
            "host": "h",
            "errors": [],
            "limits": {"used": 10, "reset": 5},
        },
        {
            "start": 2,
            "op": "ProbePlaced",
            "kind": "query",
            "host": "h",
            "errors": [],
            "limits": {"used": 500, "reset": 9},
        },
        {
            "start": 3,
            "op": "ProbeNineteen",
            "kind": "mutation",
            "host": "h",
            "errors": [],
            "limits": {"used": 11, "reset": 5},
        },
        {
            "start": 4,
            "op": "ProbeCreateChildren",
            "kind": "mutation",
            "host": "h",
            "errors": [],
            "limits": {"used": 14, "reset": 5},
        },
    ]
    (run / "requests.jsonl").write_text("".join(json.dumps(r) + "\n" for r in records))
    monkeypatch.setattr(gaps, "LOGS", tmp_path)
    assert gaps.mutation_rises() == {19: [1], 8: [3]}


def test_the_committed_analysis_is_what_the_committed_logs_add_up_to(
    tmp_path, monkeypatch
):
    committed = json.loads((gaps.LOGS / "gaps-summary.json").read_text())
    out = tmp_path / "logs"
    out.mkdir()
    for run in gaps.LOGS.iterdir():
        if run.is_dir():
            (out / run.name).symlink_to(run)
    monkeypatch.setattr(gaps, "LOGS", out)
    gaps.main()
    assert json.loads((out / "gaps-summary.json").read_text()) == committed
    by_budget = {c["budget"]: c for c in committed["candidates"]}
    assert len(by_budget) == 16
