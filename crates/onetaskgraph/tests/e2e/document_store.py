"""A file-backed document source, spoken to over `docs/plugin-protocol.md`.

It exists for one reason the suite could not otherwise satisfy. A copy is only proven by
reading the destination back *afterwards*, and when this was written every destination this
build had was either a folder of Markdown — which declared it has no documents — or the
in-memory source, whose work dies with the process that held it. One command-line
invocation is one process, so a document copy driven the way a user drives it had nothing
left to look at. This keeps its documents in a JSON file, so what one invocation writes the
next one reads. `local-md` has since grown a document side of its own and is a second such
destination; this peer is not thereby redundant, because it is the one destination written
against the protocol document rather than out of this engine.

It is written in Python on purpose, and not only to keep a fixture out of the shipped
binary. The stdio seam's whole claim is that a plugin can be written in another language
against the protocol document alone; this peer shares not one line with the engine's own
half, so the journeys that drive it test that claim rather than restating the engine's
implementation of it back to itself. `python3` is already what every guard under
`workspace:lint` runs on all three platforms, so it costs the gate no new dependency.

Its settings, handed over in the `initialize` request (§3), are
`{"store": <path>, "documents": "native"|"unsupported", "log": <path>}` — the last two
optional. `documents` lets a journey configure a peer that says it has none, and `log`
appends the name of every method this source is asked for, which is how a journey proves a
refusal happened *before* anything was read.
"""

# llmlint: ignore-file[modern_domain_modeling] This peer is a transcription of
# docs/plugin-protocol.md, and the protocol's own types are JSON objects. It is spawned
# with a cleared environment (§3.1) by whatever `python3` or `python` the host provides, so
# it may import nothing outside the standard library and cannot assume a version that has
# the typing this rule asks for. A stdlib model layer would also be erased at runtime,
# adding no check that `parameter`, the `checked_*` family, `read_store` and `initialize`
# below do not already make — while costing the file the property the journeys rely on:
# that a plugin can be written from the protocol document alone, sharing no type with the
# engine's own half.

import json
import os
import sys

KIND = "document-store"
PROTOCOL_VERSION = 2

# The largest page this source will serve, which it also declares at the handshake.
MAX_PAGE_SIZE = 50

# The `TextFields` members a `TextQuery` may name (§4.5).
TEXT_FIELDS = ("title", "content", "title-or-content")


def refused(message):
    """The contract's `SourceError::Refused`, in its serialized shape."""
    return {"kind": "refused", "message": message}


def malformed(message):
    """The contract's `SourceError::Malformed`."""
    return {"kind": "malformed", "message": message}


def a_string(value):
    """Whether a value is the JSON string the protocol asks for."""
    return isinstance(value, str)


def an_integer(value):
    """Whether a value is a JSON integer.

    `True` is an `int` in Python and is not one here: a limit of `true` is a request this
    source cannot serve, and silently reading it as `1` would answer a different question
    from the one that was asked.
    """
    return isinstance(value, int) and not isinstance(value, bool)


def checked(condition, message):
    """Refuse as `malformed` unless a protocol shape holds.

    Every boundary value below is read through this rather than indexed directly. A peer
    that raises a Python error into the pipe is a *dead* peer to the engine, which can then
    say only that the plugin closed its output — while `malformed` (§5) names what arrived
    and reaches the user as this source's own words.
    """
    if not condition:
        raise Refusal(malformed(message))


def checked_string(value, what):
    """`value` when it is a string, and a refusal naming `what` when it is not."""
    checked(a_string(value), "%s must be a string" % what)
    return value


def checked_optional_string(value, what):
    """`value` when it is a string or null — the shape every optional text member has."""
    checked(value is None or a_string(value), "%s must be a string or null" % what)
    return value


def checked_list(value, what):
    """An optional list member: absent or null is empty, and anything else is a list.

    Spelled out rather than defaulted with `or`, and that is the whole of why it exists:
    `[]`, `{}`, `0`, `""` and `false` are all falsey in Python, so `value.get(name) or []`
    would read `"labels": false` as "no labels" — accepting a shape the protocol does not
    have, at the one place that exists to refuse one.
    """
    if value is None:
        return []
    checked(isinstance(value, list), "%s must be a list" % what)
    return value


def checked_object(value, what):
    """An optional object member, on `checked_list`'s terms and for its reason."""
    if value is None:
        return {}
    checked(isinstance(value, dict), "%s must be an object" % what)
    return value


def checked_location(location, what):
    """A `Location` (§4.13): absent, null, or an object with exactly one of two keys.

    A consumer tells the two apart by which key is present, so an object carrying both —
    or neither — is not a location this interface can represent, and saying so is what
    keeps that reading true of everything this source hands out.
    """
    if location is None:
        return None
    checked(
        isinstance(location, dict)
        and len(location) == 1
        and ("url" in location or "path" in location),
        '%s\'s location must be {"url": <link>} or {"path": <absolute path>}' % what,
    )
    key = "url" if "url" in location else "path"
    checked_string(location[key], "%s's location %s" % (what, key))
    return location


def checked_document(value, what):
    """One `Document` (§4.11), refused as malformed unless every member is its own shape.

    Read on the way *in* from a write and on the way *out* of the store, because both are
    this source's boundary: a store file somebody edited by hand is as much untrusted input
    as a request line is, and a half-shaped document handed back would report a defect of
    this peer as a defect of the engine reading it. No status and no dependencies are
    validated because a document carries neither — a store holding one is holding a member
    this source never wrote and never reads.
    """
    checked(isinstance(value, dict), "%s must be an object" % what)
    checked_string(value.get("id"), "%s needs an id that" % what)
    checked_string(value.get("title"), "%s needs a title that" % what)
    for member in ("content", "project", "url", "created_at", "updated_at"):
        checked_optional_string(value.get(member), "%s's %s" % (what, member))
    for label in checked_list(value.get("labels"), "%s's labels" % what):
        checked(isinstance(label, dict), "%s's labels must each be an object" % what)
        checked_string(label.get("id"), "%s's label id" % what)
        checked_string(label.get("name"), "%s's label name" % what)
    checked_location(value.get("location"), what)
    checked_object(value.get("metadata"), "%s's metadata" % what)
    for origin in checked_list(value.get("repositories"), "%s's repositories" % what):
        checked_string(origin, "%s's repository origin" % what)
    return value


def checked_query(query, method):
    """A `DocumentQuery` (§4.11), whole, before any predicate of it narrows anything.

    There is no `statuses` member and this does not invent one: a document is not work, so
    a status filter would have nothing to compare against.
    """
    text = query.get("text")
    if text is not None:
        checked(isinstance(text, dict), "%s's text query must be an object or null" % method)
        checked_string(text.get("terms"), "%s's search terms" % method)
        checked(
            text.get("fields") in TEXT_FIELDS,
            "%s's search fields must be one of %s" % (method, ", ".join(TEXT_FIELDS)),
        )
    labels = checked_object(query.get("labels"), "%s's label filter" % method)
    for member in ("any_of", "all_of", "none_of"):
        names = checked_list(labels.get(member), "%s's %s label filter" % (method, member))
        for name in names:
            checked_string(name, "%s's %s label name" % (method, member))
    project = query.get("project", "any")
    checked(
        project in ("any", "orphans")
        or (isinstance(project, dict) and a_string(project.get("is"))),
        '%s\'s project filter must be "any", "orphans" or {"is": <native id>}' % method,
    )
    return query


def checked_page(page, method):
    """A `PageRequest` (§4.1): an optional opaque cursor, and a limit that is a number.

    Whether the limit is one this source will *serve*, and whether the cursor is one it
    ever *issued*, are `paginate`'s to answer — this is only the shape, and it runs first
    so that neither of those questions is asked of a value that is not one.
    """
    cursor = page.get("cursor")
    checked(
        cursor is None or a_string(cursor),
        "%s's page cursor must be a string or null" % method,
    )
    checked(
        an_integer(page.get("limit", 0)),
        "%s's page limit must be an integer" % method,
    )
    return page


def checked_write(write, method):
    """An `ItemWrite` (§4.12) whose item is a `Document`, as the target and the item."""
    target = write.get("target")
    checked(
        target is None or a_string(target),
        "%s's write target must be a native id or null" % method,
    )
    return target, checked_document(write.get("item"), "%s's item" % method)


def read_store(path):
    """The documents on disk, treating a file that is not there yet as an empty store.

    A missing file is the ordinary first-run state — a destination nothing has been copied
    into yet — rather than a failure. A file that is there and is not a store is a
    different thing, and says so rather than raising a Python error into the pipe. So is a
    store whose entries are not documents: this is where they re-enter the peer, so it is
    where their shape is established rather than assumed by every reader below.
    """
    try:
        with open(path, encoding="utf-8") as handle:
            held = json.load(handle)
    except FileNotFoundError:
        return []
    except ValueError:
        raise Refusal(malformed("%s is not JSON, so it is not this source's store" % path))
    documents = held.get("documents", []) if isinstance(held, dict) else None
    if not isinstance(documents, list):
        raise Refusal(malformed('%s is not a store: expected {"documents": [...]}' % path))
    for index, document in enumerate(documents):
        checked_document(document, "%s document %d" % (path, index))
    return documents


def write_store(path, documents):
    """Write the documents back, creating the directory they live in."""
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        json.dump({"documents": documents}, handle, indent=2)


def note(settings, method):
    """Record that this source was asked for `method`, when its settings ask for a record."""
    path = settings.get("log")
    if path is None:
        return
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(method + "\n")


def capabilities(settings):
    """What this source declares once, at the handshake.

    No projects of its own, and it says so: the engine then reports that predicate
    unavailable for this source rather than reading an empty page as a narrowed answer.
    """
    return {
        "projects": "unsupported",
        "documents": settings.get("documents", "native"),
        "orphan_tasks": "native",
        "filter_by_label": "native",
        "filter_by_status": "native",
        "search_title": "native",
        "search_content": "native",
        "task_dependencies": "both-directions",
        "project_dependencies": "both-directions",
        "max_page_size": MAX_PAGE_SIZE,
    }


def has_documents(settings):
    """Whether this source has documents at all, as its settings declared."""
    return settings.get("documents", "native") == "native"


def matches_text(document, query):
    """Whether one document matches a free-text query, over the fields it names.

    Both sides arrive established: the query through `checked_query` and the document
    through `checked_document`, so nothing indexed here can be a shape this source never
    agreed to read.
    """
    if query is None:
        return True
    terms = query["terms"].lower()
    in_title = terms in (document.get("title") or "").lower()
    in_content = terms in (document.get("content") or "").lower()
    fields = query["fields"]
    if fields == "title":
        return in_title
    if fields == "content":
        return in_content
    return in_title or in_content


def survives(document, query):
    """Whether one document survives every predicate a query carries."""
    held = [label["name"].lower() for label in document.get("labels") or []]
    labels = query.get("labels") or {}
    any_of = labels.get("any_of") or []
    if any_of and not any(name.lower() in held for name in any_of):
        return False
    if not all(name.lower() in held for name in labels.get("all_of") or []):
        return False
    if any(name.lower() in held for name in labels.get("none_of") or []):
        return False
    project = query.get("project", "any")
    if project == "orphans" and document.get("project") is not None:
        return False
    if isinstance(project, dict) and document.get("project") != project.get("is"):
        return False
    return matches_text(document, query.get("text"))


def paginate(items, page):
    """Slice `items` into the page asked for, refusing a cursor this source never issued.

    Its `page` has been through `checked_page`, so a cursor here is a string and a limit is
    a number: what is left to decide is whether they name a page this source can serve.
    """
    cursor = page.get("cursor")
    start = 0
    if cursor is not None:
        if not cursor.isdigit() or int(cursor) >= len(items):
            raise Refusal(
                malformed(
                    "cursor %r was not issued by this source; it addresses no row of the %d "
                    "result(s) available" % (cursor, len(items))
                )
            )
        start = int(cursor)
    limit = page.get("limit", 0)
    if limit < 1:
        raise Refusal(
            {
                "kind": "config",
                "message": "a page limit of 0 is not a page; ask for at least 1 row",
            }
        )
    end = min(start + min(limit, MAX_PAGE_SIZE), len(items))
    return {
        "items": items[start:end],
        "next": str(end) if end < len(items) else None,
    }


def unused(documents, wanted):
    """`wanted` when nothing holds it, or the first `wanted-N` that is free.

    A destination decides its own ids, exactly as every other writable source does: the id
    an item was read under at its source is a suggestion.
    """
    taken = {document["id"] for document in documents}
    if wanted not in taken:
        return wanted
    attempt = 2
    while "%s-%d" % (wanted, attempt) in taken:
        attempt += 1
    return "%s-%d" % (wanted, attempt)


class Refusal(Exception):
    """A refusal this source answers one request with, carrying the contract's own shape."""

    def __init__(self, error):
        super().__init__(error["message"])
        self.error = error


def parameter(params, name, kind, method):
    """One request parameter, refused as `malformed` when it is absent or the wrong shape.

    A plugin answers a request it cannot parse with `malformed` (§5) rather than failing in
    a way the engine has to guess at, so this names the method and the parameter instead of
    raising a Python error into the pipe — where the engine would see only a dead peer.
    """
    value = params.get(name) if isinstance(params, dict) else None
    if not isinstance(value, kind):
        raise Refusal(
            malformed(
                "%s needs a %s parameter %r" % (method, kind.__name__, name)
            )
        )
    return value


def document_side(settings):
    """Refuse every document call when this source's settings say it has none.

    In the same words `documentless` gives every other document-free source, because a
    caller must not be able to tell which plugin refused apart from the kind it names.
    """
    if not has_documents(settings):
        raise Refusal(refused("the %s plugin has no documents" % KIND))


# llmlint: ignore-block[structural_pattern_matching] `match`/`case` is a syntax error
# before Python 3.10, and this peer is spawned with a cleared environment by whichever
# `python3` or `python` the host has on PATH — see `interpreter()` in document_store.rs.
# A syntax error there aborts the interpreter at import, which the engine sees as a plugin
# that closed its output rather than as a portability problem, so the whole document-copy
# round trip would report as broken on any host still shipping 3.9.
def dispatch(settings, method, params):
    """Answer one method against the store this source's settings name."""
    store = settings["store"]
    if method == "health":
        return {
            "reachable": True,
            "detail": "%d document(s) on disk" % len(read_store(store)),
        }
    # This source holds no tasks and no projects, and an empty page is the whole truth:
    # `projects: unsupported` at the handshake is what keeps that honest.
    if method in ("get_task", "get_project"):
        return {method.removeprefix("get_"): None}
    if method in (
        "query_tasks",
        "query_projects",
        "labels",
        "task_dependencies",
        "project_dependencies",
    ):
        return {"items": [], "next": None}
    if method == "get_document":
        document_side(settings)
        wanted = parameter(params, "id", str, method)
        found = [d for d in read_store(store) if d["id"] == wanted]
        return {"document": found[0] if found else None}
    if method == "query_documents":
        document_side(settings)
        query = checked_query(parameter(params, "query", dict, method), method)
        page = checked_page(parameter(params, "page", dict, method), method)
        kept = [d for d in read_store(store) if survives(d, query)]
        return paginate(kept, page)
    if method == "write_document":
        document_side(settings)
        # The whole write is established before the store is opened, so a request this
        # source cannot represent leaves the store exactly as it found it.
        target, item = checked_write(parameter(params, "write", dict, method), method)
        documents = read_store(store)
        landing = dict(item)
        if target is None:
            landing["id"] = unused(documents, landing["id"])
            documents.append(landing)
        else:
            at = [i for i, d in enumerate(documents) if d["id"] == target]
            if not at:
                raise Refusal(
                    refused(
                        "%s names no document this source holds; next: copy with --recreate "
                        "to create one instead of updating" % target
                    )
                )
            landing["id"] = target
            documents[at[0]] = landing
        write_store(store, documents)
        return {"id": landing["id"]}
    if method == "delete_document":
        document_side(settings)
        unwanted = parameter(params, "id", str, method)
        documents = [d for d in read_store(store) if d["id"] != unwanted]
        write_store(store, documents)
        return {}
    if method in ("write_task", "write_project", "delete_task", "delete_project"):
        raise Refusal(refused("the %s plugin cannot be written" % KIND))
    raise Refusal(
        malformed("protocol version %d has no method called %r" % (PROTOCOL_VERSION, method))
    )


# llmlint: ignore-end[structural_pattern_matching]
def initialize(params):
    """The handshake (§3), which is also where this source learns where its store is."""
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
    # Every setting is read here, once, rather than trusted wherever it is later used: a
    # peer configured with a path that is not one, or a capability value the contract has
    # no member for, is a configuration mistake and says so at the handshake.
    if (
        not isinstance(settings, dict)
        or not isinstance(settings.get("store"), str)
        or settings.get("documents", "native") not in ("native", "unsupported")
        or not isinstance(settings.get("log", ""), str)
    ):
        raise Refusal(
            {
                "kind": "config",
                "message": 'this source\'s settings must be {"store": <path>, "documents": '
                '"native"|"unsupported", "log": <path>}, the last two optional',
            }
        )
    note(settings, "initialize")
    return settings, {
        "protocol_version": PROTOCOL_VERSION,
        "kind": KIND,
        "capabilities": capabilities(settings),
        "writes": "supported",
    }


def main():
    """Serve one connection until the engine closes its input."""
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
        # An `id` is a string (§2) and a response echoes it, so a line addressed with
        # anything else has no address to answer at: complaining about it in an `error`
        # would mean inventing an id the engine is not waiting on. It goes to stderr with
        # the unaddressed lines, and this connection carries on.
        if not isinstance(identifier, str):
            print("%s: ignoring a line whose id is not a string" % KIND, file=sys.stderr)
            continue
        method = request.get("method", "")
        params = request.get("params")
        try:
            if not isinstance(method, str):
                raise Refusal(malformed("a request names its method as a string"))
            # `params` is present even when empty (§2), so an absent one is a request this
            # source cannot read rather than an empty one it should guess at.
            if not isinstance(params, dict):
                raise Refusal(
                    malformed("a request carries an object `params`, present even when empty")
                )
            if method == "initialize":
                settings, result = initialize(params)
            elif settings is None:
                raise Refusal(malformed("%s arrived before the handshake" % method))
            else:
                note(settings, method)
                result = dispatch(settings, method, params)
            answer = {"id": identifier, "result": result}
        except Refusal as refusal:
            answer = {"id": identifier, "error": refusal.error}
        sys.stdout.write(json.dumps(answer) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
