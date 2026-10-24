"""The real onevcs boundary commands, against an onevcs home holding only invented identities.

The check input's fields come from the boundary schema the store pins
(`crates/onetaskgraph-core/src/engine/boundary/onevcs-boundary-schema-v1.json`), read at run
time: every array field it declares is one a union concatenates, so a field the schema gains
is unioned without an edit here. An identity's visibility is declared by a rule, so no host
is ever asked; after writing the rules, `onevcs rules check` confirms each identity matches
the rule written for it and `boundary inspect` that it answers the declared visibility, so a
rules file onevcs no longer reads as written fails the run here.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import IntEnum, StrEnum
from pathlib import Path

from gaps import proc
from gaps.json_shape import Json, ShapeError, arr, field, load, obj, parse, text

REPO = Path(__file__).resolve().parents[3]
PINNED_SCHEMA = REPO / "crates/onetaskgraph-core/src/engine/boundary/onevcs-boundary-schema-v1.json"


class Visibility(StrEnum):
    """Who can read a repository, as onevcs answers it."""

    PUBLIC = "public"
    PRIVATE = "private"
    UNKNOWN = "unknown"


class Verdict(IntEnum):
    """`onevcs boundary check`'s exit statuses: 0 passes, 1 refuses, else unavailable."""

    PASS = 0
    REFUSE = 1
    UNAVAILABLE = 2

    @classmethod
    def of(cls, returncode: int) -> Verdict:
        """The verdict an exit status means."""
        match returncode:
            case 0:
                return cls.PASS
            case 1:
                return cls.REFUSE
            case _:
                return cls.UNAVAILABLE


@dataclass(frozen=True)
class Identity:
    """An invented repository registered in the run's onevcs home."""

    owner: str
    name: str
    visibility: Visibility

    @property
    def key(self) -> str:
        """`owner/name`."""
        return f"{self.owner}/{self.name}"

    @property
    def qualified(self) -> str:
        """`github.com/owner/name`."""
        return f"github.com/{self.key}"


def merged_fields(schema: Path = PINNED_SCHEMA) -> tuple[str, ...]:
    """The check input's array fields, as the pinned schema declares them."""
    check = obj(obj(load(schema), str(schema)).get("check"), "check")
    properties = obj(obj(check.get("input"), "check.input").get("properties"), "properties")
    return tuple(
        name for name, spec in properties.items() if obj(spec, name).get("type") == "array"
    )


@dataclass(frozen=True)
class CheckInput:
    """One `onevcs boundary check` input: its array fields, its scope and its destination."""

    arrays: dict[str, tuple[str, ...]]
    scope: tuple[str, ...] | None
    destination: Visibility

    @classmethod
    def parse(cls, value: Json, where: str) -> CheckInput:
        """A recorded check input, validated against the pinned schema's array fields."""
        entry = obj(value, where)
        arrays = {
            name: tuple(text(v, f"{where}.{name}[]") for v in arr(entry.get(name, []), name))
            for name in merged_fields()
        }
        scope = entry.get("scope")
        destination = Visibility(text(field(entry, "destination", where), f"{where}.destination"))
        return cls(
            arrays=arrays,
            scope=None if scope is None else tuple(text(s, "scope[]") for s in arr(scope, "scope")),
            destination=destination,
        )

    def to_json(self) -> dict[str, Json]:
        """The input as onevcs reads it."""
        out: dict[str, Json] = {"destination": self.destination.value}
        out |= {name: list(values) for name, values in self.arrays.items()}
        if self.scope is not None:
            out["scope"] = list(self.scope)
        return out


def union(checks: list[CheckInput]) -> CheckInput:
    """One check carrying every check's arrays element by element, under their one scope."""
    scopes = {c.scope for c in checks}
    if len(scopes) != 1:
        raise ValueError(f"the checks carried {len(scopes)} different scopes; a union needs one")
    destinations = {c.destination for c in checks}
    if len(destinations) != 1:
        raise ValueError("the checks name different destinations; a union needs one")
    arrays = {name: tuple(v for c in checks for v in c.arrays[name]) for name in merged_fields()}
    return CheckInput(arrays=arrays, scope=checks[0].scope, destination=checks[0].destination)


@dataclass(frozen=True)
class OnevcsHome:
    """An onevcs home this run owns, and the onevcs that reads it."""

    path: Path
    onevcs: Path

    def env(self) -> dict[str, str]:
        """The variables every onevcs command against this home is run with."""
        return {"ONEVCS_HOME": str(self.path)}


async def make_home(root: Path, onevcs: Path, identities: tuple[Identity, ...]) -> OnevcsHome:
    """Register `identities` in a new home under `root`, each declared by a rule."""
    home = OnevcsHome(path=root / "onevcs-home", onevcs=onevcs)
    home.path.mkdir(parents=True)
    rules = "version: 4\nrules:\n" + "".join(
        f"  - match: {{host: github.com, owner: {i.owner}, name: {i.name}}}\n"
        f"    visibility: {i.visibility.value}\n"
        for i in identities
    )
    (home.path / "rules.yml").write_text(
        rules + "default: {publication: change-open, approvals: required}\n"
    )
    environment = {"PATH": "/usr/bin:/bin"} | home.env()
    for identity in identities:
        checkout = root / "checkouts" / f"{identity.owner}-{identity.name}"
        checkout.mkdir(parents=True)
        (checkout / "README.md").write_text(f"# {identity.name}\n")
        git = ["git", "-c", "user.name=Synthetic", "-c", "user.email=s@example.invalid"]
        for args in (["init", "-q", "-b", "main"], ["add", "-A"], ["commit", "-qm", "synthetic"]):
            await proc.check([*git, *args], cwd=checkout, env=environment)
        origin = f"https://github.com/{identity.key}.git"
        await proc.check([onevcs, "register", checkout, "--origin", origin], env=environment)
    for number, identity in enumerate(identities, 1):
        matched = await proc.check([onevcs, "rules", "check", identity.qualified], env=environment)
        rule = (
            f"matched: rule {number} "
            f"{{host: github.com, owner: {identity.owner}, name: {identity.name}}}"
        )
        if rule not in matched.out:
            raise ShapeError(f"onevcs did not match {identity.key} to the rule written for it")
        if await inspect(home, identity.qualified) is not identity.visibility:
            raise ShapeError(
                f"onevcs does not answer {identity.visibility.value} for {identity.key}"
            )
    return home


async def inspect(home: OnevcsHome, repository: str) -> Visibility:
    """`onevcs boundary inspect` of one repository."""
    done = await proc.check(
        [home.onevcs, "boundary", "inspect", "--input", "-"],
        stdin=json.dumps({"repository": repository}).encode(),
        env={"PATH": "/usr/bin:/bin"} | home.env(),
    )
    answer = obj(parse(done.out, "inspect"), "inspect")
    return Visibility(text(answer.get("visibility"), "inspect.visibility"))


async def check(home: OnevcsHome, request: CheckInput) -> tuple[Verdict, float]:
    """`onevcs boundary check` of one input: its verdict, and the seconds it took."""
    done = await proc.run(
        [
            home.onevcs,
            "boundary",
            "check",
            "--destination",
            request.destination.value,
            "--input",
            "-",
        ],
        stdin=json.dumps(request.to_json()).encode(),
        env={"PATH": "/usr/bin:/bin"} | home.env(),
    )
    return Verdict.of(done.returncode), done.seconds
