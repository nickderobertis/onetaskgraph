"""Every GraphQL document this spike sends, validated against GitHub's schema as pinned here.

The authority is the schema subset the GitHub Projects plugin pins and validates its own
documents against (`crates/onetaskgraph-github-projects/tests/fixtures/schema.graphql`),
extended only by `github-schema-extension.graphql` for the rate limit the plugin never asks.
"""

from __future__ import annotations

import graphql
import pytest

from gaps import allowance, analysis, board, documents, placement
from gaps.documents import Doc
from gaps.json_shape import decode, load, obj
from gaps.pricing import PricingResult
from tests.conftest import PACKAGE_ROOT, REPO

PINNED = REPO / "crates/onetaskgraph-github-projects/tests/fixtures/schema.graphql"
EXTENSION = PACKAGE_ROOT / "github-schema-extension.graphql"


# Every validation rule but one. GitHub answers documents whose union members select one field
# under two nullabilities (`Issue.body: String!` beside `DraftIssue.body: String` in the plugin's
# own ORIGIN_LOOKUP(S)) — every probe sent them live and was answered — and the plugin's pinned
# validator does not apply graphql-core's stricter overlap rule either.
RULES = [r for r in graphql.specified_rules if r is not graphql.OverlappingFieldsCanBeMergedRule]


def schema() -> graphql.GraphQLSchema:
    """The pinned subset, extended."""
    return graphql.extend_schema(
        graphql.build_schema(PINNED.read_text()), graphql.parse(EXTENSION.read_text())
    )


def sent() -> dict[str, str]:
    """Every document the probes and the quota reading send, by a name for each."""
    docs = documents.snapshot()
    out = {f"plugin {doc.value}": text for doc, text in docs.items()}
    out |= {f"dry run of {doc.value}": documents.dry_run(text) for doc, text in docs.items()}
    out["charged SUB_ISSUES"] = documents.charged(docs[Doc.SUB_ISSUES])
    out["merged SUB_ISSUES + ORIGIN_LOOKUPS"] = documents.merge(
        docs[Doc.SUB_ISSUES], docs[Doc.ORIGIN_LOOKUPS]
    )
    out["parent-scoped SUB_ISSUES"] = documents.through_parent(docs[Doc.SUB_ISSUES])
    out["trimmed parent-scoped SUB_ISSUES"] = documents.through_parent(
        docs[Doc.SUB_ISSUES], trimmed=True
    )
    out["board ids"] = board.ids_query()
    out["rate limit"] = allowance.RATE_QUERY
    out["create"] = placement.create_document("Children", 8)
    out["placement read"] = placement.placed_document(9)
    out["add"] = placement.add_document()
    out["coalesced write"] = placement.coalesced_document(9, [1, 2, 3, 4, 5, 6, 7, 8])
    out["nineteen field values"] = placement.nineteen_document()
    out["ownership read"] = placement.own_document(3)
    out["delete"] = placement.delete_document(5)
    return out


@pytest.mark.parametrize(("name", "document"), sorted(sent().items()), ids=sorted(sent()))
def test_every_document_the_spike_sends_is_valid_against_the_pinned_schema(
    name: str, document: str
) -> None:
    """Every document the spike sends is valid against the pinned schema."""
    errors = graphql.validate(schema(), graphql.parse(document), RULES)
    assert errors == [], f"{name}: {[e.message for e in errors]}"


def test_the_extension_adds_to_the_pinned_schema_and_redefines_none_of_it() -> None:
    """The extension adds to the pinned schema and redefines none of it."""
    pinned = graphql.build_schema(PINNED.read_text())
    added = graphql.parse(EXTENSION.read_text()).definitions
    declared = [d.name.value for d in added if isinstance(d, graphql.TypeDefinitionNode)]
    assert declared and not set(declared) & set(pinned.type_map)
    extended = [d.name.value for d in added if isinstance(d, graphql.TypeExtensionNode)]
    query = pinned.query_type
    assert query is not None
    query_fields = set(query.fields)
    new_fields = {
        f.name.value
        for d in added
        if isinstance(d, graphql.ObjectTypeExtensionNode)
        for f in d.fields or ()
    }
    assert extended == ["Query"] and not new_fields & query_fields


def test_an_invalid_document_is_caught() -> None:
    """A document naming a field the schema lacks is refused, so the gate can fail."""
    errors = graphql.validate(schema(), graphql.parse('query { user(login: "x") { id } }'), RULES)
    assert errors


def test_the_extension_declares_only_what_github_itself_answered() -> None:
    """The extension declares only rate-limit fields GitHub answered in the committed live runs.

    Every live probe records which fields GitHub's own `rateLimit` answered, and the pricing
    probe's dry runs each read `cost`; the extension may declare nothing beyond those.
    """
    answered: set[str] = set()
    for run in analysis.runs("probe-"):
        quota = run / "quota.json"
        if quota.exists():
            for reading in obj(load(quota), str(quota)).values():
                answered |= set(decode(allowance.Reading, reading, str(quota)).answered_fields)
    pricing = decode(
        PricingResult, load(analysis.latest("probe-alias-pricing-") / "feasibility.json"), "p"
    )
    if pricing.dry_runs:
        answered.add("cost")
    added = graphql.parse(EXTENSION.read_text()).definitions
    rate_limit = next(
        d
        for d in added
        if isinstance(d, graphql.ObjectTypeDefinitionNode) and d.name.value == "RateLimit"
    )
    declared = {f.name.value for f in rate_limit.fields or ()}
    assert declared and declared <= answered, declared - answered
