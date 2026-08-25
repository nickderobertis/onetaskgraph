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
    "protocol_version": 1,
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
| `source_name` | string | The configured name, matching `^[a-z0-9][a-z0-9-]*$`. **For error messages only** — see §3.2. |
| `config` | object | This source's `config:` block, verbatim. |
| `secrets` | object | String to string. Only the variables this plugin asked for; see §3.1. |

**Response.**

```json
{
  "id": "0",
  "result": {
    "protocol_version": 1,
    "kind": "local-md",
    "capabilities": {
      "projects": "native",
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

### 3.2 A plugin never learns its own address

`source_name` is in the handshake so a plugin can quote it in an error message and
for nothing else. Inside the plugin every identifier is a bare `NativeId` — the
source's own opaque string. Qualifying one into `<source>:<native>` is the engine's
job. A plugin that returns a qualified id has returned a wrong id.

A `NativeId` is any non-empty string, colons included: the engine splits a qualified
id on its **first** colon precisely so that stays true.

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

`kind` and `capabilities` are not methods of their own: both are settled by the
handshake, and the engine reads capabilities once per connection.

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

`projects`, `orphan_tasks`, `filter_by_label`, `filter_by_status`, `search_title` and
`search_content` are each `"native"` or `"unsupported"`. `task_dependencies` and
`project_dependencies` are each `"both-directions"` or `"forward-only"` — there is
deliberately **no** unsupported value for these two. `max_page_size` is a positive
integer.

Three rules bind every plugin, and the engine's compensation is only correct while
all three hold:

1. A plugin **applies** every predicate it declared `"native"`.
2. A plugin **ignores** every predicate it declared `"unsupported"`: it returns the
   *wider* result set, never a narrower one. Silently dropping rows for a predicate it
   did not declare is the one failure no test above the plugin can catch. A predicate
   a source can only *half* apply — a `title-or-content` search where only titles are
   searchable — must be declared unsupported and ignored outright, because half
   applying it narrows.
3. This reaches the six `"native"`/`"unsupported"` predicates alone. A dependency read
   is never ignored and never silently empty. A `"forward-only"` plugin still answers
   `depended-on-by` — see §4.8.

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
- `statuses` holds `StatusCategory` values: `"backlog"`, `"todo"`, `"in-progress"`,
  `"done"`, `"cancelled"`, `"unknown"`. An empty list is not a filter.
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

An endpoint may be in another source. The plugin reads that qualified far id from the
near item's reserved `onetaskgraph.depends_on` metadata when its backend cannot express
the relationship. The engine reports but never follows it. Keeping the far id on the
near item is plugin-owned work data, not the forbidden engine-side index or mirror.

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
nowhere else. This document specifies version **1**.

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
      "message": "protocol version 2 is not supported by this plugin; it speaks version 1"
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
{"id":"0","method":"initialize","params":{"protocol_version":1,"engine":{"name":"onetaskgraph","version":"0.1.0"},"source_name":"notes","config":{"root":"/home/someone/notes/tasks"},"secrets":{}}}
{"id":"1","method":"query_tasks","params":{"query":{"text":null,"labels":{"any_of":[],"all_of":[],"none_of":[]},"statuses":["todo"],"project":"any"},"page":{"cursor":null,"limit":2}}}
{"id":"2","method":"task_dependencies","params":{"id":"tasks/migrate.md","direction":"depends-on","page":{"cursor":null,"limit":50}}}
```

Plugin to engine:

```
{"id":"0","result":{"protocol_version":1,"kind":"local-md","capabilities":{"projects":"native","orphan_tasks":"native","filter_by_label":"native","filter_by_status":"unsupported","search_title":"native","search_content":"unsupported","task_dependencies":"forward-only","project_dependencies":"forward-only","max_page_size":200}}}
{"id":"1","result":{"items":[{"id":"tasks/migrate.md","title":"Migrate the store","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"project":null,"url":null,"created_at":null,"updated_at":null},{"id":"tasks/schema.md","title":"Settle the schema","content":null,"status":{"category":"in-progress","name":"Doing"},"labels":[],"project":null,"url":null,"created_at":null,"updated_at":null}],"next":"b2Zmc2V0PTI"}}
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
  one place that is allowed.
