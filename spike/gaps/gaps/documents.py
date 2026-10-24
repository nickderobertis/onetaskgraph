"""The GitHub plugin's own GraphQL documents, and the shapes this spike prices built from them.

`documents.json` beside this package is generated from the plugin crate by `generate`, which
compiles a throwaway example printing the constants; `tests/test_contracts.py` regenerates it
and fails when the checked-in copy has drifted from the crate. The variable builders are held
to each document's own declarations by `declared` in the same file.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path

from gaps import proc
from gaps.json_shape import Json, ShapeError, load, obj, text

HERE = Path(__file__).resolve().parent
SNAPSHOT = HERE.parent / "documents.json"
CRATE = "crates/onetaskgraph-github-projects"
LOOKUP_SLOTS = 10
PLACEMENT_SLOTS = 21
TITLE = "bcg-probe"


class Doc(StrEnum):
    """The plugin constants this spike prices."""

    ORIGIN_LOOKUPS = "ORIGIN_LOOKUPS"
    ORIGIN_LOOKUP = "ORIGIN_LOOKUP"
    CREATED_BOARD_ITEMS = "CREATED_BOARD_ITEMS"
    SUB_ISSUES = "SUB_ISSUES"
    ISSUE = "ISSUE"
    CREATION_CONTEXT = "CREATION_CONTEXT"


def snapshot(path: Path = SNAPSHOT) -> dict[Doc, str]:
    """The checked-in documents, validated."""
    raw = obj(load(path), str(path))
    try:
        return {doc: text(raw[doc.value], f"{path}:{doc.value}") for doc in Doc}
    except KeyError as missing:
        raise ShapeError(f"{path}: missing {missing}") from missing


async def generate(repo: Path) -> dict[str, str]:
    """Every `Doc` as the plugin crate at `repo` compiles it."""
    examples = repo / CRATE / "examples"
    created = not examples.exists()
    examples.mkdir(exist_ok=True)
    source = examples / "spike_gaps_documents.rs"
    lines = ["use onetaskgraph_github_projects::graphql::*;", "fn main() {"]
    lines += [f'    println!("{{}}\\t{{:?}}", "{doc.value}", {doc.value});' for doc in Doc]
    source.write_text("\n".join(lines + ["}"]) + "\n")
    try:
        done = await proc.check(
            ["cargo", "run", "-q", "-p", "onetaskgraph-github-projects", "--example", source.stem],
            cwd=repo,
        )
    finally:
        source.unlink()
        if created:
            examples.rmdir()
    out: dict[str, str] = {}
    for line in done.out.splitlines():
        name, _, debug = line.partition("\t")
        out[name] = json.loads(debug)
    return out


@dataclass(frozen=True)
class Parts:
    """A `query(<defs>){<body>} fragment …` document taken apart."""

    defs: tuple[str, ...]
    body: str
    fragments: dict[str, str]


def _closing(document: str, start: int, open_: str, close: str) -> int:
    depth = 0
    for index in range(start, len(document)):
        depth += {open_: 1, close: -1}.get(document[index], 0)
        if depth == 0:
            return index
    raise ShapeError(f"unbalanced {open_}{close} from offset {start}")


def split(document: str) -> Parts:
    """A document as its variable definitions, top-level selection and fragments."""
    open_paren = document.index("(")
    close_paren = _closing(document, open_paren, "(", ")")
    defs, current, brackets = [], "", 0
    for char in document[open_paren + 1 : close_paren]:
        brackets += {"[": 1, "]": -1}.get(char, 0)
        if char == "," and brackets == 0:
            defs.append(current.strip())
            current = ""
        else:
            current += char
    if current.strip():
        defs.append(current.strip())
    start = document.index("{", close_paren)
    end = _closing(document, start, "{", "}")
    fragments = {
        chunk.split(" ", 1)[0]: "fragment " + chunk.strip()
        for chunk in document[end + 1 :].split("fragment ")[1:]
    }
    return Parts(tuple(defs), document[start + 1 : end], fragments)


def declared(document: str) -> tuple[frozenset[str], frozenset[str]]:
    """Every variable a document declares, and those it requires (non-null with no default)."""
    names, required = set(), set()
    for definition in split(document).defs:
        name, _, rest = definition.partition(":")
        names.add(name.strip().lstrip("$"))
        if rest.strip().split("=")[0].strip().endswith("!") and "=" not in rest:
            required.add(name.strip().lstrip("$"))
    return frozenset(names), frozenset(required)


def merge(*documents: str) -> str:
    """One query selecting everything each document selects, shared variables declared once."""
    defs: dict[str, str] = {}
    bodies: list[str] = []
    fragments: dict[str, str] = {}
    for document in documents:
        parts = split(document)
        for definition in parts.defs:
            name = definition.split(":", 1)[0].strip()
            if name in defs and defs[name].split("=")[0] != definition.split("=")[0]:
                raise ValueError(f"{name} is declared two ways")
            defs.setdefault(name, definition)
        bodies.append(parts.body)
        fragments.update(parts.fragments)
    joined = " ".join(fragments.values())
    return "query(" + ",".join(defs.values()) + "){" + "\n".join(bodies) + "} " + joined


def through_parent(document: str, trimmed: bool = False) -> str:
    """`SUB_ISSUES` asked of an item's parent rather than of the item.

    One request answers a document, the project it is filed under and every other record filed
    there. `trimmed` asks nothing of the item itself but its id, since the parent's listing
    answers it.
    """
    parts = split(document)
    body = parts.body
    if trimmed:
        body = body.replace(
            "node(id:$id){__typename ...BoardIssue", "node(id:$id){__typename id", 1
        )
    start = body.index("... on Issue{") + len("... on Issue{")
    end = _closing(body, start - 1, "{", "}")
    nested = body[:start] + "parent{__typename ...BoardIssue " + body[start:end] + "}" + body[end:]
    joined = " ".join(parts.fragments.values())
    return "query(" + ",".join(parts.defs) + "){" + nested + "} " + joined


def dry_run(document: str) -> str:
    """The document asking GitHub only for its price (`rateLimit(dryRun: true)`)."""
    root = document.index("{")
    return document[: root + 1] + "rateLimit(dryRun:true){cost} " + document[root + 1 :]


def charged(document: str) -> str:
    """The document asking, beside its answer, what GitHub charged for it."""
    root = document.index("{")
    return document[: root + 1] + "rateLimit{cost used remaining resetAt} " + document[root + 1 :]


def lookups(owner: str, number: int, repo: str, asks: int, first: int = 3) -> dict[str, Json]:
    """`ORIGIN_LOOKUPS` variables asking `asks` of its slots for origins nothing carries."""
    owner_name, _, repo_name = repo.partition("/")
    out: dict[str, Json] = {
        "owner": owner,
        "number": number,
        "type": "ISSUE",
        "originFirst": first,
        "nestedFirst": 50,
        "boardItems": 3,
        "duplicates": True,
        "repositoryOwner": owner_name,
        "repositoryName": repo_name,
    }
    for slot in range(LOOKUP_SLOTS):
        origin = f"spike-md:{TITLE}/t{slot}"
        out[f"filter{slot}"] = f'onetaskgraph.origin:"{origin}"'
        out[f"search{slot}"] = f'project:{owner}/{number} is:issue in:body "{origin}"'
        out[f"ask{slot}"] = slot < asks
    return out


def lookup(owner: str, number: int) -> dict[str, Json]:
    """`ORIGIN_LOOKUP` variables for one origin nothing carries."""
    origin = f"spike-md:{TITLE}/t0"
    return {
        "owner": owner,
        "number": number,
        "type": "ISSUE",
        "originFirst": 3,
        "nestedFirst": 50,
        "boardItems": 3,
        "duplicates": True,
        "filter": f'onetaskgraph.origin:"{origin}"',
        "search": f'project:{owner}/{number} is:issue in:body "{origin}"',
        "itemsAfter": None,
        "searchAfter": None,
    }


def placements(ids: list[str], reads: int) -> dict[str, Json]:
    """`CREATED_BOARD_ITEMS` variables reading `reads` of its slots."""
    out: dict[str, Json] = {"boardItems": 3, "statusName": "Status"}
    for slot in range(PLACEMENT_SLOTS):
        out[f"id{slot}"] = ids[slot % len(ids)]
        out[f"read{slot}"] = slot < reads
    return out


def members(parent: str, first: int = 23) -> dict[str, Json]:
    """`SUB_ISSUES` variables listing `first` members of `parent`."""
    return {
        "id": parent,
        "memberFirst": first,
        "after": None,
        "nestedFirst": 50,
        "boardItems": 3,
        "duplicates": True,
    }


def issue(node: str) -> dict[str, Json]:
    """`ISSUE` variables for one issue."""
    return {"id": node, "first": 100, "nestedFirst": 50, "boardItems": 3, "duplicates": True}
