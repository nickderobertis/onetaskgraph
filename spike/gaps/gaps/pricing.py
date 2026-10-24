"""`probe-alias-pricing`: what GitHub charges for the plugin's documents.

At every alias count a copy sends, and for the coalesced and parent-scoped shapes a cheaper
copy would send.

Read-only. Each shape is priced by GitHub's own `rateLimit(dryRun: true) { cost }`; each
coalesced shape is then sent once for real, asking `rateLimit { cost }` in the same request, so
the dry-run figure is checked against the charge.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum

from gaps import documents
from gaps.board import ScratchBoard
from gaps.documents import Doc
from gaps.github import Answer, Client, GitHubError, require
from gaps.json_shape import Json, ShapeError, arr, encode, integer, obj, text


class Named(StrEnum):
    """The variants the analysis prices plans with, by the labels the probe records."""

    LOOKUPS_SIX = "ORIGIN_LOOKUPS asks=6"
    LOOKUPS_EIGHT = "ORIGIN_LOOKUPS asks=8"
    LOOKUPS_TODAY = "ORIGIN_LOOKUPS asks=10"
    LOOKUPS_FIRST_1 = "ORIGIN_LOOKUPS asks=10 originFirst=1"
    LOOKUPS_FIRST_2 = "ORIGIN_LOOKUPS asks=10 originFirst=2"
    LOOKUP = "ORIGIN_LOOKUP"
    LISTING = "SUB_ISSUES memberFirst=23"
    LISTING_50 = "SUB_ISSUES memberFirst=50"
    ISSUE = "ISSUE"
    LISTING_PLUS_LOOKUP = "SUB_ISSUES + ORIGIN_LOOKUP"
    LISTING_PLUS_PLACEMENT = "SUB_ISSUES + CREATED_BOARD_ITEMS reads=1"
    PARENT = "SUB_ISSUES through the item's parent"
    PARENT_TRIMMED = "SUB_ISSUES through the item's parent, item trimmed"


@dataclass(frozen=True)
class Shape:
    """One document with the variables it is priced with."""

    variant: str
    document: str
    variables: dict[str, Json]


@dataclass(frozen=True)
class Price:
    """What GitHub said one shape costs."""

    variant: str
    cost: int
    seconds: float


# GitHub's own description of what the spike's schema extension declares: the `RateLimit`
# type's fields and `Query.rateLimit`'s arguments and type, by introspection.
INTROSPECTION = (
    'query SpikeRateLimitSchema { rateLimitType: __type(name: "RateLimit") { fields { name'
    " type { ...TypeRef } } } queryType: __schema { queryType { fields { name type { ...TypeRef }"
    " args { name defaultValue type { ...TypeRef } } } } } }"
    " fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name } } }"
)


@dataclass(frozen=True)
class SchemaField:
    """One field or argument as GitHub's introspection describes it."""

    name: str
    type: str
    default: str | None = None


@dataclass(frozen=True)
class RateLimitSchema:
    """GitHub's own `RateLimit` and `Query.rateLimit`."""

    fields: tuple[SchemaField, ...]
    query_type: str
    arguments: tuple[SchemaField, ...]


@dataclass(frozen=True)
class PricingResult:
    """Every price the probe read, and GitHub's own description of its rate limit.

    Dry-run prices, the charge of every coalesced shape sent for real, and the introspected
    `RateLimit`, absent from runs recorded before the probe asked for it.
    """

    dry_runs: tuple[Price, ...]
    executed: tuple[Price, ...]
    rate_limit_schema: RateLimitSchema | None = None

    def to_json(self) -> Json:
        """The result as `feasibility.json` holds it."""
        return encode(self)


def cost_of(answer: Answer, variant: str) -> int:
    """The cost an answer states, refusing an errored answer or one stating none."""
    if answer.errors:
        raise GitHubError(f"{variant}: {answer.errors[0].type}: {answer.errors[0].message}")
    rate = obj(answer.data.get("rateLimit"), f"{variant}.rateLimit")
    cost = integer(rate.get("cost"), f"{variant}.rateLimit.cost")
    if cost < 0:
        raise ShapeError(f"{variant}: a negative cost")
    return cost


def shapes(board: ScratchBoard, anchor: str) -> tuple[list[Shape], list[Shape]]:
    """The shapes priced by dry run, and the coalesced ones also sent for real.

    `anchor` is any node id: GitHub prices a document from its arguments before it runs.
    """
    docs = documents.snapshot()
    owner, number, repo = board.owner, board.number, board.repository
    dry: list[Shape] = []
    for asks in (1, 2, 3, 4, 5, 6, 8, 10):
        variables = documents.lookups(owner, number, repo, asks)
        dry.append(Shape(f"ORIGIN_LOOKUPS asks={asks}", docs[Doc.ORIGIN_LOOKUPS], variables))
    for first in (1, 2):
        variables = documents.lookups(owner, number, repo, 10, first=first)
        dry.append(
            Shape(
                f"ORIGIN_LOOKUPS asks=10 originFirst={first}", docs[Doc.ORIGIN_LOOKUPS], variables
            )
        )
    dry.append(Shape(Named.LOOKUP, docs[Doc.ORIGIN_LOOKUP], documents.lookup(owner, number)))
    for reads in (1, 2, 9, 10, 21):
        variables = documents.placements([anchor], reads)
        dry.append(
            Shape(f"CREATED_BOARD_ITEMS reads={reads}", docs[Doc.CREATED_BOARD_ITEMS], variables)
        )
    for first in (10, 23, 50, 100):
        variables = documents.members(anchor, first)
        dry.append(Shape(f"SUB_ISSUES memberFirst={first}", docs[Doc.SUB_ISSUES], variables))
    dry.append(Shape(Named.ISSUE, docs[Doc.ISSUE], documents.issue(anchor)))
    listing = documents.members(anchor)
    coalesced = [
        Shape(
            "SUB_ISSUES + ORIGIN_LOOKUPS asks=1",
            documents.merge(docs[Doc.SUB_ISSUES], docs[Doc.ORIGIN_LOOKUPS]),
            listing | documents.lookups(owner, number, repo, 1),
        ),
        Shape(
            "SUB_ISSUES + ORIGIN_LOOKUPS asks=10",
            documents.merge(docs[Doc.SUB_ISSUES], docs[Doc.ORIGIN_LOOKUPS]),
            listing | documents.lookups(owner, number, repo, 10),
        ),
        Shape(
            Named.LISTING_PLUS_LOOKUP,
            documents.merge(docs[Doc.SUB_ISSUES], docs[Doc.ORIGIN_LOOKUP]),
            listing | documents.lookup(owner, number),
        ),
        Shape(
            Named.PARENT,
            documents.through_parent(docs[Doc.SUB_ISSUES]),
            listing,
        ),
        Shape(
            Named.PARENT_TRIMMED,
            documents.through_parent(docs[Doc.SUB_ISSUES], trimmed=True),
            listing,
        ),
        Shape(
            Named.LISTING_PLUS_PLACEMENT,
            documents.merge(docs[Doc.SUB_ISSUES], docs[Doc.CREATED_BOARD_ITEMS]),
            listing | documents.placements([anchor], 1),
        ),
    ]
    return dry + coalesced, coalesced


async def probe(client: Client, board: ScratchBoard, anchor: str) -> PricingResult:
    """Price every shape, then charge every coalesced one."""
    priced, coalesced = shapes(board, anchor)
    dry = []
    for shape in priced:
        answer = await client.graphql("dry run", documents.dry_run(shape.document), shape.variables)
        dry.append(Price(shape.variant, cost_of(answer, shape.variant), answer.seconds))
    executed = []
    for shape in coalesced:
        answer = await client.graphql(
            "charged read", documents.charged(shape.document), shape.variables
        )
        executed.append(Price(shape.variant, cost_of(answer, shape.variant), answer.seconds))
    schema = rate_limit_schema(await client.graphql("schema", INTROSPECTION, {}))
    return PricingResult(dry_runs=tuple(dry), executed=tuple(executed), rate_limit_schema=schema)


def type_name(value: Json, where: str) -> str:
    """An introspected type reference written as SDL writes it: `Int!`, `[ID!]`."""
    entry = obj(value, where)
    match text(entry.get("kind"), f"{where}.kind"):
        case "NON_NULL":
            return type_name(entry.get("ofType"), f"{where}.ofType") + "!"
        case "LIST":
            return "[" + type_name(entry.get("ofType"), f"{where}.ofType") + "]"
        case _:
            return text(entry.get("name"), f"{where}.name")


def schema_field(value: Json, where: str) -> SchemaField:
    """One introspected field or argument."""
    entry = obj(value, where)
    default = entry.get("defaultValue")
    return SchemaField(
        name=text(entry.get("name"), f"{where}.name"),
        type=type_name(entry.get("type"), f"{where}.type"),
        default=None if default is None else text(default, f"{where}.defaultValue"),
    )


def rate_limit_schema(answer: Answer) -> RateLimitSchema:
    """GitHub's introspection answer, as the schema extension writes the same things."""
    data = require(answer, "SpikeRateLimitSchema")
    type_fields = arr(obj(data.get("rateLimitType"), "rateLimitType").get("fields"), "fields")
    schema = obj(obj(data.get("queryType"), "queryType").get("queryType"), "queryType")
    query = next(
        obj(f, "field")
        for f in arr(schema.get("fields"), "queryType.fields")
        if isinstance(f, dict) and f.get("name") == "rateLimit"
    )
    fields = sorted((schema_field(f, "RateLimit.field") for f in type_fields), key=lambda f: f.name)
    return RateLimitSchema(
        fields=tuple(fields),
        query_type=type_name(query.get("type"), "rateLimit.type"),
        arguments=tuple(schema_field(a, "rateLimit.arg") for a in arr(query.get("args"), "args")),
    )
