"""The spike's analysis: every committed log, and the plan's budgets, into `logs/gaps-summary.json`.

Reads only sanitized logs — the timing re-measurement's baseline under
`docs/measurements/board-copy-speed-2026-10-18/after/` and this spike's own runs under
`spike/gaps/logs/` — each through the type that wrote it, plus the 16 budget records of the
plan project through the real CLI. No figure is typed in: every price is read from a probe,
every plan behind an attainable figure is a sum of priced shapes, and every seconds estimate
is the measured baseline less measured request and screening times.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path

from gaps.allowance import Reading
from gaps.budgets import Budget, Operation, Unit
from gaps.github import Kind, RequestRecord
from gaps.harness import workload
from gaps.json_shape import ShapeError, arr, decode, integer, load, load_lines, obj
from gaps.placement import AddOutcome, PlacementResult
from gaps.pricing import Named, PricingResult
from gaps.reach import Case, ReachResult
from gaps.refusal import RefusalResult
from gaps.union import Plant, UnionResult

PACKAGE_ROOT = Path(__file__).resolve().parents[1]
REPO = PACKAGE_ROOT.parents[1]
LOGS = PACKAGE_ROOT / "logs"
BASELINE = REPO / "docs/measurements/board-copy-speed-2026-10-18/after"
BASELINE_DOC = REPO / "docs/measurements/board-copy-speed-2026-10-18.md"
# GitHub's published limits, as the GitHub Projects plugin pins and gates them.
RATE_LIMITS = REPO / "crates/onetaskgraph-github-projects/tests/fixtures/rate-limits.json"
# The run of each operation the measurement document draws its figures from; a test holds each
# name to that document.
BASELINE_RUNS = {
    Operation.BARE_FIRST: "tb-first-span-20261024T095836",
    Operation.BARE_CHANGED: "tb-changed1-span-20261024T101530",
    Operation.BARE_UNCHANGED: "tb-unchanged-span-20261024T095930",
    Operation.HOST_FIRST: "host-copy-stock-20261024T100745",
    Operation.HOST_CHANGED: "host-changed1-stock-20261024T102149",
    Operation.HOST_UNCHANGED: "host-recopy-stock-20261024T100859",
    Operation.HOST_APPROVAL: "host-approve-stock-20261024T100947",
}
BURST_COPIES_AN_HOUR = 3


class Shape(StrEnum):
    """A request shape a plan is priced in."""

    BATCHED_LOOKUP = "batched origin lookup, 10 asks, first 1"
    SINGLE_LOOKUP = "single origin lookup"
    LISTING = "sub-issue listing (23 members)"
    ITEM_READ = "item read"
    PLACEMENT_READ = "placement read (up to 21 issues)"
    PARENT_LISTING = "listing through the item's parent, item trimmed"
    MUTATION = "mutation request (any alias count observed)"


@dataclass(frozen=True)
class OpFigure:
    """One operation of a baseline run, as the re-measurement's summary states it."""

    count: int
    kind: str
    points: int
    seconds: float


@dataclass(frozen=True)
class BaselineRun:
    """One baseline run's figures, as the re-measurement's summary states them."""

    points: int
    requests: int
    mutation_fields: int
    by_op: dict[str, OpFigure]
    split: dict[str, float]
    screening_calls: dict[str, int]


@dataclass(frozen=True)
class BaselineRequest:
    """One sanitized baseline request: the fields the estimates read."""

    op: str
    ms: float


@dataclass(frozen=True)
class Baseline:
    """One operation's baseline figures."""

    operation: Operation
    run: str
    points: int
    requests: int
    writes: int
    wall_seconds: float
    screening_seconds: float
    screening_calls: int


@dataclass(frozen=True)
class Prices:
    """Every price the probes observed."""

    dry_run: dict[str, int]
    charged: dict[str, int]
    dry_run_agrees_with_charge: bool
    mutation_rises_by_aliases: dict[int, tuple[int, ...]]
    mutation_least_rise_by_aliases: dict[int, int]
    dry_runs_moved_used_by: tuple[int, ...]
    placement_poll_costs: tuple[int, ...]
    shape: dict[Shape, int]
    parent_listing_seconds: float


@dataclass(frozen=True)
class Step:
    """One request kind of a plan: what it is, its priced shape, how many."""

    what: str
    shape: Shape
    count: int


@dataclass(frozen=True)
class Figure:
    """A plan's sum, and the arithmetic that makes it."""

    value: float
    arithmetic: str


@dataclass(frozen=True)
class Candidate:
    """One budget with its baseline, its attainable figure and what limits it."""

    budget: Budget
    baseline: Baseline
    attainable: float
    within: bool
    label: str
    arithmetic: str
    limits: str
    worst: float | None = None


@dataclass(frozen=True)
class Placement:
    """Every placement sample, together."""

    samples: int
    issues: int
    children_seconds: tuple[float, ...]
    polls: tuple[int, ...]
    in_create_answers: int
    idempotent: bool
    add_outcomes: tuple[AddOutcome, ...]


@dataclass(frozen=True)
class Offline:
    """The latest result of every offline case and build."""

    reach: dict[str, ReachResult]
    reach_runs: dict[str, str]
    union: UnionResult
    union_run: str
    refusal: RefusalResult
    refusal_run: str


@dataclass(frozen=True)
class Consumed:
    """What this spike's live runs sent, and what they cost."""

    runs: int
    requests: int
    dry_runs: int
    rate_limit_reads: int
    mutation_requests: int
    creations_sent: int
    points: int
    seconds: float
    remaining_before_and_after: dict[str, tuple[int, int]]
    least_share_before: float
    pauses: int
    runs_recording_pauses: int
    issues_left_by_the_last_placement: int | None


@dataclass(frozen=True)
class Workload:
    """The plan the baseline copied, as its own `workload.json` and the generator record it."""

    project_bytes: int
    tasks: int
    task_bytes_least: int
    task_bytes_most: int
    edges: int
    document_bytes: int


def measured_workload() -> Workload:
    """The bare first copy's rendered sizes, and the generator's edges."""
    path = BASELINE / BASELINE_RUNS[Operation.BARE_FIRST] / "workload.json"
    sizes = {
        label: integer(arr(value, f"{path}.{label}")[0], f"{path}.{label}[0]")
        for label, value in obj(load(path), str(path)).items()
    }
    tasks = [b for label, b in sizes.items() if label.startswith("task ")]
    return Workload(
        project_bytes=sizes["project"],
        tasks=len(tasks),
        task_bytes_least=min(tasks),
        task_bytes_most=max(tasks),
        edges=len(workload.TASK_EDGES),
        document_bytes=sizes["document"],
    )


class Share(StrEnum):
    """Where a host run's wall time went, in the order an overlapping instant is claimed."""

    GITHUB = "GitHub API requests"
    SCREENING = "onevcs screening, its GitHub refresh included"
    TRANSFERS = "the store's tunnelled transfers outside any screen: an asset upload's"
    COPY_PROCESS = "the copy's own store process, outside requests and screening"
    STORE_CALLS = "the host's other store invocations: process start-up, loading, local reads"
    HOST_STEPS = "the host program's own steps between store invocations"
    BETWEEN = "inside the host program, between its recorded steps and invocations"
    RECIPE = "the recipe outside the host program: just, interpreter start-up and exit"


# What the measurement document calls unattributed time in a stock store process.
STORE_PROCESS = frozenset({Share.TRANSFERS, Share.COPY_PROCESS, Share.STORE_CALLS})


@dataclass(frozen=True)
class Span:
    """One timed record of a baseline host run: a request, a screen, a step or a store call."""

    start: float
    end: float
    kind: str = ""


@dataclass(frozen=True)
class HostTime:
    """One baseline host run's wall time, every millisecond given to one `Share`."""

    operation: Operation
    run: str
    wall_seconds: float
    shares: dict[Share, float]
    store_calls: int
    screens_in_copy: int

    @property
    def store_process(self) -> float:
        """The seconds the measurement left unattributed inside a stock store process."""
        return round(sum(v for k, v in self.shares.items() if k in STORE_PROCESS), 2)


def spans(path: Path) -> list[Span]:
    """Every timed record of one JSON-lines log of a baseline run."""
    return [decode(Span, line, f"{path}:{n}") for n, line in enumerate(load_lines(path), 1)]


def attribute(operation: Operation, run: str) -> HostTime:
    """Give each millisecond of one host run to the first `Share` whose records cover it."""
    where = BASELINE / run
    recipe = spans(where / "steps.jsonl")
    host = spans(where / "host.jsonl")
    stores = [h for h in host if h.kind == "store"]
    if len(recipe) != 1 or not stores:
        raise ShapeError(f"{where}: expected one recipe step and at least one store invocation")
    copy = max(stores, key=lambda h: h.end - h.start)
    requests = spans(where / "requests.jsonl")
    screens = spans(where / "screen.jsonl")
    tunnels = [r for r in requests if r.kind == "tunnel"]
    screened = [t for t in tunnels if any(s.start <= t.start and t.end <= s.end for s in screens)]
    covered = {
        Share.GITHUB: [r for r in requests if r.kind != "tunnel"],
        Share.SCREENING: screens + screened,
        Share.TRANSFERS: [t for t in tunnels if t not in screened],
        Share.COPY_PROCESS: [copy],
        Share.STORE_CALLS: [h for h in stores if h is not copy],
        Share.HOST_STEPS: [h for h in host if h.kind == "step"],
        Share.BETWEEN: [Span(min(h.start for h in host), max(h.end for h in host))],
        Share.RECIPE: recipe,
    }
    origin = recipe[0].start
    millis = round((recipe[0].end - origin) * 1000)
    owner: list[Share | None] = [None] * millis
    for share, records in covered.items():
        for record in records:
            first = max(0, round((record.start - origin) * 1000))
            for i in range(first, min(millis, round((record.end - origin) * 1000))):
                if owner[i] is None:
                    owner[i] = share
    return HostTime(
        operation=operation,
        run=run,
        wall_seconds=round(millis / 1000, 1),
        shares={share: round(owner.count(share) / 1000, 2) for share in Share},
        store_calls=len(stores),
        screens_in_copy=sum(1 for t in screens if copy.start <= t.start and t.end <= copy.end),
    )


def host_time() -> tuple[HostTime, ...]:
    """Each baseline host run's wall time by `Share`."""
    return tuple(
        attribute(operation, run)
        for operation, run in BASELINE_RUNS.items()
        if operation.name.startswith("HOST_")
    )


@dataclass(frozen=True)
class Summary:
    """Everything the report states."""

    budgets: tuple[Budget, ...]
    prices: Prices
    placement: Placement
    candidates: tuple[Candidate, ...]
    offline: Offline
    consumed: Consumed
    workload: Workload
    onevcs_call_seconds: float
    host_time: tuple[HostTime, ...]


def runs(prefix: str) -> list[Path]:
    """Every committed run directory whose name starts with `prefix`, oldest first."""
    if not LOGS.exists():
        return []
    return sorted(p for p in LOGS.iterdir() if p.is_dir() and p.name.startswith(prefix))


def latest(prefix: str) -> Path:
    """The newest run directory of `prefix`."""
    found = runs(prefix)
    if not found:
        raise ShapeError(f"no run {prefix}* under {LOGS}; run spike/gaps/gaps.sh first")
    return found[-1]


def records(run: Path) -> list[RequestRecord]:
    """A live run's request log, validated, in the order the requests started."""
    log = run / "requests.jsonl"
    entries = [RequestRecord.parse(line, f"{log}") for line in load_lines(log)]
    return sorted(entries, key=lambda r: r.start)


def rises(entries: list[RequestRecord]) -> list[tuple[RequestRecord, int]]:
    """Each request's `x-ratelimit-used` rise over the last request answered in its window.

    GitHub has reported two interleaving counters, so a rise is only ever read against the
    same window; other sessions share the bucket and only ever add, so the least rise of a
    kind bounds its price from above.
    """
    last: dict[int, int] = {}
    out: list[tuple[RequestRecord, int]] = []
    for record in entries:
        if record.limits is None or record.limits.resource != "graphql":
            continue
        window = record.limits.reset
        if window in last:
            out.append((record, record.limits.used - last[window]))
        last[window] = record.limits.used
    return out


def prices() -> Prices:
    """Every price, from the latest pricing probe and every placement probe."""
    pricing = decode(PricingResult, load(latest("probe-alias-pricing-") / "feasibility.json"), "p")
    dry = {p.variant: p.cost for p in pricing.dry_runs}
    charged = {p.variant: p.cost for p in pricing.executed}
    mutation: dict[int, list[int]] = {}
    for run in runs("probe-placement-"):
        for record, rise in rises(records(run)):
            if record.kind is Kind.MUTATION and not record.errors:
                mutation.setdefault(record.aliases, []).append(rise)
    dry_rises = tuple(
        rise
        for run in runs("probe-alias-pricing-")
        for record, rise in rises(records(run))
        if record.op == "dry run"
    )
    placements = [
        decode(PlacementResult, load(run / "feasibility.json"), str(run))
        for run in runs("probe-placement-")
        if (run / "feasibility.json").exists()
    ]
    polls = tuple(sorted({c for p in placements for c in p.placement.poll_costs}))
    least = {aliases: min(values) for aliases, values in sorted(mutation.items())}
    if not least or not polls:
        raise ShapeError("no placement probe recorded a mutation rise or a placement read")
    shape = {
        Shape.BATCHED_LOOKUP: dry[Named.LOOKUPS_FIRST_1],
        Shape.SINGLE_LOOKUP: dry[Named.LOOKUP],
        Shape.LISTING: dry[Named.LISTING],
        Shape.ITEM_READ: dry[Named.ISSUE],
        Shape.PLACEMENT_READ: max(polls),
        Shape.PARENT_LISTING: charged[Named.PARENT_TRIMMED],
        Shape.MUTATION: max(least.values()),
    }
    return Prices(
        dry_run=dry,
        charged=charged,
        dry_run_agrees_with_charge=all(dry.get(v) == c for v, c in charged.items()),
        mutation_rises_by_aliases={k: tuple(v) for k, v in sorted(mutation.items())},
        mutation_least_rise_by_aliases=least,
        dry_runs_moved_used_by=tuple(sorted(set(dry_rises))),
        placement_poll_costs=polls,
        shape=shape,
        parent_listing_seconds=listing_seconds(placements),
    )


def listing_seconds(placements: list[PlacementResult]) -> float:
    """The latest executed `SUB_ISSUES` read of a real parent.

    The nearest measured shape to the trimmed parent-scoped listing an approval would send,
    which lists the same members.
    """
    timed = [p.reference_resolution_seconds for p in placements if p.reference_resolution_seconds]
    if not timed:
        raise ShapeError("no placement probe timed its parent listing")
    return timed[-1]


def placement() -> Placement:
    """Every placement sample."""
    results = [
        decode(PlacementResult, load(run / "feasibility.json"), str(run))
        for run in runs("probe-placement-")
        if (run / "feasibility.json").exists()
    ]
    if not results:
        raise ShapeError(f"no probe-placement run under {LOGS}")
    return Placement(
        samples=len(results),
        issues=sum(r.placement.issues for r in results),
        children_seconds=tuple(
            s for r in results for s in r.placement.children_seconds_after_their_answer
        ),
        polls=tuple(r.placement.polls for r in results),
        in_create_answers=sum(r.board_items_in_create_answers for r in results),
        idempotent=all(r.idempotence.one_item_each_after_settling for r in results),
        add_outcomes=tuple(
            o
            for r in results
            for o in (*r.idempotence.racing_outcomes, r.idempotence.placed_outcome)
        ),
    )


def baseline() -> dict[Operation, Baseline]:
    """Each operation's baseline figures, from the re-measurement's own summary."""
    summary = obj(obj(load(BASELINE / "summary.json"), "summary").get("runs"), "summary.runs")
    out = {}
    for operation, run in BASELINE_RUNS.items():
        figures = decode(BaselineRun, summary.get(run), f"summary.runs.{run}")
        uploads = sum(
            v.count for k, v in figures.by_op.items() if k.startswith("POST /user-attachments")
        )
        out[operation] = Baseline(
            operation=operation,
            run=run,
            points=figures.points,
            requests=figures.requests,
            writes=figures.mutation_fields + uploads,
            wall_seconds=round(sum(figures.split.values()), 1),
            screening_seconds=round(
                sum(v for k, v in figures.split.items() if "screening" in k), 2
            ),
            screening_calls=sum(figures.screening_calls.values()),
        )
    return out


def plan(shape: dict[Shape, int], steps: list[Step]) -> Figure:
    """The points of a plan of request kinds, with its arithmetic."""
    total = sum(shape[s.shape] * s.count for s in steps)
    said = " + ".join(f"{s.what} {s.count}×{shape[s.shape]}" for s in steps)
    return Figure(value=total, arithmetic=f"{said} = {total}")


def first_copy(polls: int, writes: int) -> list[Step]:
    """The selected strategy's first copy.

    `polls` placement reads and `writes` field-and-edge requests in each process.
    """
    return [
        Step("project copy: origin lookups of the project and 8 tasks", Shape.BATCHED_LOOKUP, 1),
        Step("create the project issue", Shape.MUTATION, 1),
        Step(
            "create 8 task issues, aliased, with projectV2Ids and parentIssueId", Shape.MUTATION, 1
        ),
        Step("placement read of 9 issues", Shape.PLACEMENT_READ, polls),
        Step("field values and 10 blocked-by edges, aliased", Shape.MUTATION, writes),
        Step("document copy: its origin lookup", Shape.SINGLE_LOOKUP, 1),
        Step("create the document issue", Shape.MUTATION, 1),
        Step("placement read", Shape.PLACEMENT_READ, polls),
        Step("its field value", Shape.MUTATION, 1),
    ]


PLANS: dict[Operation, list[Step]] = {
    Operation.BARE_FIRST: first_copy(1, 1),
    Operation.HOST_FIRST: first_copy(1, 1),
    Operation.BARE_CHANGED: [
        Step("project copy: listing", Shape.LISTING, 1),
        Step("update the changed body", Shape.MUTATION, 1),
        Step("document copy: listing", Shape.LISTING, 1),
    ],
    Operation.HOST_CHANGED: [
        Step("project copy: listing (field options from it)", Shape.LISTING, 1),
        Step("2 body updates, aliased", Shape.MUTATION, 1),
        Step("document copy: listing", Shape.LISTING, 1),
    ],
    Operation.BARE_UNCHANGED: [
        Step("project copy: listing", Shape.LISTING, 1),
        Step("document copy: listing", Shape.LISTING, 1),
    ],
    Operation.HOST_UNCHANGED: [
        Step("project copy: listing", Shape.LISTING, 1),
        Step("document copy: listing", Shape.LISTING, 1),
    ],
    Operation.HOST_APPROVAL: [
        Step("document list --project", Shape.LISTING, 1),
        Step("project show", Shape.ITEM_READ, 1),
        Step("task list --project", Shape.LISTING, 1),
        Step("metadata set: document, project and members in one read", Shape.PARENT_LISTING, 1),
        Step("the write", Shape.MUTATION, 1),
    ],
}
# The reads each operation's prototype plan no longer sends, by operation and count, taken from
# the end of the baseline run.
REMOVED: dict[Operation, tuple[tuple[str, int], ...]] = {
    Operation.BARE_FIRST: (
        ("ORIGIN_LOOKUP", 3),
        ("ISSUE", 1),
        ("SUB_ISSUES", 1),
        ("CREATED_BOARD_ITEMS", 2),
        ("WRITE_FIELD_VALUES", 1),
    ),
    Operation.BARE_CHANGED: (("ORIGIN_LOOKUP", 4),),
    Operation.BARE_UNCHANGED: (("ORIGIN_LOOKUP", 4),),
    Operation.HOST_FIRST: (("ISSUE", 2), ("CREATED_BOARD_ITEMS", 2), ("SUB_ISSUES", 1)),
    Operation.HOST_CHANGED: (
        ("BOARD_FIELDS", 1),
        ("ORIGIN_LOOKUP", 1),
        ("ISSUE", 1),
        ("UPDATE_ISSUE", 1),
    ),
    Operation.HOST_UNCHANGED: (("ORIGIN_LOOKUP", 1), ("ISSUE", 1)),
    Operation.HOST_APPROVAL: (("ISSUE", 2), ("SUB_ISSUES", 1)),
}
# The onevcs calls each host operation still makes with union screening: `None` is the copy's
# own measured union figure; a re-copy screens only the changed exposure, an unchanged one
# writes nothing and so screens nothing, an approval screens its one write.
SCREENED_CALLS_AFTER: dict[Operation, int | None] = {
    Operation.HOST_FIRST: None,
    Operation.HOST_CHANGED: 4,
    Operation.HOST_UNCHANGED: 0,
    Operation.HOST_APPROVAL: 2,
}


def removed(base: Baseline, picks: tuple[tuple[str, int], ...]) -> tuple[float, str]:
    """The seconds the last `n` requests of each named operation took in a baseline run."""
    log = BASELINE / base.run / "requests.jsonl"
    requests = [decode(BaselineRequest, line, str(log)) for line in load_lines(log)]
    total, said = 0.0, []
    for op, n in picks:
        mine = [r.ms for r in requests if r.op == op][-n:]
        total += sum(mine) / 1000
        said.append(f"{len(mine)} {op}")
    return round(total, 2), ", ".join(said)


def onevcs_call_seconds(bases: dict[Operation, Baseline]) -> float:
    """The slowest mean seconds a host onevcs call took in the baseline.

    Screening time over calls, for every host operation that screened.
    """
    means = [b.screening_seconds / b.screening_calls for b in bases.values() if b.screening_calls]
    if not means:
        raise ShapeError("no baseline host operation screened anything")
    return round(max(means), 2)


def seconds(base: Baseline, union_calls: int, parent_seconds: float, per_call: float) -> Figure:
    """A seconds estimate: the baseline less the reads removed, its screening replaced."""
    cut, which = removed(base, REMOVED[base.operation])
    if base.operation not in SCREENED_CALLS_AFTER:
        after = round(base.wall_seconds - cut, 1)
        return Figure(after, f"{base.wall_seconds} s measured − {cut} s ({which}) = {after} s")
    screened = SCREENED_CALLS_AFTER[base.operation]
    calls = union_calls if screened is None else screened
    added = parent_seconds if base.operation is Operation.HOST_APPROVAL else 0.0
    after = round(base.wall_seconds - base.screening_seconds + calls * per_call - cut + added, 1)
    return Figure(
        after,
        f"{base.wall_seconds} s measured − {base.screening_seconds} s screening "
        f"({base.screening_calls} onevcs calls) + {calls} calls × {per_call} s − {cut} "
        "s "
        f"({which}) + {added} s added = {after} s",
    )


def content_creation_per_minute() -> int:
    """GitHub's published per-minute ceiling on content-creating requests, as pinned."""
    pinned = obj(load(RATE_LIMITS), str(RATE_LIMITS))
    creation = obj(pinned.get("content_creation"), f"{RATE_LIMITS}.content_creation")
    return integer(creation.get("per_minute"), f"{RATE_LIMITS}.content_creation.per_minute")


def hourly_allowance() -> int:
    """The GraphQL bucket's limit, as the latest live run's gate read it."""
    for run in reversed(runs("probe-")):
        quota = run / "quota.json"
        if quota.exists() and "before" in (recorded := obj(load(quota), str(quota))):
            return decode(Reading, recorded["before"], f"{quota}.before").least.limit
    raise ShapeError(f"no live run under {LOGS} recorded a quota reading")


def candidate(
    budget: Budget,
    base: Baseline,
    prices_: Prices,
    union_calls: int,
    allowance: int,
    per_call: float,
) -> Candidate:
    """One budget's attainable figure, its arithmetic and its limits."""
    shape = prices_.shape
    worst_value: float | None = None
    match budget.unit:
        case Unit.POINTS:
            figure = plan(shape, PLANS[budget.operation])
            label = "estimate from observed prices"
            arithmetic = figure.arithmetic
            if budget.operation in (Operation.BARE_FIRST, Operation.HOST_FIRST):
                worst = plan(shape, first_copy(2, 2))
                worst_value = worst.value
                arithmetic += (
                    "; worst case, two placement reads and two write requests a process: "
                    f"{worst.arithmetic}"
                )
            burst = BURST_COPIES_AN_HOUR * figure.value
            limits = (
                f"GraphQL allows {allowance} points an hour, shared with all host "
                "work: "
                f"{BURST_COPIES_AN_HOUR} operations an hour at {figure.value:g} is {burst:g} "
                "points "
                f"({burst / allowance:.2%}). Every price is GitHub's own (dry run, "
                "matched "
                "by the charge of each coalesced shape); a mutation request rose by its least "
                "observed amount whatever its aliases, up to "
                f"{max(prices_.mutation_least_rise_by_aliases)}."
            )
        case Unit.WRITES:
            figure = Figure(
                base.writes, f"{base.writes} as measured: coalescing moves requests, not mutations"
            )
            label = "observed, unchanged by the selected strategy"
            limits = (
                f"GitHub's secondary limit is {content_creation_per_minute()} content-creating "
                "requests a minute; the first copy "
                "creates 10 issues in 3 requests and uploads 1 asset. The total holds only while "
                "placement rides on createIssue(projectV2Ids): an addProjectV2ItemById per issue "
                "adds 10."
            )
            arithmetic = figure.arithmetic
        case Unit.SECONDS:
            figure = seconds(base, union_calls, prices_.parent_listing_seconds, per_call)
            label = "estimate"
            arithmetic = figure.arithmetic
            limits = (
                "Seconds warn on this host; one baseline sample each; host onevcs calls at "
                f"{per_call} s, the baseline's slowest mean."
            )
    return Candidate(
        budget=budget,
        baseline=base,
        attainable=figure.value,
        within=figure.value <= budget.threshold,
        label=label,
        arithmetic=arithmetic,
        limits=limits,
        worst=worst_value,
    )


def offline() -> Offline:
    """The latest result of every offline case and build."""
    reach: dict[str, ReachResult] = {}
    names: dict[str, str] = {}
    for case in Case:
        for build in ("stock", "head", "closure"):
            found = runs(f"{case.value}-{build}-")
            if found:
                key = f"{case.value} {build}"
                reach[key] = decode(ReachResult, load(found[-1] / "result.json"), str(found[-1]))
                names[key] = found[-1].name
    union_run = latest("screen-union-closure-")
    refusal_run = latest("proxy-refusal-")
    return Offline(
        reach=reach,
        reach_runs=names,
        union=decode(UnionResult, load(union_run / "result.json"), str(union_run)),
        union_run=union_run.name,
        refusal=decode(RefusalResult, load(refusal_run / "result.json"), str(refusal_run)),
        refusal_run=refusal_run.name,
    )


def consumed() -> Consumed:
    """What the live runs sent, priced as the probes observed."""
    entries = [(run, records(run)) for run in runs("probe-")]
    all_records = [r for _, rs in entries for r in rs]
    free = {"dry run", "rateLimit", "GET /rate_limit"}
    # A dry run states the price of the document it carries, not of itself, and moved `used` by
    # nothing; it and the quota reads cost 0. Every other query costs what it stated, or 1.
    points = sum(
        0 if r.op in free else (r.cost if r.cost is not None else 1)
        for r in all_records
        if r.kind is not Kind.REST
    )
    remaining = {}
    shares: list[float] = []
    pauses, recording = 0, 0
    for run, _ in entries:
        if not (run / "quota.json").exists():
            continue
        quota = obj(load(run / "quota.json"), str(run))
        if "before" not in quota or "after" not in quota:
            continue
        before = decode(Reading, quota.get("before"), f"{run}/quota.json.before")
        after = decode(Reading, quota.get("after"), f"{run}/quota.json.after")
        remaining[run.name] = (before.least.remaining, after.least.remaining)
        shares.append(before.least.remaining / before.least.limit)
        if "pauses" in quota:
            recording += 1
            pauses += integer(quota.get("pauses"), f"{run}/quota.json.pauses")
    placements = runs("probe-placement-")
    last = placements[-1] if placements else None
    left = None
    if last is not None and (last / "feasibility.json").exists():
        left = decode(PlacementResult, load(last / "feasibility.json"), str(last)).cleanup.left
    return Consumed(
        runs=len(entries),
        requests=len(all_records),
        dry_runs=sum(1 for r in all_records if r.op == "dry run"),
        rate_limit_reads=sum(1 for r in all_records if r.op in ("rateLimit", "GET /rate_limit")),
        mutation_requests=sum(1 for r in all_records if r.kind is Kind.MUTATION),
        creations_sent=sum(r.aliases for r in all_records if r.op.startswith("ProbeCreate")),
        points=points,
        seconds=round(sum(r.end - r.start for r in all_records), 1),
        remaining_before_and_after=remaining,
        least_share_before=round(min(shares), 3) if shares else 0.0,
        pauses=pauses,
        runs_recording_pauses=recording,
        issues_left_by_the_last_placement=left,
    )


def analyse(budgets: tuple[Budget, ...]) -> Summary:
    """Every figure the report states."""
    priced = prices()
    bases = baseline()
    found = offline()
    allowance = hourly_allowance()
    per_call = onevcs_call_seconds(bases)
    union_calls = sum(
        next(v.unioned.calls for v in variants if v.plant is Plant.NONE)
        for variants in found.union.commands.values()
    )
    return Summary(
        budgets=budgets,
        prices=priced,
        placement=placement(),
        candidates=tuple(
            candidate(b, bases[b.operation], priced, union_calls, allowance, per_call)
            for b in budgets
        ),
        offline=found,
        consumed=consumed(),
        workload=measured_workload(),
        onevcs_call_seconds=per_call,
        host_time=host_time(),
    )
