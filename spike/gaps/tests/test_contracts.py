"""Every restatement this harness makes, held to its one source.

The plugin's documents to the crate, each variable builder to its document's declarations,
the pricing labels the analysis reads to the labels the probe records, the test board to the
live crate, the union's fields to the pinned boundary schema, the baseline runs to the
measurement document, the shim to the real onevcs, and the typed decoder to what it refuses.
"""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path

import graphql
import pytest

from gaps import analysis, board, boundary, documents, guard, pricing, reach, report, union
from gaps.boundary import Verdict
from gaps.budgets import Budget, Operation, Unit, from_tasks
from gaps.documents import Doc
from gaps.json_shape import Json, ShapeError, decode, encode
from gaps.shim import USAGE
from tests.conftest import PYTHON, REPO, gaps_env, spawn

ONEVCS = Path.home() / "ai-orchestrator/.venv/bin/onevcs"


def test_the_checked_in_documents_are_what_the_plugin_crate_compiles() -> None:
    """The checked in documents are what the plugin crate compiles."""
    generated = asyncio.run(documents.generate(REPO))
    assert generated == {doc.value: text for doc, text in documents.snapshot().items()}


@pytest.mark.parametrize(
    ("doc", "variables"),
    [
        (Doc.ORIGIN_LOOKUPS, documents.lookups("o", 1, "o/r", 10)),
        (Doc.ORIGIN_LOOKUP, documents.lookup("o", 1)),
        (Doc.CREATED_BOARD_ITEMS, documents.placements(["I_1"], 1)),
        (Doc.SUB_ISSUES, documents.members("I_1")),
        (Doc.ISSUE, documents.issue("I_1")),
    ],
)
def test_each_variable_builder_supplies_exactly_what_its_document_declares(
    doc: Doc, variables: dict[str, Json]
) -> None:
    """Each variable builder supplies exactly what its document declares."""
    names, required = documents.declared(documents.snapshot()[doc])
    assert set(variables) <= names
    assert required <= set(variables)


def test_every_label_the_analysis_prices_is_one_the_probe_sends() -> None:
    """Every label the analysis prices is one the probe sends."""
    scratch = board.ScratchBoard("o", 1, "o/r")
    priced, coalesced = pricing.shapes(scratch, "I_1")
    labels = {s.variant for s in priced}
    assert {n.value for n in pricing.Named} <= labels
    assert {s.variant for s in coalesced} <= labels


def test_merged_and_parent_scoped_documents_keep_one_declaration_of_each_variable() -> None:
    """Merged and parent scoped documents keep one declaration of each variable."""
    docs = documents.snapshot()
    for document in (
        documents.merge(docs[Doc.SUB_ISSUES], docs[Doc.ORIGIN_LOOKUPS]),
        documents.through_parent(docs[Doc.SUB_ISSUES], trimmed=True),
    ):
        parts = documents.split(document)
        names = [d.split(":", 1)[0] for d in parts.defs]
        assert len(names) == len(set(names))
        assert parts.body.count("{") == parts.body.count("}")
    trimmed = documents.split(documents.through_parent(docs[Doc.SUB_ISSUES], trimmed=True))
    assert "node(id:$id){__typename id" in trimmed.body
    with pytest.raises(ValueError, match=r"\$id is declared two ways"):
        documents.merge("query($id:ID!){a}", "query($id:String!){b}")


def test_the_scratch_board_is_the_one_the_live_crate_declares(tmp_path: Path) -> None:
    """The scratch board is the one the live crate declares."""
    scratch = board.scratch_board()
    source = board.LIVE_CRATE.read_text()
    assert f'"{scratch.owner}"' in source and f'"{scratch.repository}"' in source
    assert f"BOARD_NUMBER: u32 = {scratch.number};" in source
    missing = tmp_path / "lib.rs"
    missing.write_text('pub const BOARD_OWNER: &str = "o";\n')
    with pytest.raises(ShapeError, match="BOARD_NUMBER"):
        board.scratch_board(missing)


def test_the_union_concatenates_exactly_the_pinned_schemas_array_fields() -> None:
    """The union concatenates exactly the pinned schemas array fields."""
    schema = json.loads(boundary.PINNED_SCHEMA.read_text())
    properties = schema["check"]["input"]["properties"]
    assert set(boundary.merged_fields()) == {
        name for name, spec in properties.items() if spec.get("type") == "array"
    }
    a = boundary.CheckInput(
        {"text": ("x",), "paths": (), "metadata": ("t",)}, None, boundary.Visibility.PUBLIC
    )
    b = boundary.CheckInput(
        {"text": ("y",), "paths": ("p",), "metadata": ()}, None, boundary.Visibility.PUBLIC
    )
    merged = boundary.union([a, b])
    assert merged.arrays == {"text": ("x", "y"), "paths": ("p",), "metadata": ("t",)}
    with pytest.raises(ValueError, match="different scopes"):
        boundary.union([a, boundary.CheckInput(b.arrays, ("s",), b.destination)])


def test_every_baseline_run_is_one_the_measurement_document_draws_on() -> None:
    """Every baseline run is one the measurement document draws on."""
    document = analysis.BASELINE_DOC.read_text()
    for run in analysis.BASELINE_RUNS.values():
        assert run in document


@pytest.mark.parametrize(
    ("query", "variables", "destinations", "issues"),
    [
        ("mutation($p: ID!) { a(input: {projectId: $p}) { x } }", {"p": "P1"}, {"P1"}, set()),
        (
            'mutation { a(input: {repositoryId: "R1", projectV2Ids: ["P1", "P2"]}) { x } }',
            {},
            {"R1", "P1", "P2"},
            set(),
        ),
        ("mutation($i: In!) { a(input: $i) { x } }", {"i": {"repositoryId": "R2"}}, {"R2"}, set()),
        ("mutation($p: ID!) { a(input: {projectV2Ids: [$p]}) { x } }", {"p": "P3"}, {"P3"}, set()),
        ('mutation($r: ID = "R9") { a(input: {repositoryId: $r}) { x } }', {}, {"R9"}, set()),
        (
            'mutation { a(input: {parentIssueId: "I1", issueId: "I2"}) { x } }',
            {},
            set(),
            {"I1", "I2"},
        ),
    ],
)
def test_the_guard_reads_every_way_a_destination_or_an_issue_can_be_named(
    query: str, variables: dict[str, Json], destinations: set[str], issues: set[str]
) -> None:
    """The guard reads every way a destination or an issue can be named, defaults included."""
    executed = guard.operation(query, variables, None)
    assert executed.mutation
    assert executed.destinations == destinations and executed.issues == issues


def test_the_guard_reads_a_mutation_hidden_by_a_comment_or_a_string() -> None:
    """A mutation behind a `#` in an earlier string, or a field behind a comment, is read."""
    hidden = (
        'query Q { a(x: "#") { b } } mutation M { createIssue(#\n input: {}) { issue { id } }'
        ' updateIssue(#\n input: {id: "I"}) { issue { id } } }'
    )
    executed = guard.operation(hidden, {}, "M")
    assert executed.mutation
    assert {f for _, f in executed.fields} == {"createIssue", "updateIssue"}
    refusal = guard.Guard(frozenset({"R"})).refusal(
        "POST", "/graphql", json.dumps({"query": hidden, "operationName": "M"}).encode()
    )
    assert refusal == "a mutation this spike's probes never send"
    with pytest.raises(guard.Unreadable, match="2 operations"):
        guard.operation(hidden, {}, None)
    with pytest.raises(guard.Unreadable, match="not a GraphQL document"):
        guard.operation("mutation { a(", {}, None)


def test_the_shim_passes_onevcs_through_and_refuses_a_malformed_invocation(tmp_path: Path) -> None:
    """The shim passes onevcs through and refuses a malformed invocation."""
    home = asyncio.run(
        boundary.make_home(
            tmp_path, ONEVCS, (boundary.Identity("o", "n", boundary.Visibility.PUBLIC),)
        )
    )
    log = tmp_path / "calls.jsonl"
    argv = [PYTHON, "-m", "gaps.shim", str(log), str(ONEVCS), "boundary", "inspect", "--input", "-"]
    env = gaps_env(tmp_path) | home.env()
    done = spawn(argv, env, stdin=b'{"repository": "github.com/o/n"}')
    assert done.returncode == 0 and json.loads(done.stdout) == {"visibility": "public"}
    assert json.loads(log.read_text())["name"] == "inspect"
    bad = spawn([PYTHON, "-m", "gaps.shim", str(log)], env)
    assert bad.returncode == USAGE and b"usage" in bad.stderr


def test_the_decoder_names_the_first_value_that_is_not_its_type() -> None:
    """The decoder names the first value that is not its type."""
    record: dict[str, Json] = {
        "id": "x-first",
        "name": "n",
        "threshold": 3,
        "unit": "points",
        "repository": "r",
    }
    assert decode(Budget, record, "b") == Budget("x-first", "n", 3.0, Unit.POINTS, "r")
    assert decode(Budget, encode(decode(Budget, record, "b")), "b").unit is Unit.POINTS
    with pytest.raises(ShapeError, match=r"b\.threshold: expected a number"):
        decode(Budget, record | {"threshold": "3"}, "b")
    with pytest.raises(ShapeError, match=r"b: missing `unit`"):
        decode(Budget, {k: v for k, v in record.items() if k != "unit"}, "b")
    with pytest.raises(ShapeError, match="not a valid Unit"):
        decode(Budget, record | {"unit": "dollars"}, "b")


@pytest.mark.parametrize(
    ("budget", "operation"),
    [
        ("github-plan-copy-points-first", Operation.BARE_FIRST),
        ("github-plan-copy-content-creations", Operation.BARE_FIRST),
        ("plan-copy-writes-first", Operation.HOST_FIRST),
        ("plan-copy-seconds-changed", Operation.HOST_CHANGED),
        ("github-plan-copy-points-unchanged", Operation.BARE_UNCHANGED),
        ("plan-approval-points", Operation.HOST_APPROVAL),
    ],
)
def test_a_budget_is_held_against_the_operation_its_id_names(
    budget: str, operation: Operation
) -> None:
    """A budget is held against the operation its id names."""
    listing: Json = {
        "items": [
            {
                "item": {
                    "metadata": {
                        "orchestrator.budgets": [
                            {
                                "id": budget,
                                "name": "n",
                                "threshold": 1,
                                "unit": "points",
                                "repository": "r",
                            }
                        ]
                    }
                }
            }
        ]
    }
    [parsed] = from_tasks(listing)
    assert parsed.operation is operation


def test_a_budget_naming_no_measured_operation_is_refused() -> None:
    """A budget naming no measured operation is refused."""
    with pytest.raises(ShapeError, match="names no operation"):
        _ = Budget("plan-copy-spend", "n", 1, Unit.POINTS, "r").operation


def test_the_visibilities_are_the_pinned_schemas_own() -> None:
    """The visibilities are the pinned schema's own."""
    schema = json.loads(boundary.PINNED_SCHEMA.read_text())
    variants = schema["inspect"]["output"]["$defs"]["Visibility"]["oneOf"]
    assert {v.value for v in boundary.Visibility} == {v["const"] for v in variants}


def test_the_lint_record_model_reads_what_llmlint_itself_records() -> None:
    """The lint record model reads what llmlint itself records."""
    done = spawn(["llmlint", "history", "latest", "--format", "json"], dict(os.environ))
    assert done.returncode == 0, done.err
    lint = decode(report.Lint, json.loads(done.stdout), "llmlint history")
    assert lint.summary.total >= lint.summary.passed + lint.summary.failed
    assert all(rule.outcome for rule in lint.rules)


def test_an_unavailable_check_stays_unavailable_after_the_union() -> None:
    """An unavailable check stays unavailable after the union, and a refusal beside a pass refuses.

    Asserted on the verdict arithmetic rather than end to end: the real onevcs has no input whose
    check is unavailable while the same home's inspections answer, and a fake onevcs would
    replace the very layer the union experiment exists to exercise.
    """
    verdicts = [Verdict.PASS, Verdict.REFUSE, Verdict.UNAVAILABLE]
    assert union.strictest(verdicts) is Verdict.UNAVAILABLE
    assert union.strictest([Verdict.PASS, Verdict.REFUSE]) is Verdict.REFUSE
    assert union.strictest([]) is Verdict.PASS
    way = union.Way(1, 0, 1, Verdict.UNAVAILABLE, (), 0.0)
    disagreeing = union.Variant(union.Plant.NONE, way, union.Way(1, 0, 1, Verdict.PASS, (), 0.0))
    assert not disagreeing.agrees
    screens = reach.Screens(0, 0, 0, 0.0, ())
    copy = reach.ReachResult(reach.Case.PUBLIC, "", "", (), 0, False, 0, False, 0, screens)
    assert not union.UnionResult(copy=copy, commands={}).all_agree


def test_union_agreement_holds_each_repository_to_its_own_answer(tmp_path: Path) -> None:
    """Union agreement holds each repository to its own answer, through the real onevcs."""
    public = boundary.Identity("o", "open", boundary.Visibility.PUBLIC)
    private = boundary.Identity("o", "closed", boundary.Visibility.PRIVATE)
    home = asyncio.run(boundary.make_home(tmp_path, ONEVCS, (public, private)))
    check = boundary.CheckInput(
        {"text": ("x",), "paths": (), "metadata": ()}, None, boundary.Visibility.PUBLIC
    )
    repositories = [public.qualified, private.qualified, public.qualified]
    variant = asyncio.run(union.screen(home, repositories, [check], union.Plant.NONE, "t"))
    assert variant.agrees
    assert dict(variant.unioned.visibilities) == {
        public.qualified: boundary.Visibility.PUBLIC,
        private.qualified: boundary.Visibility.PRIVATE,
    }
    swapped = union.Way(
        variant.unioned.calls,
        variant.unioned.inspect,
        variant.unioned.check,
        variant.unioned.verdict,
        (
            (public.qualified, boundary.Visibility.PRIVATE),
            (private.qualified, boundary.Visibility.PUBLIC),
        ),
        variant.unioned.seconds,
    )
    assert not union.Variant(variant.plant, variant.per_item, swapped).agrees


def _named_type(kind: graphql.GraphQLType) -> graphql.GraphQLNamedType:
    """A type with its list and non-null wrappers taken off."""
    return graphql.get_named_type(kind)


def _id_fields(kind: graphql.GraphQLInputObjectType, allowed: frozenset[str] | None) -> set[str]:
    """Every id-typed field `kind` declares (of `allowed` only), at any depth of its inputs."""
    found: set[str] = set()
    for name, input_field in kind.fields.items():
        if allowed is not None and name not in allowed:
            continue
        inner = _named_type(input_field.type)
        if isinstance(inner, graphql.GraphQLScalarType) and inner.name == "ID":
            found.add(name)
        elif isinstance(inner, graphql.GraphQLInputObjectType):
            found |= _id_fields(inner, None)
    return found


def test_the_guards_input_table_is_the_pinned_schemas_and_every_id_it_allows_is_classed() -> None:
    """The guard's input table is the pinned schema's, and every id it lets through is classed.

    Each allowed mutation exists, each allowed input field is one its input type declares, and
    every id-typed field reachable through them is an owned destination, an owned issue, an
    owned board item, or a board-scoped field and option.
    """
    pinned = graphql.build_schema(
        (REPO / "crates/onetaskgraph-github-projects/tests/fixtures/schema.graphql").read_text()
    )
    mutation = pinned.mutation_type
    assert mutation is not None
    classed = guard.DESTINATION_KEYS | guard.ISSUE_KEYS | guard.ITEM_KEYS | guard.BOARD_SCOPED
    for name, allowed in guard.PROBE_INPUTS.items():
        assert name in mutation.fields, name
        input_type = _named_type(mutation.fields[name].args["input"].type)
        assert isinstance(input_type, graphql.GraphQLInputObjectType)
        assert allowed <= set(input_type.fields), (name, allowed - set(input_type.fields))
        assert _id_fields(input_type, allowed) <= classed, (name, _id_fields(input_type, allowed))
