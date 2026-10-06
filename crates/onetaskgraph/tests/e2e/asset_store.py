"""A file-backed source that serves image assets at URLs, spoken to over `docs/plugin-protocol.md`.

It is the out-of-process destination the asset journeys copy into: a peer that keeps its
tasks, projects and documents in a JSON file, so what one invocation writes the next one
reads, and that declares `assets` `"native"` and serves each asset at a URL derived from the
bytes it received — exactly what §4.9a asks of a hosted backend. It is written in Python, as
`document_store.py` beside it is and for that file's reasons: it shares not one line with the
engine's own half of the protocol, so the journeys that drive it test the claim that a plugin
can be written from the protocol document alone.

Its settings, handed over in the `initialize` request (§3), are
`{"store": <path>, "log": <path>, "assets": "native"}`. With `assets` absent the handshake says
nothing about assets, which is a plugin written before there were any. `log` receives one JSON
line per write this source is sent — what arrived and what it answered — which is how a
journey proves which bytes reached the plugin, and that a refused copy sent it nothing.
"""

# llmlint: ignore-file[modern_domain_modeling] This peer is a transcription of
# docs/plugin-protocol.md, and the protocol's own types are JSON objects. It is spawned with a
# cleared environment (§3.1) by whatever `python3` or `python` the host provides, so it may
# import nothing outside the standard library and cannot assume a version with the typing
# this rule asks for; `document_store.py` beside it records the same reasoning at length.

import base64
import binascii
import hashlib
import json
import os
import re
import sys

KIND = "asset-store"
PROTOCOL_VERSION = 2
MAX_PAGE_SIZE = 50
KINDS = {"task": "tasks", "project": "projects", "document": "documents"}
ASSETS_KEY = "onetaskgraph.assets"
TEMPLATE_KEY = "onetaskgraph.template"


class Refusal(Exception):
    """A refusal this source answers one request with, carrying the contract's own shape."""

    def __init__(self, error):
        super().__init__(error["message"])
        self.error = error


def refused(message):
    return Refusal({"kind": "refused", "message": message})


def malformed(message):
    return Refusal({"kind": "malformed", "message": message})


def read_store(path):
    """Every held item by kind, a store not written yet being an empty one."""
    try:
        with open(path, encoding="utf-8") as handle:
            held = json.load(handle)
    except FileNotFoundError:
        return {name: [] for name in KINDS.values()}
    except ValueError:
        raise malformed("%s is not JSON, so it is not this source's store" % path)
    if not isinstance(held, dict) or not all(
        isinstance(held.get(name, []), list) for name in KINDS.values()
    ):
        raise malformed("%s is not a store of tasks, projects and documents" % path)
    for name in KINDS.values():
        for item in held.get(name, []):
            checked_item(item, "%s's %s" % (path, name))
    return {name: held.get(name, []) for name in KINDS.values()}


def checked_item(item, what):
    """An item as this source holds it: an object with a string `id`, and a string or null
    `content` and an object or null `metadata`, the three members this source reads."""
    if (
        not isinstance(item, dict)
        or not isinstance(item.get("id"), str)
        or not isinstance(item.get("content", None), (str, type(None)))
        or not isinstance(item.get("metadata", None), (dict, type(None)))
    ):
        raise malformed(
            "%s holds an item that is not an object with a string id, a string or null "
            "content and an object or null metadata" % what
        )
    return item


def write_store(path, held):
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(held, handle, indent=2)


def log(settings, entry):
    path = settings.get("log")
    if path is None:
        return
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(entry) + "\n")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def served_at(digest, name):
    """The URL this source serves bytes with SHA-256 `digest` at: derived from the bytes."""
    return "https://assets.example.invalid/%s/%s" % (digest, name)


def reference(name):
    """An asset reference to `name`: a Markdown image whose target is `./<name>`."""
    return re.compile(r"(!\[[^\]\n]*\]\()\./" + re.escape(name) + r"(?=[\s)])")


def store_assets(item, payloads, recorded):
    """Serve each asset of a write and point the item at them, as §4.9a asks.

    Answers what was received of each asset, for the log: its name, content type and the
    SHA-256 of the bytes as decoded here, or `None` for an asset sent without bytes.
    """
    if not isinstance(payloads, list):
        raise malformed("`assets` must be a list")
    if recorded is not None and not isinstance(recorded, dict):
        raise malformed("`recorded_assets` must be an object")
    uploads = {}
    received = []
    for payload in payloads:
        if not isinstance(payload, dict) or not all(
            isinstance(payload.get(member), str) for member in ("name", "sha256", "content_type")
        ):
            raise malformed("an asset is {name, sha256, content_type, bytes}")
        name = payload["name"]
        if "bytes" in payload:
            try:
                data = base64.b64decode(payload["bytes"], validate=True)
            except (binascii.Error, TypeError, ValueError):
                raise malformed("the asset %s's bytes are not base64" % name)
            digest = sha256(data)
            if digest != payload["sha256"]:
                raise refused(
                    "the asset %s's bytes do not hash to the sha256 %s it carries"
                    % (name, payload["sha256"])
                )
            uploads[name] = {"sha256": digest, "url": served_at(digest, name)}
            received.append(
                {
                    "name": name,
                    "content_type": payload["content_type"],
                    "decoded_sha256": digest,
                }
            )
        else:
            held = (recorded or {}).get(name)
            if (
                not isinstance(held, dict)
                or held.get("sha256") != payload["sha256"]
                or not isinstance(held.get("url"), str)
            ):
                raise refused(
                    "the asset %s carries no bytes and nothing records an upload of it with "
                    "sha256 %s" % (name, payload["sha256"])
                )
            uploads[name] = {"sha256": held["sha256"], "url": held["url"]}
            received.append(
                {"name": name, "content_type": payload["content_type"], "decoded_sha256": None}
            )
    content = item.get("content") or ""
    rewritten = content
    for name, upload in uploads.items():
        rewritten = reference(name).sub(lambda found: found.group(1) + upload["url"], rewritten)
    metadata = dict(item.get("metadata") or {})
    entry = metadata.get(TEMPLATE_KEY)
    if (
        rewritten != content
        and isinstance(entry, dict)
        and entry.get("body_digest") == "sha256:" + sha256(content.encode("utf-8"))
    ):
        entry = dict(entry)
        entry["body_digest"] = "sha256:" + sha256(rewritten.encode("utf-8"))
        metadata[TEMPLATE_KEY] = entry
    if uploads:
        metadata[ASSETS_KEY] = uploads
    else:
        metadata.pop(ASSETS_KEY, None)
    if item.get("content") is not None:
        item["content"] = rewritten
    item["metadata"] = metadata
    return received


def unused(items, wanted):
    taken = {item["id"] for item in items}
    if wanted not in taken:
        return wanted
    attempt = 2
    while "%s-%d" % (wanted, attempt) in taken:
        attempt += 1
    return "%s-%d" % (wanted, attempt)


def write(settings, kind, params):
    """`write_task`, `write_project` or `write_document`, with §4.9a's assets when present."""
    written = params.get("write")
    if not isinstance(written, dict) or not isinstance(written.get("item"), dict):
        raise malformed("write_%s needs a `write` carrying an `item`" % kind)
    target = written.get("target")
    if target is not None and not isinstance(target, str):
        raise malformed("write_%s's target must be a native id or null" % kind)
    item = dict(checked_item(written["item"], "write_%s's item" % kind))
    carrying = "assets" in params
    entry = {
        "method": "write_" + kind,
        "target": target,
        "members": sorted(params.keys()),
        "recorded_assets": params.get("recorded_assets"),
    }
    if carrying:
        if settings.get("assets") != "native":
            log(settings, entry)
            raise refused("the %s plugin cannot store image assets" % KIND)
        entry["received"] = store_assets(item, params["assets"], params.get("recorded_assets"))
    store = settings["store"]
    held = read_store(store)
    items = held[KINDS[kind]]
    if target is None:
        item["id"] = unused(items, item.get("id") or kind)
        items.append(item)
    else:
        at = [index for index, kept in enumerate(items) if kept["id"] == target]
        if not at:
            raise refused("%s names no %s this source holds" % (target, kind))
        item["id"] = target
        items[at[0]] = item
    write_store(store, held)
    answer = {"id": item["id"]}
    if carrying:
        answer["content"] = item.get("content")
    entry["answered"] = answer
    log(settings, entry)
    return answer


def capabilities(settings):
    declared = {
        "projects": "native",
        "documents": "native",
        "orphan_tasks": "unsupported",
        "filter_by_label": "unsupported",
        "filter_by_status": "unsupported",
        "search_title": "unsupported",
        "search_content": "unsupported",
        "task_dependencies": "forward-only",
        "project_dependencies": "forward-only",
        "max_page_size": MAX_PAGE_SIZE,
    }
    if settings.get("assets") == "native":
        declared["assets"] = "native"
    return declared


def page_of(items, page):
    """One page of `items`: this source applies no predicate, so it answers the wider set."""
    if not isinstance(page, dict):
        raise malformed("a paged method needs a `page` object")
    cursor = page.get("cursor")
    limit = page.get("limit")
    if cursor is not None and not (isinstance(cursor, str) and cursor.isdigit()):
        raise malformed("cursor %r was not issued by this source" % (cursor,))
    if not isinstance(limit, int) or isinstance(limit, bool) or limit < 1:
        raise malformed("a page limit is a positive integer")
    start = int(cursor) if cursor is not None else 0
    end = min(start + min(limit, MAX_PAGE_SIZE), len(items))
    return {"items": items[start:end], "next": str(end) if end < len(items) else None}


# llmlint: ignore-block[structural_pattern_matching] `match`/`case` is a syntax error before
# Python 3.10, and this peer is spawned by whichever interpreter the host has; see
# `document_store.py`, which records the same reason.
def dispatch(settings, method, params):
    store = settings["store"]
    if method == "health":
        return {"reachable": True, "detail": "a file-backed asset store"}
    for kind in KINDS:
        if method == "get_" + kind:
            wanted = params.get("id")
            found = [item for item in read_store(store)[KINDS[kind]] if item["id"] == wanted]
            return {kind: found[0] if found else None}
    if method == "query_tasks":
        return page_of(read_store(store)["tasks"], params.get("page"))
    if method == "query_projects":
        return page_of(read_store(store)["projects"], params.get("page"))
    if method == "query_documents":
        return page_of(read_store(store)["documents"], params.get("page"))
    if method in ("labels", "task_dependencies", "project_dependencies"):
        return {"items": [], "next": None}
    for kind in KINDS:
        if method == "write_" + kind:
            return write(settings, kind, params)
        if method == "delete_" + kind:
            held = read_store(store)
            held[KINDS[kind]] = [
                item for item in held[KINDS[kind]] if item["id"] != params.get("id")
            ]
            write_store(store, held)
            log(settings, {"method": method, "target": params.get("id")})
            return {}
    raise malformed("protocol version %d has no method called %r" % (PROTOCOL_VERSION, method))


# llmlint: ignore-end[structural_pattern_matching]
def initialize(params):
    version = params.get("protocol_version")
    if version != PROTOCOL_VERSION:
        raise Refusal(
            {
                "kind": "config",
                "message": "protocol version %s is not supported by this plugin; it speaks "
                "version %d" % (version, PROTOCOL_VERSION),
            }
        )
    settings = params.get("config") or {}
    if (
        not isinstance(settings, dict)
        or not isinstance(settings.get("store"), str)
        or not isinstance(settings.get("log", ""), str)
        or settings.get("assets", "native") != "native"
    ):
        raise Refusal(
            {
                "kind": "config",
                "message": 'this source\'s settings are {"store": <path>, "log": <path>, '
                '"assets": "native"}, the last two optional',
            }
        )
    return settings, {
        "protocol_version": PROTOCOL_VERSION,
        "kind": KIND,
        "capabilities": capabilities(settings),
        "writes": "supported",
    }


def main():
    settings = None
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            request = json.loads(line)
            identifier = request["id"]
        except (ValueError, KeyError, TypeError):
            print("%s: ignoring an unaddressed line" % KIND, file=sys.stderr)
            continue
        if not isinstance(identifier, str):
            print("%s: ignoring a line whose id is not a string" % KIND, file=sys.stderr)
            continue
        method = request.get("method")
        params = request.get("params")
        try:
            if not isinstance(method, str):
                raise malformed("a request names its method as a string")
            if not isinstance(params, dict):
                raise malformed("a request carries an object `params`")
            if method == "initialize":
                settings, result = initialize(params)
            elif settings is None:
                raise malformed("%s arrived before the handshake" % method)
            else:
                result = dispatch(settings, method, params)
            answer = {"id": identifier, "result": result}
        except Refusal as refusal:
            answer = {"id": identifier, "error": refusal.error}
        sys.stdout.write(json.dumps(answer) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
