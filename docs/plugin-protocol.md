# The onetaskgraph stdio plugin protocol

A source that is not a Rust crate speaks this protocol. The engine spawns it as a
subprocess, writes one JSON request object per line to its standard input, and reads
one JSON response object per line from its standard output. Everything in
`onetaskgraph-plugin-api` has a shape here and nothing else does, so a plugin written
against this document alone — in any language — is a source this product can drive.

This document is normative and complete: the framing, the handshake, one method per
trait method with its request and response shapes, the error envelope, and what a
peer does when the other side speaks a version it does not know. Both halves of it
are implemented in `onetaskgraph-core`: `SubprocessSource` is the engine's side, and
`serve` is a reference plugin side that hosts any plugin of this build — which is
what the shipped `onetaskgraph-source` program runs, and what lets the shared
journeys run every one of themselves a second time over a real pipe. Where this
document and the trait disagree, that is a defect in one of them and is worth
reporting rather than reconciling by guesswork.

Every name below is a restatement of Rust, so none of them is prose alone:
`scripts/check-protocol-contract.sh` — a target in `just check` — reconciles them with
the contract crate. The method table in §4 and the error table in §5 are checked both
ways against `TaskSource`'s methods and `SourceError`'s variants, failing on a name
either side has and the other does not. The `Capabilities` fields and every serialized
value of the contract's enums are checked one way, because prose has no rows to read
back: a value Rust serializes and this document never spells is drift, and fails.

Throughout, **engine** is the side that spawns and asks, and **plugin** is the side
that is spawned and answers. Field names, and the JSON encoding of every contract
type, are exactly what `onetaskgraph schema` emits — that bundle is generated from
the same Rust types that cross this boundary, so it is the machine-readable companion
to this document rather than a second source of truth.

## 1. Framing

- One **request object** per line on the plugin's standard input.
- One **response object** per line on the plugin's standard output.
- A line is terminated by a single line feed (`U+000A`). A carriage return
  immediately before it is stripped by the reader, so a plugin written on Windows is
  not broken by its own runtime.
- Each line is one complete JSON value, encoded as UTF-8, containing **no unescaped
  line feed**. JSON's own escaping (`\n`) carries newlines inside strings, so a task
  body with paragraphs is one line on the wire.
- There is no length prefix and no framing beyond the newline.
- A line has a maximum length: **16 MiB**. A peer that writes more than that without
  ending the line has its connection closed, because a reader must hold a whole line
  before it can say anything about it, and an unbounded line lets the other side choose
  how much memory this one uses. No real page approaches it.
- Standard error is **diagnostics only**. The engine never parses it. A plugin may
  write anything there, and should write nothing on a successful call; the engine may
  surface it when a call fails or when a plugin exits unexpectedly.
- Neither side may write anything to standard output that is not a response line —
  not a banner, not a progress bar, not a warning. A plugin whose runtime prints to
  standard output by default must redirect that stream to standard error before its
  first response.

Both sides flush after every line. A plugin that buffers its standard output until
exit will appear to hang.

The engine applies a deadline independently to `initialize` and to every later
request. A subprocess source configures it with the positive integer `deadline_ms`;
when omitted it is 30000 milliseconds. Passing the deadline closes the connection
and reports an `unavailable` source error naming the method and elapsed limit. A
plugin therefore cannot extend a request indefinitely by remaining alive and silent.

### 1.1 Ordering and concurrency

Every request carries an `id`, and every response echoes it. A plugin **may** answer
out of order and **may** work on several requests at once; the engine matches on
`id`, not on arrival order. A plugin that answers strictly in order is also correct
and is the simpler thing to write.

An `id` is a string, unique within one connection. The engine's own ids are opaque to
the plugin: a plugin must echo the value it received, byte for byte, and must not
parse it.

### 1.2 The lifetime of a connection

1. The engine spawns the plugin. The plugin's working directory, environment and
   arguments are the engine's to set; the plugin reads its configuration from the
   `initialize` request rather than from either.
2. The engine sends `initialize` and reads the response before sending anything else.
3. The engine sends any number of method requests.
4. The engine closes the plugin's standard input. The plugin finishes any request it
   has accepted, writes those responses, and exits `0`.

A plugin that reaches end-of-file on standard input with no outstanding requests
exits `0` immediately. A plugin that exits before answering an outstanding request
has failed that request; the engine reports it as
`{"kind": "unavailable"}` (§5) quoting whatever the plugin wrote to standard error.

## 2. The request and response envelopes

Every request is an object with exactly these members:

```json
{ "id": "7", "method": "query_tasks", "params": { } }
```

| Member | Type | Meaning |
| --- | --- | --- |
| `id` | string | Unique within the connection. Echoed in the response. |
| `method` | string | One of the names in §4, or `initialize`. |
| `params` | object | The method's parameters. Present even when empty (`{}`). |

Every response is an object with exactly two or three members:

```json
{ "id": "7", "result": { } }
{ "id": "7", "error": { "kind": "rate-limited", "retry_after_seconds": 30 } }
```

| Member | Type | Meaning |
| --- | --- | --- |
| `id` | string | The request's `id`, echoed. |
| `result` | any | Present on success. The method's result (§4). |
| `error` | object | Present on failure. A `SourceError` (§5). |

Exactly one of `result` and `error` is present. A response carrying both, or
neither, is a protocol violation (§6.3).

A response to a request the plugin never received an `id` for — or a second response
to one `id` — is a protocol violation.

### 2.1 Unknown members are ignored

A reader on either side **ignores** members it does not know, at every level. That is
what lets a later protocol version add an optional field without a version bump: an
old peer skips it, and the meaning of everything it does understand is unchanged. A
version bump is for changes that are **not** safe this way — a removed field, a
narrowed type, a changed meaning — and §6 is how those are refused rather than
guessed at.

New fields added this way are always optional and always have a documented default
when absent. A field whose absence has no sensible default is a version bump.

## 3. The handshake

The first request on a connection is `initialize`, and the plugin answers it before
any other. It settles three things at once: which protocol version is in force, what
the source can do natively, and what configuration it is being built with.

**Request.**

```json
{
  "id": "0",
  "method": "initialize",
  "params": {
    "protocol_version": 2,
    "engine": { "name": "onetaskgraph", "version": "0.1.0" },
    "source_name": "work",
    "config": { "root": "/home/someone/notes/tasks" },
    "secrets": { "LINEAR_API_KEY": "lin_api_…" }
  }
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `protocol_version` | integer | The version the engine is speaking. See §6. |
| `engine.name` | string | For the plugin's diagnostics only. |
| `engine.version` | string | The engine's own version. Advisory. |
| `source_name` | string | The configured name, matching `^[a-z0-9][a-z0-9-]*$`. **For error messages and for recognising itself in a qualified id** — see §3.2. |
| `config` | object | This source's `config:` block, verbatim. |
| `secrets` | object | String to string. Only the variables this plugin asked for; see §3.1. |

**Response.**

```json
{
  "id": "0",
  "result": {
    "protocol_version": 2,
    "kind": "local-md",
    "capabilities": {
      "projects": "native",
      "documents": "unsupported",
      "orphan_tasks": "native",
      "filter_by_label": "native",
      "filter_by_status": "unsupported",
      "search_title": "native",
      "search_content": "unsupported",
      "task_dependencies": "both-directions",
      "project_dependencies": "forward-only",
      "max_page_size": 100
    }
  }
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `protocol_version` | integer | The version the plugin is speaking. See §6. |
| `kind` | string | The plugin kind, matching the `plugin:` field a configuration names it by. |
| `capabilities` | object | A `Capabilities` (§4.2). Read **once**; the engine does not ask again. |
| `writes` | string | Whether this plugin can be written through. Optional; see §3.3. |

An `initialize` that fails answers with an `error` envelope, ordinarily
`{"kind": "config"}` for a `config` block this plugin cannot use, or
`{"kind": "auth"}` for a credential that is absent or refused. The engine reports it
against the configured source and does not send further requests on that connection.

### 3.1 Secrets

A configuration document never carries a credential value. It names an environment
variable — `api_key_env: LINEAR_API_KEY` — and the engine resolves that name through
its own two layers (the process environment, then
`$XDG_CONFIG_HOME/onetaskgraph/secrets.env`) before spawning anything.

The `secrets` object carries **only** the variables this plugin's configuration
names, and nothing else from the engine's environment. A plugin must not read
credentials from its own process environment: doing so would make it work on a host
where the engine's resolution would have failed, which is exactly the difference the
`config show` verb reports and a user relies on.

The two names this product knows are `LINEAR_API_KEY` and `GH_PROJECTS_TOKEN`. There
is no second spelling of either, anywhere, and nothing translates between spellings.

A plugin must not echo a secret value into a response, into an error message, or onto
standard error.

### 3.2 A plugin never speaks in qualified ids

Inside the plugin every identifier it *reports* is a bare `NativeId` — the source's own
opaque string. Qualifying one into `<source>:<native>` is the engine's job. A plugin that
returns a qualified id has returned a wrong id.

`source_name` is in the handshake for two uses and no others: quoting the source in an
error message, and recognising itself in a qualified id a *near item recorded* under
`onetaskgraph.depends_on` (§4.8), which is how a plugin tells a far end its own backend
could have related from one in a system it knows nothing about. Nothing else about a
plugin's behaviour may depend on it: a source answers the same way whatever a document
chose to call it.

A `NativeId` is any non-empty string, colons included: the engine splits a qualified
id on its **first** colon precisely so that stays true.

### 3.3 `writes`

A `WriteSupport`, and the whole of what a plugin says about its write side. It is
`"supported"` when this plugin implements §4.9, and `"unsupported"` when it does not.

The member is **optional**, and an absent one means `"unsupported"`. That is §2.1 doing
its job: a plugin written before there was a write side says nothing here and is read as
the read-only source it is, with no version bump on either side.

The engine reads this once, at the handshake, exactly as it reads `capabilities`. A copy
naming a plugin that answered `"unsupported"` as its destination is refused before
anything is read, naming the configured source and this plugin kind — so a plugin never
receives a write it would only have to refuse.

## 4. The methods

One method per trait method, named after it. Each is given below as its `params` and
its `result`; the JSON shape of every contract type in them is what
`onetaskgraph schema` emits for the type of the same name.

| Method | Trait method |
| --- | --- |
| `initialize` | `SourcePlugin::build` + `TaskSource::kind` + `TaskSource::capabilities` |
| `health` | `TaskSource::health` |
| `get_task` | `TaskSource::get_task` |
| `get_project` | `TaskSource::get_project` |
| `query_tasks` | `TaskSource::query_tasks` |
| `query_projects` | `TaskSource::query_projects` |
| `labels` | `TaskSource::labels` |
| `task_dependencies` | `TaskSource::task_dependencies` |
| `project_dependencies` | `TaskSource::project_dependencies` |
| `write_task` | `TaskSource::write_task` |
| `write_project` | `TaskSource::write_project` |
| `delete_task` | `TaskSource::delete_task` |
| `delete_project` | `TaskSource::delete_project` |
| `get_document` | `TaskSource::get_document` |
| `query_documents` | `TaskSource::query_documents` |
| `write_document` | `TaskSource::write_document` |
| `delete_document` | `TaskSource::delete_document` |

`kind`, `capabilities` and `writes` are not methods of their own: all three are settled
by the handshake, and the engine reads capabilities once per connection.

### 4.1 Common parameter shapes

**`PageRequest`** — every paged method takes one under `page`:

```json
{ "cursor": "eyJvZmZzZXQiOjIwfQ", "limit": 50 }
```

`cursor` is `null` or absent for the first page, and otherwise exactly the `next`
value from a previous `Page` of the **same** method with the **same** other
parameters. It is the plugin's own encoding and is opaque to the engine, which never
interprets, rewrites or compares one. `limit` is a positive integer and never exceeds
the `max_page_size` the handshake declared.

**`Page<T>`** — every paged method returns one:

```json
{ "items": [], "next": "eyJvZmZzZXQiOjQwfQ" }
```

`next` is `null` (or absent) when the page just returned is the last one. A plugin
returning fewer items than `limit` is not thereby saying there are no more: only
`next: null` says that.

### 4.2 `Capabilities`

`projects`, `documents`, `orphan_tasks`, `filter_by_label`, `filter_by_status`,
`search_title` and `search_content` are each `"native"` or `"unsupported"`.
`task_dependencies` and `project_dependencies` are each `"both-directions"` or
`"forward-only"` — there is deliberately **no** unsupported value for these two.
`max_page_size` is a positive integer.

`documents` is the one member of this object that is **optional**, and an absent one means
`"unsupported"`. That is §2.1 doing its job, exactly as it does for the write-support member
§3.3 specifies: a plugin written before there were documents says nothing here and is read
as the document-free source it is, with no version bump on either side.

`documents` is also not a *predicate*, and the rules below do not reach it. It says whether
this source has documents at all, in the shape `projects` uses, so there is no wider result
set to return and nothing for the engine to narrow. The engine reads it once at the
handshake and never sends a document method to a plugin that answered `"unsupported"` — see
§4.11, which is also where a plugin is told to refuse rather than answer an empty page if
one arrives anyway.

Three rules bind every plugin, and the engine's compensation is only correct while
all three hold:

1. A plugin **applies** every predicate it declared `"native"`.
2. A plugin **ignores** every predicate it declared `"unsupported"`: it returns the
   *wider* result set, never a narrower one. Silently dropping rows for a predicate it
   did not declare is the one failure no test above the plugin can catch. A predicate
   a source can only *half* apply — a `title-or-content` search where only titles are
   searchable — must be declared unsupported and ignored outright, because half
   applying it narrows.
3. This reaches the six `"native"`/`"unsupported"` predicates alone — not `documents`,
   which is not one of them. A dependency read is never ignored and never silently empty.
   A `"forward-only"` plugin still answers `depended-on-by` — see §4.8.

### 4.3 `health`

```json
{ "id": "1", "method": "health", "params": {} }
{ "id": "1", "result": { "reachable": true, "detail": "api.linear.app in 84ms" } }
```

`result` is a `Health`. `detail` is `null` or absent when there is nothing useful to
say. A source that cannot be reached answers `reachable: false` with a `detail` rather
than an `error` envelope; an `error` here means the *check itself* could not be made.

### 4.4 `get_task` and `get_project`

```json
{ "id": "2", "method": "get_task", "params": { "id": "ENG-1" } }
{ "id": "2", "result": { "task": { } } }
```

`result.task` is a `Task`, or `null` when there is no such task. `get_project` takes
the same `params` and returns `result.project`, a `Project` or `null`.

"No such task" is `null`, not an error. A plugin that returns
`{"kind": "refused"}` for an id that simply does not exist makes an ordinary lookup
look like a failure of the source.

### 4.5 `query_tasks`

```json
{
  "id": "3",
  "method": "query_tasks",
  "params": {
    "query": {
      "text": { "terms": "migration", "fields": "title-or-content" },
      "labels": { "any_of": ["bug"], "all_of": [], "none_of": ["wontfix"] },
      "statuses": ["todo", "in-progress"],
      "project": { "is": "PRJ-4" }
    },
    "page": { "cursor": null, "limit": 50 }
  }
}
```

`result` is a `Page<Task>`.

- `text` is `null` or absent when there is no search. `fields` is `"title"`,
  `"content"` or `"title-or-content"`.
- `labels` names labels **by name**, matched **case-insensitively** — a label id is
  per-source, and a user filtering across sources types a word. `any_of` matches a
  task carrying at least one; `all_of` matches a task carrying every one; `none_of`
  excludes a task carrying any. An empty list is not a filter.
- `statuses` holds `StatusCategory` values: `"draft"`, `"backlog"`, `"todo"`,
  `"in-progress"`, `"done"`, `"cancelled"`, `"unknown"`. An empty list is not a filter.
- `project` is the string `"any"`, the string `"orphans"` — tasks belonging to no
  project — or the object `{"is": "<native id>"}`. It is externally tagged, unlike
  `SourceError` (§5), which is tagged on `kind`; both shapes are as
  `onetaskgraph schema` emits them.

### 4.6 `query_projects`

The same shape without `project`:

```json
{ "query": { "text": null, "labels": { }, "statuses": [] }, "page": { } }
```

`result` is a `Page<Project>`.

### 4.7 `labels`

```json
{ "id": "4", "method": "labels", "params": { "page": { "cursor": null, "limit": 50 } } }
```

`result` is a `Page<Label>`: every label this source knows, whether or not anything
carries it.

### 4.8 `task_dependencies` and `project_dependencies`

```json
{
  "id": "5",
  "method": "task_dependencies",
  "params": {
    "id": "ENG-1",
    "direction": "depends-on",
    "page": { "cursor": null, "limit": 50 }
  }
}
```

`direction` is `"depends-on"` or `"depended-on-by"`. `result` is a
`Page<DependencyEdge>`, each edge being `{"from":{"id":"<qualified id>",
"kind":"task"},"to":{"id":"<qualified id>","kind":"project"},"kind":"blocks"}`.
An endpoint kind is `"task"` or `"project"`; an edge kind is `"blocks"` or
`"related"`, and `from` **depends on** `to`.

**`from` depends on `to` in both directions**, which is the part worth reading twice: the
direction says which end the caller asked about, not which end the edge starts at. A
backend that spells the relationship from the blocking side — GitHub's `blockedBy` and
`blocking` are one relationship read from either end — reports the *same* edge for both,
with the item that waits as `from`. A plugin that mirrored the edge for
`"depended-on-by"` would make one relationship read as two contradictory ones.

An endpoint may be in another source. One rule decides where each edge lives, and every
plugin follows it: **the backend's own relationship wherever that relationship can name
the far end, and the reserved key only where it cannot.** A plugin reads and writes its
edges natively for every far end its backend can hold, so the backend knows the graph and
its own interface draws it.

A far end in another source is the case no backend can hold — nothing relates an id in a
system it knows nothing about — so every plugin falls back for that one, and none falls
back for a far end its own relationship can name. The fallback is the near item's
`onetaskgraph.depends_on` metadata: a list of endpoints, each either a bare native id
naming a task or `{"id": "<source>:<native>", "kind": "project"}`, and each read as one
`"blocks"` edge from the near item to that endpoint.

Only the forward direction is ever recorded. The reverse of a recorded edge is derived
from the far end, exactly as a `"forward-only"` plugin's reverse is, so a plugin never
returns a recorded edge for `"depended-on-by"`.

**The fallback is refused where the backend could have answered.** A plugin rejects a
recorded endpoint its own relationship can name — an id of the kind that relationship
holds, naming an item of the plugin's own source — with `{"kind": "malformed"}` naming the
entry, because such an edge belongs in the backend where its own interface can draw it.
An unqualified id names the plugin's own source implicitly and `<own name>:<native>` names
it in writing, so **both spellings are refused**: which one a plan happened to use says
nothing about where the edge belongs. An endpoint qualified to a *different* source is
never refused — that is the case this key exists for — and neither is an endpoint at a
level the backend cannot relate across, which is a gap of the same shape and stays the
key's case however it is qualified. So a GitHub issue refuses `I_sibling` and
`<own name>:I_sibling` alike, and accepts a board or another source; a GitHub draft, having
no relationship at all, accepts anything.

Comparing against its own name is the one thing a plugin's configured name decides, and it
learns that name from the handshake and nowhere else (§3.2).

The engine reports such an edge and never follows it: the read names the far end, and
fetching it is the caller's next command against that qualified id. Keeping the far id on
the near item is plugin-owned work data, not the forbidden engine-side index or mirror —
what the invariant forbids is the engine holding a resolution from one source's id to
another's, and reporting an id a plugin already owns holds nothing.

**A cursor is resumed only in the direction that reported it.** Only a forward walk ever
reaches the recorded fallback, so its tail cursor names a position no reverse walk has; a
plugin handed one in the reverse direction refuses it, naming the cursor, rather than
answering an empty page — which reads as a walk that ended — or, worse, serving the
forward edges it points at. The engine never sends one: a dependency query's fingerprint
carries its direction, so a token minted forwards is refused before it resumes a reverse
walk. A peer writing the protocol by hand can, which is why the plugin refuses rather than
trusting.

A plugin that declared `"both-directions"` answers both directions itself, and must
never return an empty page for a direction it declared.

A plugin that declared `"forward-only"` is **not asked** for `"depended-on-by"`: the
engine reads that from the handshake and answers the reverse direction itself, by a
bounded page-by-page scan asking each item for its forward edges, reporting the
predicate as *emulated* in the plan the caller sees. Such a plugin should still
implement the case defensively — answering `{"kind": "unavailable"}` with a message
saying the direction is not served — because a silently empty page there is
indistinguishable from "nothing depends on this", which is the one wrong answer this
method can give.

### 4.9 `write_task` and `write_project`

Only a plugin that answered `"supported"` to §3.3 is ever sent either of these.

```json
{
  "id": "6",
  "method": "write_task",
  "params": {
    "write": {
      "target": "ENG-1",
      "item": { "id": "T-1", "title": "Rate-limit the sync loop", "…": "…" },
      "depends_on": [
        {
          "from": { "id": "T-1", "kind": "task" },
          "to": { "id": "other:P-9", "kind": "project" },
          "kind": "blocks"
        }
      ]
    }
  }
}
```

`write` is an `ItemWrite`, and it carries three members. `target` is the id of the item
at **this** source to update, or `null` to create one. `item` is a `Task` for
`write_task` and a `Project` for `write_project`, holding the item as this source should
hold it once the write lands; its own `id` is the id it was read under at the *source* it
came from, which a create may derive a name from and an update ignores. `depends_on` is
the forward edges to record, each one's `from` naming the item as its source named it, so
a plugin reads the `to` and the `kind` of each and supplies its own near end. It defaults
to the empty list.

`result` is `{"id": …}`, carrying the `NativeId` this source now holds the item under. A
create is free to choose an id other than the one `item` suggested; an update answers
with `target`.

`url`, `location` (§4.13), `created_at` and `updated_at` are the destination's own and are
never written — where the *source* holds an item says nothing about where this one does.
Everything else `item` carries is, and **nothing is silently dropped**: a field this
source cannot represent, and a metadata key it cannot carry, are each a
`{"kind": "refused"}` naming the field or the keys. A `target` this source does not hold
is refused the same way rather than created, because the engine established that id
before asking.

### 4.10 `delete_task` and `delete_project`

Only a plugin that answered `"supported"` to §3.3 is ever sent either of these.

```json
{ "id": "7", "method": "delete_task", "params": { "id": "ENG-1" } }
```

`id` is the `NativeId` of the item at **this** source to remove. `result` is the empty
object `{}` — the trait method carries `()`, so there is nothing for it to say beyond
having done it.

This is not a verb of the product: nothing a user types deletes anything, and the engine
sends it in one situation only — a copy that could not finish, undoing the items it
itself created in that run. A copy is either complete or it never happened, because a
half-written project has to be run again and the re-run is the mutation burst that trips a
hosted destination's rate limiter.

**An `id` naming nothing is not an error.** The item is already gone, which is the state
this method asks for, so a plugin answers `{}` rather than refusing. What a plugin does
refuse is what it refuses everywhere else: no write side at all, or an item it cannot
remove, each as a `{"kind": "refused"}` saying which.

### 4.11 `get_document` and `query_documents`

Only a plugin that declared `documents` `"native"` in §4.2 is ever sent either of these.

```json
{ "id": "8", "method": "get_document", "params": { "id": "D-1" } }
{ "id": "8", "result": { "document": { } } }
```

`result.document` is a `Document`, or `null` when there is no such document — "no such
document" is `null` and not an error, exactly as it is in §4.4.

```json
{
  "id": "9",
  "method": "query_documents",
  "params": {
    "query": {
      "text": { "terms": "design", "fields": "title-or-content" },
      "labels": { "any_of": [], "all_of": [], "none_of": [] },
      "project": { "is": "PRJ-4" }
    },
    "page": { "cursor": null, "limit": 50 }
  }
}
```

`query` is a `DocumentQuery` and `result` is a `Page<Document>`. Its `text`, `labels` and
`project` members are read exactly as §4.5 reads the members of the same names. There is no
`statuses` member, and a plugin must not invent one: a document is not work and carries no
status, so there is nothing for a status filter to compare against.

A `Document` is a piece of information that lives in a project and is not work. It carries
an `id`, a `title`, an optional `content`, an optional `project` — `null` is an orphan
document — its `labels`, its `url`, its `location` (§4.13), its `created_at` and
`updated_at`, its `metadata` and its `repositories`, each on the terms the `Task` member of
the same name is read on. It carries **no status and no dependencies**, and both absences
are the contract rather than an oversight: nothing may point at a document, which is why
the endpoint kinds of §4.8 remain `"task"` and `"project"` alone.

**A plugin with no documents refuses rather than answering an empty page.** It declared
`documents` `"unsupported"`, so the engine never sends it one — the declaration is read
once at the handshake, exactly as `writes` is (§3.3) — and a document read spanning several
sources reports such a source as holding none rather than as having failed. A plugin should
still implement the case defensively, with a `{"kind": "refused"}` naming itself, for the
reason a `"forward-only"` plugin implements `"depended-on-by"` defensively: an empty page
here is indistinguishable from a source that has documents and holds none matching, which
is the one wrong answer these methods can give.

### 4.12 `write_document` and `delete_document`

Only a plugin that answered `"supported"` to §3.3 is ever sent either of these.

```json
{ "id": "10", "method": "write_document", "params": { "write": { } } }
{ "id": "11", "method": "delete_document", "params": { "id": "D-1" } }
```

`write` is an `ItemWrite` whose `item` is a `Document`, and it is read exactly as §4.9
reads one: `target` is the id of the document at **this** source to update or `null` to
create, `result` is `{"id": …}` carrying the `NativeId` this source now holds it under, and
nothing the item carries is silently dropped. `depends_on` is always the empty list here,
because a document has no dependencies.

`delete_document` takes an `id` and answers the empty object `{}` — the trait method
carries `()`, so there is nothing for it to say beyond having done it — on every term §4.10
sets: it is not a verb of the product, an `id` naming nothing is not an error, and the
engine sends it only while undoing a copy that could not finish.

### 4.13 `Location`

Where an entity is, in the one form a consumer can act on without knowing the backend. It
appears as the optional `location` member of a `Task`, a `Project` and a `Document`, and it
is an object with exactly one of two keys:

```json
{ "url": "https://example.invalid/D-1" }
{ "path": "/home/someone/notes/design.md" }
```

- `url` — the entity lives at an external website, and this is a link a reader can open.
- `path` — the entity is a file on the machine the *plugin* runs on, and this is that
  file's absolute path, so a reader can print the path or read the contents out.

A consumer tells the two apart by which key is present, and a plugin sends exactly one of
them. The member is **optional** on all three entities and an absent one means `null`,
which says *this source did not say where the entity is* — not that it is nowhere.

This is not the `url` field those three entities already carry, does not replace it, and is
not derived from it. A plugin that reported a web address there goes on reporting exactly
what it reported before, whether or not it also says where the entity is.

## 5. The error envelope

`error` carries a `SourceError` whole. It is internally tagged on `kind`, and every
variant's data is owned — the type was shaped this way for exactly this boundary.

| `kind` | Other members | Meaning |
| --- | --- | --- |
| `config` | `message` (string) | The configuration for this source is invalid. |
| `auth` | `message` (string) | Authentication failed or a credential is absent. |
| `refused` | `message` (string) | The source understood the request and refused it. |
| `rate-limited` | `retry_after_seconds` (integer or `null`) | The source rate-limited the request. |
| `unavailable` | `message` (string) | The source could not be reached. |
| `malformed` | `message` (string) | The source returned data this interface cannot represent. |

```json
{ "id": "3", "error": { "kind": "rate-limited", "retry_after_seconds": 30 } }
```

A `message` is for a person to read. It must not contain a credential, and it must
not contain a qualified id (§3.2).

One source failing never fails a whole query: the engine records the failure against
that source, keeps every other source's results, and exits non-zero unless the caller
allowed partial results.

## 6. Versions

The protocol version is a single positive integer, carried in the handshake and
nowhere else. This document specifies version **2**.

Version 2 added §4.10 — `delete_task` and `delete_project`. Adding a method is not safe
under §2.1: a plugin speaking version 1 has never heard of either, and the one moment the
engine sends them is while it is undoing a copy that has already written to that plugin.
Refusing such a plugin at the handshake, by name, is the only place that difference can be
reported before anything has been written.

The documents of §4.11 and §4.12, and the `location` member of §4.13, were added **without**
a bump, and the difference from §4.10 above is the whole reason. The engine sends a document
method only to a plugin that declared `documents` `"native"` in its handshake, and a plugin
written before there were documents cannot have declared that: it omits the member, §4.2
reads the omission as `"unsupported"`, and it is never sent one. A delete had no such gate —
`writes` says whether a plugin can be written, not whether it can be un-written — so a
version 1 plugin could be sent one halfway through undoing a copy. Adding a method behind a
declaration its peer cannot accidentally make is the "method a peer may decline" case below;
adding one a peer has already implicitly opted into is not.

A version is bumped when a change is **not** safe under §2.1 — a member removed, a
type narrowed, a meaning changed, a method removed or renamed. Adding an optional
member with a documented default, or adding a method a peer may decline, is not a
bump.

### 6.1 The rule

**A protocol version a peer does not know is refused by name, never guessed at.**
There is no negotiation, no "closest supported", and no silent downgrade. A peer that
guessed would be running against a shape it has not been written for, and the failure
would surface later as wrong data rather than as a refusal.

### 6.2 How the refusal is made

The engine sends `initialize` with the version it speaks. Then:

- **The plugin does not know that version.** It answers with an `error` envelope,
  `kind` `"config"`, whose `message` names both versions and the versions it does
  know:

  ```json
  {
    "id": "0",
    "error": {
      "kind": "config",
      "message": "protocol version 3 is not supported by this plugin; it speaks version 2"
    }
  }
  ```

  It then exits `0`. The engine reports the source as unusable, naming the plugin and
  both versions.

- **The plugin knows that version.** It answers with `protocol_version` set to the
  version it will speak, which is the version the engine asked for. A plugin that
  supports several versions answers in the one it was asked for.

- **The plugin answers with a version the engine did not ask for**, or omits
  `protocol_version`. The engine refuses the source by name, saying which version it
  asked for and which it was answered in, and closes the connection. It does not
  proceed in either version.

Both refusals name the plugin, both versions, and the fact that the two are
incompatible. Neither is a warning that a run continues past.

### 6.3 Protocol violations

A message that is not one line of valid JSON, an envelope with both `result` and
`error`, an envelope with neither, a response whose `id` was never sent or was
answered already, or a response before the handshake — each is a protocol violation.

The engine reports a violating plugin as `{"kind": "malformed"}` against its
configured source, quoting the offending line truncated to a readable length, and
closes the connection. A plugin that receives a violating request answers, if it can
associate one with an `id`, with `{"kind": "malformed"}`; otherwise it writes a
diagnostic to standard error and continues reading. Neither side crashes on a bad
line from the other.

## 7. A complete exchange

Engine to plugin:

```
{"id":"0","method":"initialize","params":{"protocol_version":2,"engine":{"name":"onetaskgraph","version":"0.1.0"},"source_name":"notes","config":{"root":"/home/someone/notes/tasks"},"secrets":{}}}
{"id":"1","method":"query_tasks","params":{"query":{"text":null,"labels":{"any_of":[],"all_of":[],"none_of":[]},"statuses":["todo"],"project":"any"},"page":{"cursor":null,"limit":2}}}
{"id":"2","method":"task_dependencies","params":{"id":"tasks/migrate.md","direction":"depends-on","page":{"cursor":null,"limit":50}}}
```

Plugin to engine:

```
{"id":"0","result":{"protocol_version":2,"kind":"local-md","capabilities":{"projects":"native","documents":"unsupported","orphan_tasks":"native","filter_by_label":"native","filter_by_status":"unsupported","search_title":"native","search_content":"unsupported","task_dependencies":"forward-only","project_dependencies":"forward-only","max_page_size":200}}}
{"id":"1","result":{"items":[{"id":"tasks/migrate.md","title":"Migrate the store","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"project":null,"url":null,"location":null,"created_at":null,"updated_at":null},{"id":"tasks/schema.md","title":"Settle the schema","content":null,"status":{"category":"in-progress","name":"Doing"},"labels":[],"project":null,"url":null,"location":null,"created_at":null,"updated_at":null}],"next":"b2Zmc2V0PTI"}}
{"id":"2","result":{"items":[{"from":"tasks/migrate.md","to":"tasks/schema.md","kind":"blocks"}],"next":null}}
```

Note that this plugin declared `filter_by_status` unsupported and answered a query for
`todo` with two tasks, the second of them `in-progress`. That is rule 2 working: the
plugin returned the **wider** set rather than a narrower one, and the engine drops the
second row itself. A plugin that had filtered here — half-applying a predicate it
declared it would not apply — would have been indistinguishable from one whose source
simply held one task, which is the failure no test above the plugin can catch.

## 8. What this document does not cover

- **Transport other than stdio.** There is one, and adding another is a change to
  this document.
- **A plugin calling back into the engine.** It cannot. Every message is a request
  from the engine and a response from the plugin.
- **Persisting anything.** No work data may be stored, cached, indexed or mirrored
  outside the plugin that owns it. The engine compensates transiently and writes
  nothing down; a plugin that caches is caching its own source's data, which is the
  one place that is allowed. §4.9 is not an exception to this: a destination write is
  at the user's explicit request, names its destination, goes through that source's own
  write interface into that source's own store, and is never read back to answer a
  query. A cache is a write nobody asked for that the engine reads back.
