"""A writable task source that meters its requests and searches natively, over the stdio protocol.

`sources sanitize` searches a source rather than listing it only when that source spends a
quota, which it knows by the source answering `metering` (§3.4). Every credential-free source
this build ships spends none, so the SDK's sweep journeys speak to this peer, spoken to over
`docs/plugin-protocol.md` exactly as the engine speaks to any plugin. Its tasks live in the JSON
file named by its one argument, so what one invocation writes the next one reads.

"""

import json
import sys
from pathlib import Path
from typing import Literal, NotRequired, TypedDict, cast, get_args

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    JsonValue,
    TypeAdapter,
    ValidationError,
    field_validator,
)

from onetaskgraph_sdk import StatusCategory
from onetaskgraph_sdk._generated.document_query import TextFields
from onetaskgraph_sdk._generated.query_response_of_qualified_edge import (
    DependencyKind,
    Designator,
    QualifiedEndpoint,
)
from onetaskgraph_sdk._generated.task_detail import Task as WireTask

KIND = "sweep-source"
PROTOCOL_VERSION = 2
MAX_PAGE_SIZE = 100
DependencyDirection = Literal["depends-on", "depended-on-by"]

CAPABILITIES = {
    "projects": "unsupported",
    "documents": "unsupported",
    "comments": "native",
    "orphan_tasks": "native",
    "filter_by_label": "unsupported",
    "filter_by_status": "unsupported",
    "search_title": "native",
    "search_content": "native",
    "task_dependencies": "forward-only",
    "project_dependencies": "forward-only",
    "max_page_size": MAX_PAGE_SIZE,
}


type TaskId = str
type ProjectId = str


class Label(TypedDict):
    """A named label with the protocol's nullable colour."""

    id: str
    name: str
    color: NotRequired[str | None]


class UrlLocation(TypedDict):
    """A hosted record's link (§4.4)."""

    url: str


class PathLocation(TypedDict):
    """A local record's absolute path (§4.4)."""

    path: str


type Location = UrlLocation | PathLocation


class Status(TypedDict):
    """A task's status on the wire (§4.4)."""

    category: StatusCategory
    name: str


class Task(TypedDict):
    """A task as this peer holds it and sends it (§4.4)."""

    id: TaskId
    key: NotRequired[str | None]
    version: NotRequired[str | None]
    title: str
    content: str | None
    status: Status
    labels: list[Label]
    project: ProjectId | None
    url: str | None
    location: Location | None
    created_at: str | None
    updated_at: str | None


class WireEndpoint(QualifiedEndpoint):
    """The native wire endpoint also accepts the protocol's opaque designator (§4.7)."""

    model_config = ConfigDict(extra="forbid")

    designator: Designator | None = None


class WireEdge(BaseModel):
    """A native edge, using the generated endpoint and relationship vocabularies."""

    model_config = ConfigDict(extra="forbid")

    from_: WireEndpoint = Field(alias="from")
    to: WireEndpoint
    kind: DependencyKind


class PageRequest(BaseModel):
    """The peer issues decimal offsets and honours its advertised page ceiling."""

    cursor: str | None = None
    limit: int = Field(strict=True, gt=0, le=MAX_PAGE_SIZE)

    @field_validator("cursor")
    @classmethod
    def issued_cursor(cls, value: str | None) -> str | None:
        """Reject a cursor the peer's decimal encoder could not have issued."""
        if value is not None and (
            not value.isascii() or not value.isdecimal() or value != str(int(value))
        ):
            raise ValueError("a cursor is a canonical decimal offset")
        return value

    @property
    def offset(self) -> int:
        """The first row to return, after cursor validation."""
        return 0 if self.cursor is None else int(self.cursor)


class SearchText(BaseModel):
    """A hosted index may keep these older strings after the record itself changes."""

    title: str
    content: str | None = None


class TextQuery(BaseModel):
    """The three native text selectors of protocol §4.5."""

    terms: str
    fields: TextFields = TextFields.TextFieldsTitleOrContent


class Held(TypedDict):
    """The peer's own requests, records, relationships and lagging text index."""

    requests: int
    tasks: list[Task]
    edges: dict[TaskId, list[WireEdge]]
    index: dict[TaskId, SearchText]


def _decode[T: BaseModel](model: type[T], value: object) -> T | None:
    """Use JSON-mode strict validation, including wire enum and timestamp strings."""
    try:
        return model.model_validate_json(json.dumps(value), strict=True)
    except (ValidationError, TypeError, ValueError):
        return None


def _json_model(value: object) -> JsonValue:
    """Encode only validated models; preserve their protocol aliases and omit absent fields."""
    if not isinstance(value, BaseModel):
        raise TypeError("not a wire model")
    return TypeAdapter(JsonValue).validate_python(
        value.model_dump(mode="json", by_alias=True, exclude_none=True)
    )


def _save(store: Path, held: Held) -> None:
    store.write_text(json.dumps(held, default=_json_model), encoding="utf-8")


def _malformed(message: str) -> dict[str, object]:
    return {"error": {"kind": "malformed", "message": message}}


def _page[T](items: list[T], page: PageRequest) -> dict[str, object]:
    end = min(page.offset + page.limit, len(items))
    return {"items": items[page.offset : end], "next": str(end) if end < len(items) else None}


def _shown(task: Task) -> Task:
    # This peer issues no change tokens; a caller must compare its fresh fields instead.
    shown = task.copy()
    shown.pop("version", None)
    return shown


def _valid_task(task: object) -> bool:
    """Validate every member the peer reads or returns before trusting its persisted state."""
    if not isinstance(task, dict) or _decode(WireTask, task) is None:
        return False
    nullable = ("content", "project", "url", "created_at", "updated_at")
    if not (
        isinstance(task.get("id"), str)
        and isinstance(task.get("title"), str)
        and all(key in task and isinstance(task[key], (str, type(None))) for key in nullable)
    ):
        return False
    status = task.get("status")
    if not isinstance(status, dict) or not isinstance(status.get("name"), str):
        return False
    try:
        category = StatusCategory(status.get("category"))
    except (ValueError, TypeError):
        return False
    labels = task.get("labels")
    if not isinstance(labels, list) or not all(
        isinstance(label, dict)
        and isinstance(label.get("id"), str)
        and isinstance(label.get("name"), str)
        and isinstance(label.get("color"), (str, type(None)))
        for label in labels
    ):
        return False
    if "location" not in task:
        return False
    location = task["location"]
    if location is not None and not (
        isinstance(location, dict)
        and len(location) == 1
        and (
            (set(location) == {"url"} and isinstance(location["url"], str))
            or (
                set(location) == {"path"}
                and isinstance(location["path"], str)
                and Path(location["path"]).is_absolute()
            )
        )
    ):
        return False
    status["category"] = category
    return True


def _held(store: Path) -> Held:
    """Refuse malformed persisted work before it can be returned as a protocol record."""
    try:
        held = json.loads(store.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        sys.exit(f"{KIND}: not this peer's state")
    match held:
        case {"requests": int(requests), "tasks": list(tasks)} if (
            type(requests) is int and requests >= 0 and all(_valid_task(task) for task in tasks)
        ):
            pass
        case _:
            sys.exit(f"{KIND}: not this peer's state")
    edges = held.get("edges", {})
    index = held.get("index", {})
    if (
        not isinstance(edges, dict)
        or not isinstance(index, dict)
        or not all(isinstance(key, str) for key in edges)
        or not all(isinstance(key, str) for key in index)
    ):
        sys.exit(f"{KIND}: not this peer's state")
    try:
        held["edges"] = {
            key: TypeAdapter(list[WireEdge]).validate_json(json.dumps(value), strict=True)
            for key, value in edges.items()
        }
        held["index"] = {
            key: SearchText.model_validate_json(json.dumps(value), strict=True)
            for key, value in index.items()
        }
    except (ValidationError, TypeError, ValueError):
        sys.exit(f"{KIND}: not this peer's state")
    for task in tasks:
        held["index"].setdefault(
            task["id"], SearchText(title=task["title"], content=task["content"])
        )
    # Every known task field was checked by the generated wire model and the stricter
    # persisted-state checks above. Opaque extra JSON fields are retained, not interpreted.
    return cast(Held, held)


def _matches(text: SearchText, query: TextQuery | None) -> bool:
    if query is None:
        return True
    wanted = query.terms.lower()
    title = text.title.lower()
    content = (text.content or "").lower()
    match query.fields:
        case "title":
            return wanted in title
        case "content":
            return wanted in content
        case _:
            return wanted in title or wanted in content


def _answer(store: Path, method: str, params: dict[str, object]) -> dict[str, object]:
    held = _held(store)
    tasks = held["tasks"]
    if method != "initialize":
        held["requests"] = held["requests"] + 1
        _save(store, held)
    match method:
        case "initialize":
            return {
                "result": {
                    "protocol_version": PROTOCOL_VERSION,
                    "kind": KIND,
                    "capabilities": CAPABILITIES,
                    "writes": "supported",
                    "meters": True,
                }
            }
        case "health":
            return {"result": {"reachable": True, "detail": f"{len(tasks)} task(s)"}}
        case "metering":
            return {"result": {"metering": {"requests": held["requests"], "budgets": []}}}
        case "get_task":
            if not isinstance(params.get("id"), str):
                return {"error": {"kind": "malformed", "message": "the task id is a string"}}
            found = [task for task in tasks if task["id"] == params.get("id")]
            return {"result": {"task": _shown(found[0]) if found else None}}
        case "query_tasks":
            page = _decode(PageRequest, params.get("page"))
            if page is None:
                return _malformed("a page carries a cursor and a positive bounded limit")
            query = params.get("query")
            if not isinstance(query, dict):
                return _malformed("a query is an object")
            text = None if query.get("text") is None else _decode(TextQuery, query["text"])
            if query.get("text") is not None and text is None:
                return _malformed("search terms and fields follow the text query contract")
            matched = [_shown(task) for task in tasks if _matches(held["index"][task["id"]], text)]
            return {"result": _page(matched, page)}
        case "task_comments" | "labels" | "task_dependencies":
            page = _decode(PageRequest, params.get("page"))
            if page is None:
                return _malformed("a page carries a cursor and a positive bounded limit")
            if method == "labels":
                labels = {label["id"]: label for task in tasks for label in task["labels"]}
                return {"result": _page(list(labels.values()), page)}
            identifier = params.get("task" if method == "task_comments" else "id")
            if not isinstance(identifier, str):
                return _malformed("the task id is a string")
            if method == "task_comments":
                exists = any(task["id"] == identifier for task in tasks)
                return {"result": {"page": _page([], page) if exists else None}}
            direction = params.get("direction")
            if direction not in get_args(DependencyDirection):
                return _malformed("the dependency direction follows protocol section 4.7")
            edges = (
                held["edges"].get(identifier, [])
                if direction == "depends-on"
                else [
                    edge
                    for grouped in held["edges"].values()
                    for edge in grouped
                    if edge.to.id.root == identifier
                ]
            )
            return {"result": _page(edges, page)}
        case "write_task":
            write = params.get("write")
            if not isinstance(write, dict) or not isinstance(write.get("item"), dict):
                return _malformed("write_task carries a whole task")
            item = write["item"]
            parsed = _decode(WireTask, item)
            if parsed is None:
                return _malformed("the whole task must follow its wire contract")
            try:
                edges = TypeAdapter(list[WireEdge]).validate_json(
                    json.dumps(write.get("depends_on", [])), strict=True
                )
            except (ValidationError, TypeError, ValueError):
                return _malformed("dependency edges must follow their wire contract")
            target = write.get("target", parsed.id.root)
            if target is None:
                target = parsed.id.root
            if not isinstance(target, str):
                return _malformed("a write's target is an id")
            defaults = parsed.model_dump(mode="json")
            written = dict(item)
            for field in ("content", "project", "url", "location", "created_at", "updated_at"):
                written.setdefault(field, defaults[field])
            # A destination ignores the source's read-only key and change token (§4.4).
            written.pop("key", None)
            written.pop("version", None)
            old = next((task for task in tasks if task["id"] == target), None)
            if old is not None and "key" in old:
                written["key"] = old["key"]
            written["id"] = target
            if not _valid_task(written):
                return _malformed("the stored task must follow its wire contract")
            # Validated above, preserving every provided work field rather than rebuilding
            # a selected subset; the cast only connects decoded JSON to the peer's record.
            record = cast(Task, written)
            held["tasks"] = [record if task["id"] == target else task for task in tasks]
            if old is None:
                held["tasks"].append(record)
            held["edges"][target] = [
                WireEdge.model_validate_json(
                    json.dumps(
                        {
                            "from": {"id": target, "kind": "task"},
                            "to": _json_model(edge.to),
                            "kind": edge.kind.value,
                        }
                    ),
                    strict=True,
                )
                for edge in edges
            ]
            held["index"].setdefault(
                target, SearchText(title=record["title"], content=record["content"])
            )
            _save(store, held)
            return {"result": {"id": target}}
        case _:
            return {
                "error": {
                    "kind": "malformed",
                    "message": f"protocol version {PROTOCOL_VERSION} has no method {method!r}",
                }
            }


def _main() -> None:
    store = Path(sys.argv[1])
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            request = json.loads(line)
        except ValueError:
            print(f"{KIND}: ignoring a line that is not JSON", file=sys.stderr)
            continue
        if (
            not isinstance(request, dict)
            or not isinstance(request.get("id"), str)
            or not isinstance(request.get("method"), str)
        ):
            print(f"{KIND}: ignoring a line that is not a request", file=sys.stderr)
            continue
        params = request.get("params")
        if params is not None and not isinstance(params, dict):
            answer = _malformed("request params are an object")
        else:
            answer = _answer(store, request["method"], params or {})
        sys.stdout.write(json.dumps({"id": request["id"], **answer}, default=_json_model) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    _main()
