"""Command-scoped deduplicated inspection and union screening, against the real onevcs.

`screen-union` runs `reach-public`, keeping every `write_policy` call each copy command made
in that command's own log, and replays each command's calls through the same
onevcs and home three ways: per item, as the store sends them; deduplicated and unioned, as a
command-scoped store would (one inspect per distinct repository, one check of every check's
arrays concatenated element by element); and planted, after one item's metadata, then its
text, then a path it names carries the synthetic private identity's name.

The two ways agree only when every inspection answers the same visibility both ways and the
union's verdict is exactly the strictest per-item verdict: a refusal beside a pass refuses, and
an unavailable check anywhere is unavailable, never a pass.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, replace
from enum import StrEnum
from pathlib import Path

from gaps import boundary, reach
from gaps.boundary import CheckInput, OnevcsHome, Verdict, Visibility
from gaps.json_shape import Json, encode, obj, parse, text
from gaps.reach import Case, ReachResult
from gaps.shim import Command


class Plant(StrEnum):
    """Where a variant writes the private identity's name."""

    NONE = "as copied"
    METADATA = "planted in metadata"
    TEXT = "planted in text"
    PATHS = "planted in paths"


@dataclass(frozen=True)
class Way:
    """One way of screening a command's writes."""

    calls: int
    inspect: int
    check: int
    verdict: Verdict
    visibilities: tuple[Visibility, ...]
    seconds: float


@dataclass(frozen=True)
class Variant:
    """One variant of one command, screened both ways."""

    plant: Plant
    per_item: Way
    unioned: Way

    @property
    def agrees(self) -> bool:
        """Same visibilities both ways, and the union's verdict the strictest per-item one."""
        return (
            set(self.per_item.visibilities) == set(self.unioned.visibilities)
            and self.per_item.verdict == self.unioned.verdict
        )


@dataclass(frozen=True)
class UnionResult:
    """The copy, and every command's variants."""

    copy: ReachResult
    commands: dict[str, tuple[Variant, ...]]

    @property
    def all_agree(self) -> bool:
        """Whether every variant of every command agrees; no command screened is no agreement."""
        variants = [v for each in self.commands.values() for v in each]
        return bool(variants) and all(v.agrees for v in variants)

    def to_json(self) -> Json:
        """The result as `result.json` holds it, its agreement stated beside it."""
        return obj(encode(self), "union") | {"all_agree": self.all_agree}


def strictest(verdicts: list[Verdict]) -> Verdict:
    """What a set of verdicts allows together: unavailable over refuse over pass."""
    return max(verdicts, default=Verdict.PASS)


def planted(checks: list[CheckInput], plant: Plant, term: str) -> list[CheckInput]:
    """The same checks with `term` written into the third item's `plant` array."""
    if plant is Plant.NONE:
        return checks
    name = plant.name.lower()
    target = min(2, len(checks) - 1)
    values = list(checks[target].arrays[name])
    marked = f"notes/{term}/plan.md" if plant is Plant.PATHS else f"see {term}"
    values = [f"{values[0]} see {term} for details", *values[1:]] if values else [marked]
    arrays = checks[target].arrays | {name: tuple(values)}
    return [*checks[:target], replace(checks[target], arrays=arrays), *checks[target + 1 :]]


async def screen(
    home: OnevcsHome, repositories: list[str], checks: list[CheckInput], plant: Plant, term: str
) -> Variant:
    """One variant of one command, per item and unioned."""
    checks = planted(checks, plant, term)
    item_visibility = [await boundary.inspect(home, r) for r in repositories]
    item_checks = [await boundary.check(home, c) for c in checks]
    distinct = sorted(set(repositories))
    union_visibility = [await boundary.inspect(home, r) for r in distinct]
    union_verdict, union_seconds = await boundary.check(home, boundary.union(checks))
    return Variant(
        plant=plant,
        per_item=Way(
            calls=len(repositories) + len(checks),
            inspect=len(repositories),
            check=len(checks),
            verdict=strictest([v for v, _ in item_checks]),
            visibilities=tuple(item_visibility),
            seconds=round(sum(s for _, s in item_checks), 3),
        ),
        unioned=Way(
            calls=len(distinct) + 1,
            inspect=len(distinct),
            check=1,
            verdict=union_verdict,
            visibilities=tuple(union_visibility),
            seconds=round(union_seconds, 3),
        ),
    )


async def run(run_dir: Path, exe: Path, onevcs: Path) -> UnionResult:
    """Copy the synthetic public plan, then replay its screening every way."""
    copied = await reach.run(run_dir, Case.PUBLIC, exe, onevcs)
    home = OnevcsHome(path=run_dir / "sandbox" / "onevcs-home", onevcs=onevcs)
    commands: dict[str, tuple[Variant, ...]] = {}
    for step in (s.step for s in copied.steps):
        made, _ = reach.screens([reach.screen_log(run_dir, step)])
        repositories = [
            text(
                obj(parse(c.input, "inspect input"), "inspect input").get("repository"),
                "repository",
            )
            for c in made
            if c.name is Command.INSPECT
        ]
        checks = [
            CheckInput.parse(parse(c.input, "check input"), "check input")
            for c in made
            if c.name is Command.CHECK
        ]
        if not checks:
            continue
        commands[step] = tuple(
            [await screen(home, repositories, checks, plant, reach.PRIVATE.key) for plant in Plant]
        )
    result = UnionResult(copy=copied, commands=commands)
    (run_dir / "result.json").write_text(json.dumps(result.to_json(), indent=1))
    return result
