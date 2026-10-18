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
what the `onetaskgraph-source` reference host runs, and what lets the shared
journeys run every one of themselves a second time over a real pipe. That host is a
binary target of the `onetaskgraph` crate rather than part of its command-line
interface: build it from a checkout to read a working peer, and do not depend on
finding it beside an installed CLI. Where this
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
| `statuses` | array of strings | The status categories this engine knows. Optional; see §3.5. |
| `document_dir` | string | The absolute directory holding the configuration document that supplied `config`. Optional; see §3.8. |

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
| `meters` | boolean | Whether this plugin answers `metering`. Optional; see §3.4. |
| `statuses` | array of strings | The status categories this plugin knows. Optional; see §3.5. |
| `task_updates` | boolean | Whether this plugin answers the two narrow task writes and holds a task's `delivers` and `delivered_by`. Optional; see §3.6. |
| `metadata_updates` | boolean | Whether this plugin answers the three narrow metadata writes. Optional; see §3.7. |
| `content_updates` | boolean | Whether this plugin answers the narrow content write. Optional; see §3.9. |
| `targeted_updates` | boolean | Whether this plugin answers the targeted update of one task. Optional; see §3.10. |
| `ends_commands` | boolean | Whether this plugin answers `end_command`. Optional; see §3.11. |
| `comment_copies` | boolean | Whether this plugin answers `write_comment_copy`. Optional; see §3.12. |
| `task_details` | boolean | Whether this plugin answers `get_task_detail`. Optional; see §3.13. |
| `write_checks` | boolean | Whether this plugin answers `check_task_write`. Optional; see §3.14. |

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

### 3.4 `meters`

A boolean: `true` when this plugin counts the requests it sends to its own backend and
answers `metering` (§4.14) with them, and `false` when it does not.

The member is **optional**, and an absent one means `false`. A plugin written before there
was metering says nothing here, is never sent `metering`, and is reported as not metering —
which a command reports by leaving what it spent out, never by reporting that nothing was
spent.

### 3.5 `statuses`

An array of the `StatusCategory` values a peer knows, each spelled as §4.5 spells it. Both
sides send one: the engine in the `params` of `initialize`, and the plugin in its `result`.

The member is **optional**, and an absent one means the **first vocabulary**: `"draft"`,
`"backlog"`, `"todo"`, `"in-progress"`, `"done"`, `"cancelled"` and `"unknown"` — every
category there was before `"queued"`. A listed value the other side has never heard of is
ignored, by §2.1.

Nothing crosses to a peer that did not list it without a refusal by name:

- **An engine never hands a plugin a category that plugin did not list.** A `query_tasks` or
  `query_projects` filter naming one is answered without that category: a plugin that does
  not know a category holds no item in it, so every other category of the filter is sent as
  it was and a filter naming only such categories is answered with an empty last page and
  nothing sent. A `write_task` or `write_project` carrying one, and a `set_task_status` naming
  one, are refused before anything is sent, with a `{"kind": "refused"}` naming the plugin
  and the category.
- **A plugin never tells an engine a category that engine did not list.** An item in such a
  category is reported as `"unknown"`, keeping its `name`, in every result that carries a
  `Status`: `get_task`, `get_project`, `query_tasks`, `query_projects` and `set_task_status`.

### 3.6 `task_updates`

A boolean: `true` when this plugin answers `set_task_status` and `set_delivered_by` (§4.17)
and holds a task's `delivers` and `delivered_by`, and `false` when it does not.

The member is **optional**, and an absent one means `false`. Such a plugin is never sent
either method — the call is refused before anything is sent, naming the plugin — and is never
handed a `write_task` whose task carries a non-empty `delivers` or `delivered_by`, which a
plugin written before those lists would otherwise drop in silence under §2.1.

### 3.7 `metadata_updates`

A boolean: `true` when this plugin answers `set_task_metadata`, `set_project_metadata` and
`set_document_metadata` (§4.18), and `false` when it does not.

The member is **optional**, and an absent one means `false`. Such a plugin is never sent any
of the three: the call is refused before anything is sent with `{"kind": "refused"}` and the
message `the <kind> plugin cannot write a task's metadata on its own` — `a project's` or
`a document's` for the other two — which is exactly what a plugin that declares the member and
cannot make the write answers with, so a caller cannot tell the two apart and has no reason to.

### 3.8 `document_dir`

The absolute directory holding the configuration document that supplied this source's
`config` block — the directory a relative path written in that document is measured from.

The member is **optional**, and it is absent when the block came from no one document: when
any of its settings came from the environment layer or a command-line flag, where there is
no document to measure from, and when two documents each supplied part of it. Absent, a
relative path keeps resolving against the plugin's working directory, which is the
engine's: the engine does not set the child's working directory, so the child inherits it.
That is the same rule the engine applies in process, stated for a user under "Relative
paths in a configuration document" in `README.md`.

The engine resolves nothing inside `config` itself — the block is this plugin's, so which of
its fields are paths is this plugin's to say. A plugin that has path fields resolves each
relative one it holds against `document_dir` when the member is present, leaves an absolute
one and an empty one as they are, and resolves nothing else; a plugin with no path fields
receives the member and does nothing with it. The reference host `onetaskgraph-source`
resolves exactly the fields its hosted plugin declares through
`SourcePlugin::document_relative_paths`, which is what the engine resolves against the
document for that plugin when it runs in process — so one relative root names one directory
on either side of this seam.

A plugin written before this member ignores it (§2.1) and behaves exactly as it did, so it
was added without a protocol version bump. The directory is always absolute; a plugin that
receives one that is not answers `initialize` with `{"kind": "config"}` rather than
measuring from its own working directory.

### 3.9 `content_updates`

A boolean: `true` when this plugin answers `set_task_content` (§4.20), and `false` when it
does not.

The member is **optional**, and an absent one means `false`. Such a plugin is never sent the
method: the call is refused before anything is sent with `{"kind": "refused"}` and the message
`the <kind> plugin cannot write a task's content on its own`, which is exactly what a plugin
that declares the member and cannot make the write answers with.

### 3.10 `targeted_updates`

A boolean: `true` when this plugin answers `update_task` (§4.21), and `false` when it does not.

The member is **optional**, and an absent one means `false`. Such a plugin is **never sent the
method, and the update is not refused**: the engine carries it out through what the plugin
already answers — `get_task`, `task_dependencies` when the update names `depends_on` or has to
carry the edges through a rewrite, and one `write_task` with its `target` set, sent only when
something differs — which is the trait's own default. A plugin written before the method is
therefore correct and merely not minimal, and it was added without a protocol version bump.

### 3.11 `ends_commands`

A boolean: `true` when this plugin answers `end_command` (§4.22), and `false` when it does not.

The member is **optional**, and an absent one means `false`. Such a plugin is never sent the
method, and the engine reads its silence as a promise: **a plugin that holds anything between
requests that §4.22 forbids it to keep must answer `true`.** One that reads its backend afresh
for every request has nothing to drop and may leave the member out, which is what every plugin
written before the method did.

### 3.12 `comment_copies`

A boolean: `true` when this plugin answers `write_comment_copy` (§4.16a), and `false` when it
does not.

The member is **optional**, and an absent one means `false`. Such a plugin is never sent the
method: a comment a `--comments` copy carries to it is written through `add_comment` and
`edit_comment` (§4.16) instead, with its origin appended to the body as the hidden marker
§4.15 describes, and every comment read back from it has the marker taken out again. A plugin
written before the method therefore keeps working unchanged, and it was added without a
protocol version bump.

### 3.13 `task_details`

A boolean: `true` when this plugin answers `get_task_detail` (§4.4a), and `false` when it does
not.

The member is **optional**, and an absent one means `false`. Such a plugin is never sent the
method: the engine reads a task and a page of its comments from it as two requests, `get_task`
and then `task_comments`, which is the trait's own default. A plugin that can read the two in
one — and so promise they are one revision of the task — answers `true`. Added without a
protocol version bump.

### 3.14 `write_checks`

Optional, and absent means `false`: a plugin that answers `check_task_write` (§4.9b) says so
here. One that does not is never sent the method; every task a copy plans for it is read as one
it can represent, and a write it refuses as one it cannot takes the whole copy back, as it did
before the method existed. Omitted when `false`.

## 4. The methods

One method per trait method, named after it. Each is given below as its `params` and
its `result`; the JSON shape of every contract type in them is what
`onetaskgraph schema` emits for the type of the same name.

| Method | Trait method |
| --- | --- |
| `initialize` | `SourcePlugin::build` + `TaskSource::kind` + `TaskSource::capabilities` |
| `health` | `TaskSource::health` |
| `get_task` | `TaskSource::get_task` |
| `get_task_detail` | `TaskSource::get_task_details`, one task and a page of its comments |
| `get_project` | `TaskSource::get_project` |
| `query_tasks` | `TaskSource::query_tasks` |
| `query_projects` | `TaskSource::query_projects` |
| `labels` | `TaskSource::labels` |
| `task_dependencies` | `TaskSource::task_dependencies` |
| `project_dependencies` | `TaskSource::project_dependencies` |
| `write_task` | `TaskSource::write_task` |
| `write_project` | `TaskSource::write_project` |
| `check_task_write` | `TaskSource::check_task_write` |
| `delete_task` | `TaskSource::delete_task` |
| `delete_project` | `TaskSource::delete_project` |
| `get_document` | `TaskSource::get_document` |
| `query_documents` | `TaskSource::query_documents` |
| `write_document` | `TaskSource::write_document` |
| `delete_document` | `TaskSource::delete_document` |
| `metering` | `TaskSource::metering` |
| `task_comments` | `TaskSource::task_comments` |
| `add_comment` | `TaskSource::add_comment` |
| `edit_comment` | `TaskSource::edit_comment` |
| `delete_comment` | `TaskSource::delete_comment` |
| `write_comment_copy` | `TaskSource::write_comment_copy` |
| `set_task_status` | `TaskSource::set_task_status` |
| `set_delivered_by` | `TaskSource::set_delivered_by` |
| `set_task_priority` | `TaskSource::set_task_priority` |
| `set_task_content` | `TaskSource::set_task_content` |
| `set_task_metadata` | `TaskSource::set_task_metadata` |
| `set_project_metadata` | `TaskSource::set_project_metadata` |
| `set_document_metadata` | `TaskSource::set_document_metadata` |
| `update_task` | `TaskSource::update_task` |
| `end_command` | `TaskSource::end_command` |
| `status_contradictions` | `TaskSource::status_contradictions` |
| `issue_holdings` | `TaskSource::issue_holdings` |

`kind`, `capabilities` and `writes` are not methods of their own: all three are settled
by the handshake, and the engine reads capabilities once per connection.

The template operations are not carried. There is no method for any of these; for a source
hosted over this protocol the engine answers the first itself, and refuses each of the others
before sending anything, naming the source and the operation:

- `keeps_template_answers`, which the engine answers itself: a hosted source keeps none;
- `task_template_answers`, `project_template_answers` and `document_template_answers`, a read
  of an item's stored template answers;
- `write_task_rendered`, `write_project_rendered` and `write_document_rendered`, a create from
  a template;
- `set_task_rendering`, `set_project_rendering` and `set_document_rendering`, a regenerate in
  place, and `set_task_rendering_with_assets` and `set_document_rendering_with_assets`, the
  same regenerate storing the item's image assets with it.

The engine never stands `write_task` in for a create from a template, which would land the
content and its provenance without the answers a regenerate needs. A plain-body create is
`write_task`, `write_project` or `write_document` and crosses as it always has.

The image-asset methods are not methods of their own either. `write_task_with_assets` and
`write_document_with_assets` cross as `write_task` and `write_document` carrying the `assets`
member §4.9a specifies. `task_assets`, `document_assets`, `task_asset` and `document_asset` — a
read of the assets a record holds — are not carried at all: a source hosted over this protocol
reports every record as holding none, so a copy *out of* one whose content references an asset
is refused naming the record and the asset, as it is for any source that does not hold it.

`visibility` — a live read of who can read where a write lands, which the engine holds a
source declared `visibility: private` to before every write — is not carried either. A source
hosted over this protocol answers `unknown`, which is never private: the engine refuses to
write an item that may only be written somewhere private to it, whatever its configuration
declares, and treats it as public for everything else. Its writes still cross only after the
engine has applied the same classification and term checks it applies to every source, so a
hosted plugin cannot be a way around them.

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

`projects`, `documents`, `comments`, `assets`, `priority`, `filter_by_priority`,
`filter_by_comment_activity`, `filter_by_metadata`, `filter_by_origin`, `reconcile_status`,
`complete_by_issue`, `orphan_tasks`, `filter_by_label`, `filter_by_status`, `search_title` and
`search_content` are each `"native"` or `"unsupported"`.
`task_dependencies` and `project_dependencies` are each `"both-directions"` or
`"forward-only"` — there is deliberately **no** unsupported value for these two.
`max_page_size` is a positive integer.

`documents`, `comments`, `assets`, `priority`, `filter_by_priority`,
`filter_by_comment_activity`, `filter_by_metadata`, `filter_by_origin`, `reconcile_status` and
`complete_by_issue` are the ten members of this object that are **optional**, and an absent one
means `"unsupported"`. That is §2.1
doing its job, exactly as it does for the write-support member §3.3 specifies: a plugin written
before there were documents, comments, image assets, priorities, a comment-activity filter or a
metadata or origin filter says nothing here and is read as the source without them it is, with
no version bump on either side.

`documents` is also not a *predicate*, and the rules below do not reach it. It says whether
this source has documents at all, in the shape `projects` uses, so there is no wider result
set to return and nothing for the engine to narrow. The engine reads it once at the
handshake and never sends a document method to a plugin that answered `"unsupported"` — see
§4.11, which is also where a plugin is told to refuse rather than answer an empty page if
one arrives anyway.

`comments` is not a predicate either, on exactly those terms: it says whether this source's
tasks have comments at all. The engine never sends a comment method to a plugin that
answered `"unsupported"` — see §4.15 — and sends the three that write (§4.16) only to a
plugin whose write-support member (§3.3) also says it can be written.

`assets` is not a predicate either: it says whether this source stores the image assets a
task's or a document's content references (§4.9a). Only a plugin that answered `"native"`, and
that §3.3 says can be written, is ever sent a write carrying one. A create or a copy of a record whose
content references an asset into a plugin that answered `"unsupported"`, or said nothing, is
refused by the engine before anything is written for that record, naming the source, the record
and the asset; a record whose content references none is written to it exactly as before.

`priority` is not a predicate either: it says whether this source's tasks hold a priority
(§4.13b) at all. A plugin that answered `"unsupported"` reports every task's priority as none,
is never sent the narrow priority write (§4.19), and is never handed a task write (§4.9)
carrying a priority other than none — the engine refuses such a write before anything is sent,
naming the source and the field. `filter_by_priority` **is** a predicate, the priority filter a
task query carries (§4.5), and the rules below reach it like any other.

`filter_by_comment_activity` is a predicate too: the comment-activity instant a task query
carries (§4.5), which keeps a task one of whose comments was created or last edited at or
after it. A plugin that declared it `"native"` applies it; one that answered `"unsupported"`
— or omitted it — is never sent the instant, returns the wider set, and the engine narrows
that set by **reading the plugin's comments task by task** (§4.15) for every task the other
predicates kept. That is correct and it is not cheap, so a plugin that can ask its store the
narrower question should declare it. A plugin whose `comments` is `"unsupported"` holds no
comment activity, and the engine keeps none of its tasks without asking it anything.

`filter_by_metadata` and `filter_by_origin` are predicates too: the metadata matches and the
copy origin a task query carries (§4.5). A plugin that declared one `"native"` applies it; one
that answered `"unsupported"` — or omitted it — is never sent it, returns the wider set, and
the engine narrows that set over each task's own metadata, which every task read already
carries. They are two members rather than one because a store may be able to ask one question
and not the other: a board keeps a copy's origin in a field of its own and caller metadata in
the issue body.

A plugin that declared a predicate `"native"` may still answer `{"kind": "refused"}`, naming
the value, for one value its store cannot search — never a narrower set in its place. The
built-in `github-projects` source does that for two, both before it sends GitHub anything,
because GitHub's issue search indexes words and so no bounded query finds a string with no
letter or digit in it: a metadata match's value with no letter or digit — the empty string,
whitespace or punctuation alone — and a task query's text (§4.5) that is not blank and has no letter
or digit, such as `--`. It neither reads its whole board for either nor answers either as
empty. A blank text is not refused; it keeps the read that source always gave it. A copy
matching by title or by a metadata value (`--match-by`) never asks a native source for such a
title or value, and reads the destination's tasks instead, as it did before either predicate
could be pushed down.

`reconcile_status` and `complete_by_issue` are not predicates either. Each says whether this
source answers one question — which of its tasks contradict their closure (§4.23), and which of
its items issue references name (§4.24) — and only a plugin that answered `"native"` is ever
sent it. Both belong to a source whose status is
a card on a board beside an issue that closes on its own; a source whose status *is* its
closure — a Linear workflow state, a file's own `status:` — has nothing for either to find, and
declares neither. The engine sends a plugin that answered `"unsupported"`, or said nothing,
neither method: `task status reconcile` answers it no items, and `task status complete` lists
it as unsupported.

Three rules bind every plugin, and the engine's compensation is only correct while
all three hold:

1. A plugin **applies** every predicate it declared `"native"`.
2. A plugin **ignores** every predicate it declared `"unsupported"`: it returns the
   *wider* result set, never a narrower one. Silently dropping rows for a predicate it
   did not declare is the one failure no test above the plugin can catch. A predicate
   a source can only *half* apply — a `title-or-content` search where only titles are
   searchable — must be declared unsupported and ignored outright, because half
   applying it narrows.
3. This reaches the ten `"native"`/`"unsupported"` predicates alone — not `documents`,
   `comments`, `assets`, `priority`, `reconcile_status` or `complete_by_issue`, which are not
   among them. A dependency read is never ignored
   and never silently empty.
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

`TaskSource::get_task_details` — several tasks, each with a page of its comments — crosses as
one `get_task_detail` per task (§4.4a) to a plugin that declared `task_details` (§3.13). To one
that did not, the engine answers it the way the trait's default does, with one `get_task` per id
and, for a source whose tasks have comments, one `task_comments` for that page of each task
found, so a plugin written against this document implements nothing more for `task show-many`
to reach it.

`TaskSource::get_task_details_for_copy` — the read a copy makes of the tasks it carries — is not
a message either: for a hosted source the engine answers it exactly as `get_task_details`, so it
crosses as the same `get_task_detail` or `get_task` and `task_comments` requests.

### 4.4a `get_task_detail`

Only a plugin that declared `task_details: true` (§3.13) and `comments` `"native"` (§4.2) is
ever sent this.

```json
{ "id": "2", "method": "get_task_detail", "params": { "id": "ENG-1", "comments": { "cursor": null, "limit": 100 } } }
{ "id": "2", "result": { "task": { }, "page": { "items": [], "next": null } } }
```

`id` is the task and `comments` a `PageRequest` (§4.1) for its comments. `result.task` is the
`Task` §4.4 answers and `result.page` the `Page<Comment>` §4.15 answers for that page, **both
read as one revision of the task**: a plugin answering this owes that no change landed between
what it read of the two. Both are `null` when there is no such task. A task that exists and
whose comments could not be read — not refused as a whole — is answered with `task` and
`comments_error`, the `SourceError` its comments were refused with, in place of `page`.

A copy reads a task whose comments run past one page a page at a time through this method, and
compares the task each answer carries with the first: when they differ, the task changed while
it was being read, and the copy reads it again (§4.13d). The `version`s are compared only where
the source's version coverage says its token covers comments; otherwise — and for a task carrying
no `version` — nothing the task carries moves for a comment edited alone, so the copy also reads
every page again from the first,
until two full passes in a row agree on every comment's id, body and `updated_at`, at most three
passes; a task that never settles is written nowhere and reported failed.

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
      "project": { "is": "PRJ-4" },
      "priorities": ["urgent", "high"],
      "commented_since": "2026-09-20T12:00:00Z",
      "metadata": [
        { "key": "orchestrator.follow-up", "path": ["root_cause"], "value": "stale-cache" }
      ],
      "origin": "notes:N-12"
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
- `statuses` holds `StatusCategory` values: `"draft"`, `"backlog"`, `"todo"`, `"queued"`,
  `"in-progress"`, `"done"`, `"cancelled"`, `"unknown"`. An empty list is not a filter.
  `"todo"` is work accepted and ready to be picked up that nothing has claimed; `"queued"` is
  work claimed by something that will do it and not yet started. §3.5 says what a plugin
  whose handshake does not list `"queued"` is handed instead.
- `project` is the string `"any"`, the string `"orphans"` — tasks belonging to no
  project — or the object `{"is": "<native id>"}`. It is externally tagged, unlike
  `SourceError` (§5), which is tagged on `kind`; both shapes are as
  `onetaskgraph schema` emits them.
- `priorities` holds `Priority` values (§4.13b); a task matches when its priority is any one
  of them, none included. It is **optional**: absent — which is how the engine sends an empty
  list — is not a filter, and the engine sends a non-empty one only to a plugin that declared
  `filter_by_priority` native (§4.2), so a plugin written before priorities never receives
  one.
- `commented_since` is an RFC 3339 instant. A task matches when **at least one of its
  comments** (§4.15) has a `created_at`, or a last edit `updated_at`, at or after it; a task
  with no comments never matches, and a comment deleted before the query is not a match. It is
  **optional**: absent is not a filter, and the engine sends it only to a plugin that declared
  `filter_by_comment_activity` native (§4.2), so a plugin written before it never receives
  one.
- `metadata` is a list of matches over a task's caller-defined `metadata` (§4.13), **every**
  one of which must hold. A match is `key` — one top-level metadata key, which may itself
  contain dots, such as `orchestrator.follow-up` — `path`, zero or more nested object keys
  under it (absent means none), and `value`, a string. It holds when the value at that
  location is a JSON **string** equal to `value`, case-sensitively; a number, a boolean, an
  array, an object, a missing key and a path through anything that is not an object all fail
  it. It is **optional**: absent — how the engine sends an empty list — is not a filter, and the
  engine sends it only to a plugin that declared `filter_by_metadata` native (§4.2).
- `origin` is the qualified id a copy records under `onetaskgraph.origin` — `<source>:<id>`,
  spelled exactly as the copy stored it. A task matches when its `metadata` holds that key as
  a string equal to it, byte for byte: a plugin compares the string and never parses it. It is
  **optional**: absent is not a filter, and the engine sends it only to a plugin that declared
  `filter_by_origin` native (§4.2). On the command line these two are `task list --metadata
  <KEY>[/<SEGMENT>…]=<VALUE>` (repeatable) and `task list --origin <SOURCE>:<ID>`.

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
with `target`. A create's answer is the only way the engine learns that id, so a create
answered in any other shape is reported to a copy's caller as a destination that may hold
an item the copy could not take back, rather than as a refusal: a plugin that refuses a
create answers `error` and holds nothing.

`url`, `location` (§4.13), `created_at` and `updated_at` are the destination's own and are
never written — where the *source* holds an item says nothing about where this one does.
Everything else `item` carries is, and **nothing is silently dropped**: a field this
source cannot represent, and a metadata key it cannot carry, are each a
`{"kind": "refused"}` naming the field or the keys. A `target` this source does not hold
is refused the same way rather than created, because the engine established that id
before asking.

`TaskSource::check_status_write` — whether a write of an item of one kind at one status
category would have a status to write, asked by a copy before it records what an item held
and overwrites it — is not a message of this protocol. For a hosted source the engine answers
it the way the trait's default does, admitting every status, so a status this plugin has no
name for is refused by `write_task` or `write_project` itself, as above, and a plugin written
against this document implements nothing more.

### 4.9a Image assets on `write_task` and `write_document`

A task's or a document's content references an image **asset** as a Markdown image whose
target is `./<name>` — `![<alt>](./<name>)` — where `<name>` is a bare file name, with no `/`,
no `\` and no `..`, ending, case-insensitively, in `.png`, `.jpg`, `.jpeg`, `.gif` or `.webp`.
Only an image outside code is one: an image whose `!` is escaped with a backslash, and image
syntax inside an inline code span or a fenced or indented code block — as CommonMark decides
what is code, in a list item or a block quote too — is not, and a plugin rewriting references
leaves it byte for byte. Nothing else is a reference, and every other link is content like any
other.

Only a plugin that declared `assets` `"native"` (§4.2) is ever sent a write carrying assets.
Such a write is a `write_task` or a `write_document` whose `params` carry, beside `write`, the
members below. Their names are the members of the `AssetWrite` root `onetaskgraph schema`
emits, and each asset is its `AssetPayload`:

```json
{
  "id": "12",
  "method": "write_document",
  "params": {
    "write": { "target": "D-1", "item": { "id": "notes:design", "…": "…" }, "depends_on": [] },
    "assets": [
      { "name": "before.png", "sha256": "9f86d0…", "content_type": "image/png", "bytes": "iVBORw0KGgo…" },
      { "name": "after.png", "sha256": "60303a…", "content_type": "image/png" }
    ],
    "recorded_assets": {
      "after.png": { "sha256": "60303a…", "url": "https://example.invalid/a/60303a.png" }
    }
  }
}
```

| Member | Type | Meaning |
| --- | --- | --- |
| `assets` | array | One entry per asset the content references, in the order it first references them: the record's **whole** asset set once the write lands, so an asset the record held and this does not name is removed. Present, even empty, on every write that carries assets, and **never** present on any other write — which is what keeps every write a plugin unaware of assets is sent exactly as it was. |
| `recorded_assets` | object | The destination record's existing `onetaskgraph.assets` (below), exactly as the record holds it. Absent when it holds none, and always absent on a create. |

| Asset member | Type | Meaning |
| --- | --- | --- |
| `name` | string | The asset's name, which the content references as `./<name>`. |
| `sha256` | string | The lowercase hex SHA-256 of its bytes. |
| `content_type` | string | `"image/png"`, `"image/jpeg"`, `"image/gif"` or `"image/webp"`, decided by the name's extension alone. |
| `bytes` | string | The bytes, base64 (RFC 4648, with padding). **Absent** exactly when `recorded_assets` records this name with this `sha256`: the destination already serves those bytes, so they are not sent again. |

The plugin stores each asset, and answers with the record and the content it stored:

```json
{ "id": "12", "result": { "id": "D-1", "content": "![before](https://example.invalid/a/9f86d0.png) …" } }
```

| Result member | Type | Meaning |
| --- | --- | --- |
| `id` | string | The `NativeId` this source now holds the record under, as §4.9's `result` says. |
| `content` | string or `null` | The content as stored. |

A plugin keeps the assets one of two ways:

- **Beside the record**, as `local-md` does in a directory beside the record's file. It stores
  each asset's bytes under its name, removes every asset the record held that `assets` does not
  name, and stores — and answers — the content exactly as it was given: `./<name>` already
  names the file beside it. An entry without `bytes` reuses the bytes the record already holds
  under that name, which it refuses when they do not hash to `sha256`.
- **At a URL it serves**, as a hosted backend does. It uploads each entry carrying `bytes`, and
  for each entry without them reuses the `url` `recorded_assets` records. It then writes the
  record with every `./<name>` target replaced by that asset's URL, with **`onetaskgraph.assets`**
  in the record's `metadata` holding what it served — an object keyed by asset name, each value
  `{"sha256": <lowercase hex>, "url": <string>}`, the shape the `AssetUploads` root describes —
  and, when the record's `onetaskgraph.template` entry records a `body_digest` that is still the
  SHA-256 (`sha256:<hex>`) of the content it was given, with that `body_digest` re-recorded as
  the digest of the content as rewritten and the entry's `template`, `digest` and
  `answers_digest` carried as they were. It answers the content it wrote. A Rust plugin does all
  three through `onetaskgraph_plugin_api::serve_asset_references`.

The engine sends a write carrying assets when the record's content references an asset, and
when the record it overwrites holds one — so a copy whose source record no longer references an
asset takes it away at the destination too. Before it sends one, it has already refused, naming
the record and the asset, a reference the source record does not hold, and — naming the source,
the record and the asset — a destination that declared `assets` `"unsupported"`. A plugin must
still refuse, with `{"kind": "refused"}` naming the asset, an entry whose `bytes` are absent
with nothing recorded to reuse, and one whose `bytes` do not hash to its `sha256`.

A create from a template, and a regenerate, carrying assets are template operations, and are
not carried (§4).

### 4.9b `check_task_write`

Sent only to a plugin that answered `write_checks: true` at the handshake (§3.14). It changes
nothing: it says whether this plugin can represent every field of one task write — its status,
its content, the tasks it delivers and every other field a `write_task` (§4.9) of it would
refuse as one it cannot represent.

```json
{ "id": "27", "method": "check_task_write", "params": { "write": { "target": null, "item": { "id": "Q-1", "title": "…", "…": "…" }, "depends_on": [] } } }
{ "id": "27", "result": {} }
```

`params.write` is the `ItemWrite` a `write_task` of the task would carry. The trait method
carries `()`, so `result` is an empty object, and it means the plugin can represent it; `{"kind": "refused"}`, naming the field and why, means it
cannot — in the words its `write_task` would refuse it with. A task copy asks this of every task
it plans before its first write, and leaves out each one refused: answered `failed` with that
refusal as its reason, while the rest land as one copy. Any other error fails the whole copy, as
a failed write would. It is a promise about representability alone: a write the check accepted
may still fail, and a copy then takes back everything it wrote.

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
`updated_at`, its `metadata`, its `repositories`, its `classification` (§4.13c) and its
optional `version` (§4.13d), each on the terms the `Task` member of the same name is read on. It carries **no status and no dependencies**, and both absences
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

### 4.13a A task's `key`

The short handle the backend shows people, beside a `Task`'s `id` and never instead of it:

```json
{ "id": "I_kwDOAbc123", "key": "1043", "title": "Rate-limit the sync loop", "…": "…" }
```

The member is **optional** and an absent one means `null`, which says *this backend has no
short handle for this task*. A plugin that has one sends it — Linear's `ENG-123`, a GitHub
issue's number alone as a decimal string, `1043` and never `owner/repo#1043` — and one that
has none sends nothing rather than a copy of the `id`.

It is **human-facing and it may change**: an issue moved between teams or transferred
between repositories gets a new one. So `id` stays what the engine and every consumer
store and match on, and nothing resolves a task by its key.

<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Held by behaviour
     rather than by words: `an_update_keeps_the_destinations_own_key_whatever_the_incoming_task_carried`
     (github-projects), `a_write_carrying_a_key_lands_and_the_item_reads_back_without_one`
     (in-memory) and the copy journey's absent `key` each fail if a written key is stored. -->
It is **read-only**. A plugin derives it on a read and never stores one it is handed: an
`ItemWrite` whose task holds one is read exactly as one that does not.
A `Project` and a `Document` have no `key` at all.
<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->

### 4.13b A task's `priority`

How much a task matters, as one of five values: `"none"`, `"urgent"`, `"high"`, `"medium"`
and `"low"`.

```json
{ "id": "ENG-1", "key": "ENG-1", "title": "Rate-limit the sync loop", "priority": "high", "…": "…" }
```

The member is **optional** and an absent one means `"none"`, which says *no priority is set*
— and is what a source that cannot hold a priority reports for every task. The engine always
sends it, so a reader never has to tell an absent member from a `"none"` one; a plugin written
before this member omits it and is read as the source without priorities it is.

It is written, unlike `key`: a `write_task` carries the priority the item should hold, and
`"none"` clears one. A plugin that declared `priority` unsupported (§4.2) is never handed a
task carrying another value, so it never has a priority to drop. A `Project` and a `Document`
have no `priority` at all.

### 4.13c A record's `classification`

Who may read a task, a project or a document: `"public"` or `"private"`.

```json
{ "id": "ENG-1", "title": "Rotate the signing key", "classification": "private", "…": "…" }
```

The member is **optional** on all three, and an absent one means `"public"` — which is how
every record written before it existed reads. The engine leaves it out of what it sends while
the record is public, so a plugin written before it existed is sent exactly what it was sent
before, and it is sent `"private"` only for a record that is. A plugin stores what it is sent
and reports what it stored; a backend with no field of its own keeps it under the reserved
metadata key `onetaskgraph.classification` while it is `"private"` and nowhere while it is
public, and reports it as this member rather than as metadata.

It is the record's *declared* classification. The engine tightens it before it writes — a
record naming a private repository, or filed under a private project, is private however this
member reads — and never loosens it. Where a private record may be written is the engine's to
decide: it writes one only to a source declared `visibility: private` whose own live read,
`TaskSource::visibility`, answers that it is private; a source hosted over this protocol
answers `unknown` there (see the paragraph after the method table), so a private record is never
written to one.

### 4.13d A record's `version`

A `Task`, a `Project` and a `Document` each carry one more optional member, `version`: a
string, the record's **change token**. Two equal versions of one record from one source mean
nothing has changed in any part of it the source declares its token covers (§4.13e names those
parts, kind by kind) — for a task that is its title, body, status, priority, labels, board
fields and metadata, and, where covered, its comments and its forward edges. A plugin that
cannot promise that for a record leaves the member out, which the engine reads as *always read
it*, and a plugin written before the member existed is read exactly that way: it is optional
under §2.1, and was added without a protocol version bump.

It is **opaque** to every caller. The one reader that looks inside is the source that spelled
it: the engine orders two versions of one record (`Engine::compare_versions`) by asking that
source's `TaskSource::order_versions`, and answers `equal`, `later`, `earlier` or
`incomparable` — the last for versions of different records, a source it does not hold, a value
the source cannot parse, and two different versions a token says differ without saying which
came first. The protocol carries no ordering method, so a plugin hosted over it has its tokens
compared for equality alone: one token is one version, and two different ones are unordered,
which a copy reads as changed and never as unchanged.

A plugin derives its token on a read and never stores one it is handed: a record arriving on a
write may still hold its source's token, and a destination ignores it.

What each plugin of this build keys its token on is stated here once, as what the journeys
named below hold it to:

<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Held by the journeys it names: every row of the GitHub member table below names its leg in `every_change_kind_moves_the_changed_tasks_token_and_no_other` in `crates/onetaskgraph-e2e/tests/e2e/change_tokens.rs`, which makes that change to one loopback board task and holds its token to moving (to a later one, or for a transfer to an unordered one) and both ends of a relationship with it, and in `every_change_kind_moves_a_tasks_token` and `every_facet_change_moves_a_projects_and_a_documents_token` in the GitHub Projects plugin's credentialed `tests/journey`, which re-take each live leg against GitHub; the `gh3` facet rows are held on the loopback by `a_label_or_option_renamed_or_recoloured_is_read_again_for_a_task_a_project_and_a_document`, `a_far_end_deleted_lowers_the_count_and_the_pinned_read_is_never_the_entry`, `an_items_own_kind_change_is_never_answered_with_the_entry_of_its_old_kind` and `a_far_ends_kind_flip_or_retitle_into_a_document_is_read_from_the_board_in_every_edge` in `crates/onetaskgraph-github-projects-e2e/tests/e2e/cache_kinds.rs`; every `lin1` row by `every_change_measured_or_covered_by_construction_is_read_again_pinned_to_the_new_token`, `a_relation_added_removed_or_whose_far_end_is_trashed_reads_as_an_uncached_read_does` and `a_token_linear_cannot_vouch_for_is_never_kept_or_served` in `crates/onetaskgraph-linear-e2e/tests/e2e/cache.rs` and the comment, inverse-relation and project-relation rows by `every_comment_and_relation_change_is_read_again_pinned_to_the_new_token`, `a_comment_or_edge_read_its_token_cannot_vouch_for_is_never_kept_or_served` and `a_comment_authors_renamed_display_name_is_the_one_member_served_older_than_its_token` in `cache_parts.rs` beside it, and by `every_field_change_moves_a_linear_tasks_token` in the Linear plugin's credentialed `tests/live.rs`; `a_recopy_by_query_of_unchanged_board_items_skips_each_and_sends_nothing_but_the_listing` in `crates/onetaskgraph-e2e/tests/e2e/copy_skip.rs` makes the task changes and holds the copy to reading exactly the tasks each reaches; `two_digests_of_one_task_file_are_equal_until_it_changes_and_then_incomparable` in `crates/onetaskgraph-core/tests/version.rs` holds the folder's; `two_edits_inside_one_second_are_never_served_from_a_skip` and `a_board_read_without_the_servers_clock_gives_no_token_and_is_never_skipped` in that `copy_skip.rs` hold the settling rule; `a_later_time_beside_fewer_relationship_events_is_no_version_read_after_the_other` in that `change_tokens.rs` puts a crossed pair to the engine's own ordering; `a_destination_recorded_under_the_tokens_older_spelling_is_read_again_once_and_then_skipped` in that `copy_skip.rs` and `an_entry_a_release_before_gh3_left_is_read_again_rather_than_served` in that `cache_kinds.rs` hold the older spellings; and the plugin's `tests/point_cost.rs` holds the unchanged prices. The spike evidence each table cites is the two spike branches' retained session records. -->
| Plugin | Tasks | Projects | Documents |
| --- | --- | --- | --- |
| `github-projects` | `gh3` (below) | `gh3` | `gh3` |
| `linear` | `lin1` (below) | `lin1` | `lin1` |
| `local-md` | `md1\|<SHA-256 of the task file's bytes>`: everything a task is lives in its one file, so every kind is covered; two different digests say the file changed and never which came first | none | none |
| `in-memory` | none | none | none |

#### `gh3`, a GitHub board's token

`gh3|<issue updatedAt>|<board item updatedAt>|<comment count>|<relationship events>|<facets>|<repository>`,
every part spelled from what every list, search and detail read already selects — nothing is
added to any request. Tasks, projects and documents are all issues on the board and share it.

- **The times, the comment count, the relationship events and the repository** are exactly
  `gh2`'s, below. A token is given only once the item has settled (below), and never for a
  draft or an item with any part missing.
- **`<facets>`** is the lowercase hex SHA-256 of the canonical JSON
  `{"kind":<"task"|"project"|"document">,"fields":[[<field name>,<option name or text>],...],"labels":[[<id>,<name>,<color>],...]}`,
  each list sorted by its first element: what a person can change about an item without moving
  either time. `kind` is the item's own kind as the plugin derives it from that response — the
  `DESIGN: ` title prefix, `subIssuesSummary{total}` above zero — rather than the raw total, so
  a project gaining a task does not move it. `fields` and `labels` come from the item's
  `fieldValues` and its `labels(first:$nestedFirst)`; a connection reporting `hasNextPage` gives
  no token.
- **Ordering.** Two tokens whose repositories, facets or relationship counts differ are never
  ordered. Otherwise the later version is the one with the later or equal times and the
  greater or equal comment count, as `gh2` ordered them; two crossed are not ordered, and two
  are `equal` only when every part is. A `gh1` or `gh2` token is never ordered, so an entry or
  a copy record a release before `gh3` wrote reads its item once more.

**Why `gh3` replaced `gh2`.** `spike-coverage` (branch
`nick/otg-cache-everything-versioned-plan/spike-coverage`, sessions 1 at `a978016c` and 2 at
`eae332e9`, on the nominated board) made each change below to a real project, document and task
and read the `gh2` token before and after:

| Change | `gh2`, measured | `gh3` | Live leg |
| --- | --- | --- | --- |
| a label renamed, on the label rather than the item | equal: stale, 4 of 4 (a project and a document, each session) | the facets move | `its label renamed` |
| a label recoloured | equal: stale, 4 of 4 | the facets move | `its label recoloured` |
| a board single-select option renamed, its id kept | equal, 1 of 1 (session 2) | the facets move | `a scratch field option renamed` |
| a native edge's far end deleted | the relationship count fell, so the new token ordered *earlier* and the held entry read as fresh: 3 of 3 | a changed count is never ordered | `its native far end deleted` and `its project deleted` |
| an item's own kind changed (a task gaining a sub-issue) | equal: stale, 1 of 1 | the facets move | `after it gained a sub-issue` |
| a far end's kind flipped, or retitled into a document | equal: stale, 1 of 1 each | not covered: an item's token cannot attest its far ends, so forward edges are always read (§4.13e) | — |
| native edges added, removed or replaced; edges, classification and metadata in the body slot; a document refiled; a document's content and asset manifest replaced; a label deleted | later: covered, every case | later | the task legs below |
| a field the item held a value in removed from the board | later with nothing stored changed: a conservative miss | later | — |

Covered by construction: a repository transfer, and the `url` and `location` derived from the
repository, because the token embeds it (`a repository transfer`, on the loopback); a direct
title edit and a project's status change rest on the task legs `a title edit` and `a status
change`, since a project is an issue whose title and `Status` field move the same times.

**What keeps each member of a GitHub task fresh.** Every member a copy writes is covered by a
part of the token GitHub moves for it, and the leg named beside it — the same name in the
loopback journey and in the credentialed one, unless the row says otherwise — makes that change
alone and holds the token to moving:

| Member | Where the board keeps it | Kept fresh by | Leg |
| --- | --- | --- | --- |
| title | the issue's title | the issue's `updatedAt` | `a title edit` |
| content | the issue's body | the issue's `updatedAt` | `a body edit` |
| metadata, `delivers`, `delivered_by`, classification, named repositories, asset references | the body's metadata slot (`docs/metadata.md`) | the issue's `updatedAt`: the slot is bytes of the body, so any change to it is a body edit | `a metadata change` |
| labels | the issue's labels | the issue's `updatedAt`; a label's name or colour changed on the label, the facets | `a label change`; `its label renamed` and `its label recoloured` |
| status | the `Status` field, and the issue's state | the board item's `updatedAt`, and the issue's for a close or reopen; an option renamed, the facets | `a status change`; `a scratch field option renamed` |
| priority | the `Priority` field | the board item's `updatedAt` | `a priority change` (live, on a board with a `Priority` field) |
| projected metadata, `onetaskgraph.origin` | board text fields | the board item's `updatedAt` | `a board text field change` live; `the origin field rewritten` and `a project field change` on the loopback |
| its own kind | the title prefix and its sub-issues | the facets | `after it gained a sub-issue` |
| project | the issue's parent | the relationship events — `PARENT_ISSUE_ADDED_EVENT` and `PARENT_ISSUE_REMOVED_EVENT`, on the task. A project's own token does not count the sub-issue events GitHub records on it | `taken out of its project` and `filed under another project` live, each sent as the project's own `removeSubIssue` or `addSubIssue`; `filed under a project`, `moved to another project` and `taken out of its project` on the loopback |
| dependencies | the issue's `blockedBy` | the relationship events — `BLOCKED_BY_ADDED_EVENT` and `BLOCKED_BY_REMOVED_EVENT` on the dependent task, `BLOCKING_ADDED_EVENT` and `BLOCKING_REMOVED_EVENT` on the one it depends on. A far end deleted takes its events with it, which the count says by being unordered | `a dependency added`, `a dependency removed` and `a dependency replaced`; `its native far end deleted` |
| comments, under `--comments` | the issue's comments | the issue's `updatedAt` for one added or edited, the comment count for one deleted | `a comment added`, `a comment edited` and `a comment deleted` live; `a new comment`, `an edited comment` and `a deleted comment` on the loopback |
| repository, when the slot names none | the issue's repository | the token itself: it carries the `owner/name` every read returns, so a transfer leaves no two tokens equal, and two tokens naming different repositories are never ordered | `a repository transfer`, on the loopback alone: the lane holds one scratch repository and refuses the core one, so it has nowhere to transfer to. What it proves needs no live leg, because the token holds the very value the read returned |

A project's and a document's members are the same issue's: title, content, metadata, labels,
status and repositories are kept fresh exactly as a task's, and a document's project and its
asset manifest — recorded in the body's metadata slot — by the relationship events and the
issue's `updatedAt`. A document filed under a deleted project reads unordered, which
`its project deleted` takes live.

**GitHub moves neither time nor the comment count for a relationship.** The credentialed lane
added a dependency to a real issue and read all three where they were, so a token of those
three alone read an edge change as no change, and a copy skipped the task with its edges stale.
So the token counts the issue's timeline events of the six relationship types above
(`timelineItems(itemTypes: [...]) { totalCount }`), and a project membership — a parent issue
— is counted the same way rather than trusted to a time. Each relationship added or removed is
one more event, a replacement or a move between projects two. A dependency's types are counted
on both its ends, so one a person makes from the far end still moves this end's token — and the
far end's token moves with it, which reads that task again once and never serves either stale.
A membership is counted on the task, where GitHub records it whichever of the two issues the
move was made from: the live legs make it from the project's side. An issue deleted takes its
events off the issues it was related to, which is why two different counts are never ordered.
The types are pinned in the plugin's `tests/fixtures/schema.graphql`, which its schema test
holds every document's default to, and the credentialed lane holds GitHub to the pin.

The count is a connection selected for its `totalCount` alone, with no page size, exactly as
the comment count beside it is, and the offline model `tests/point_cost.rs` prices documents by
adds nothing for it, nor for the facets, which are spelled from what each read already selects:
every document's recorded price is what it was before, and the credentialed lane reconciles
each against GitHub's own `cost`.

#### `lin1`, a Linear item's token

Every part is spelled from the response the plugin already receives; only a cache attached to
the source (§4.13e) widens a request.

- **A task:** `lin1|<Issue.updatedAt>|<facets>[|c:<comments>|r:<relations>|i:<inverse>]`, the
  facets being
  `{"key":<identifier>,"labels":[[<id>,<name>,<color>],...],"project":<id|null>,"state":[<name>,<type>],"url":<url>}`.
- **A project:** `lin1|<Project.updatedAt>|<facets>[|r:<relations>|i:<inverse>]`, the facets being
  `{"labels":[[<id>,<name>,<color>],...],"status":[<name>,<type>]}`.
- **A document:** `lin1|<Document.updatedAt>|<facets>`, the facets being
  `{"content":<SHA-256 of its content>,"project":<id|null>,"title":<title>,"url":<url>}`.
- Each facets part is the lowercase hex SHA-256 of that canonical JSON, labels sorted by id.
- **The bracketed parts are spelled only with the cache attached, and all of them or none.**
  Each is the lowercase hex SHA-256 of the canonical JSON of one first page's rows, sorted:
  - `c:` — a task's comments, `[[<comment id>,<updatedAt>],...]`, from
    `comments(first:10){nodes{id updatedAt} pageInfo{hasNextPage}}`; a detail read spells it
    from the `comments(last:…)` page it already reads, when that page was not resumed from a
    cursor and reports no earlier one;
  - `r:` — a task's relations, `[[<relation id>,<type>,<relatedIssue id>],...]`, from
    `relations(first:10){nodes{id type relatedIssue{id}} pageInfo{hasNextPage}}`, or a project's,
    `[[<relation id>,<type>,<relatedProject id>],...]`, from
    `relations(first:10){nodes{id type relatedProject{id}} pageInfo{hasNextPage}}`;
  - `i:` — a task's inverse relations, `[[<relation id>,<type>,<issue id>],...]`, from
    `inverseRelations(first:10){nodes{id type issue{id}} pageInfo{hasNextPage}}`, or a project's,
    `[[<relation id>,<type>,<project id>],...]`, from
    `inverseRelations(first:10){nodes{id type project{id}} pageInfo{hasNextPage}}`: the relations
    other items record with it, which is what a `DependedOnBy` read answers.
- **Completeness.** A labels connection names no `first` and no `pageInfo`, so a response
  returning 50 or more label nodes — Linear's default page — gives no token, and so does one
  whose comments, relations or inverse relations page reports another page or holds more than
  10 rows. Nothing is selected to learn more. So, with the cache on, a task with more than ten
  comments, relations or dependents, or a project with more than ten relations, is read every
  time.
- **Ordering.** Two tokens whose kinds, facets, or presence or value of any bracketed part differ
  are never ordered; otherwise the later `updatedAt`, as an instant, is the later version, and
  two are `equal` only when every part is. A value of any other spelling is never ordered —
  among them the spelling 0.6.1 gave with the cache attached, a task's relations part alone and a
  project's none, so an entry a release of that spelling wrote is read again once. No settling
  margin applies: the spike measured none.

**Why `lin1` has those parts.** `spike-linear-coverage` (branch
`nick/otg-cache-everything-versioned-plan/spike-linear-coverage`, three sessions on a scratch
team at 0.6.0 — 1 at `c3e00cbd`, 2 at `52a59149`, 3 at `087d4097`, which priced the documents
below) made each change and read each part before and after:

| Change | `updatedAt` | What moved | Steps |
| --- | --- | --- | --- |
| an issue's title, description, metadata slot or priority | moved | — | s01–s03, s07 (session 1) |
| an issue's label added or removed, its state changed, or its project removed or added | moved | the facets too | s09, s12, s33, s34 (session 1); s04, s06 (session 2) |
| a label renamed or recoloured, a workflow state renamed | **missed, 3 of 3** | the facets | s10, s11 (session 1); s05 (session 2) |
| a `blocks` or `related` relation added | **missed, 3 of 3** | the relations digest, on all 10 adds and removes | s26, s28 (session 1); s26 (session 2) |
| a `blocks`, `related` or `duplicate` relation added or removed, read on the issue it names | moved, 8 of 8 | that issue's inverse relations digest, 8 of 8 | s26–s31 (session 1); s26, s27 (session 2) |
| a comment added, edited or deleted | **missed, 0 of 3 moved** | the comments digest | s15–s17 (session 1) |
| a project's name or content | moved | — | s35, s36 (session 1) |
| a project's status changed, a label added or removed | moved | the facets too | s37–s39 (session 1) |
| a project relation added or removed | **missed, 0 of 2 moved, on either project** | the near project's relations digest, and the far project's inverse relations digest, 2 of 2 | s40, s41 (session 1) |
| a document's title, or its project | moved | the facets too, for its project | s42, s45 (session 1) |
| a document's content, or an asset reference in its slot | **missed, 4 of 4** | the content digest the facets carry | s43, s44 (sessions 1 and 2) |

Covered by construction, because the facets embed the value itself: an issue's identifier and
url after a team move, a project's label renamed or recoloured and its status renamed, a
document's title and refiling, and a workflow state renamed. The loopback journeys make every
row's change and every one of those, and hold a read pinned to the new token to the changed
value.

**The one member a token does not cover: a comment author's display name.** `c:` digests each
comment's id and `updatedAt`, which move for a comment added, edited or deleted, and the comment
set; an author renaming themselves in Linear moves neither, nor any time of the issue. Renames
are rare and the name is a label for a person rather than work, so by the user's ruling of
2026-10-14 a comment served from the cache may name its author as they were named when the entry
was filled — the one member a served read may answer older than its token. Every other member of
a comment — its id, body, creation and update times, and which comments there are — is served no
older than the caller's token. **A task's reverse edges are covered by `i:` by construction** —
it digests the very rows a `DependedOnBy` read answers — and the issue a relation names moved its
`updatedAt` too on each of those 8 changes. **A project's reverse edges are covered by its `i:`
part alone**: neither project's `updatedAt` moved for a project relation added or removed (0 of
2), and the far project's inverse relations digest moved on both.

**What a copy's skip takes a token as vouching for.** A repeat copy, or a copy by query, skips a
task whose source token is the one its destination counterpart records only when the source's
`copy_version_coverage` covers every part the copy would write: the task's fields, its forward
edges, and its comments when the copy carries them. It defaults to the cache's
`version_coverage` and, like it, is read in process and never carried over this protocol, so a
task read from a hosted plugin is never skipped. So a copy out of a Linear source with no cache
never skips — its token then covers neither forward edges nor comments — and one with the cache
on skips an unchanged task, `--comments` or not, since its token's `c:` and `r:` parts then move
for every comment and relation change; a comment author renamed since the last copy is the one
change such a skip does not carry, as above. A folder of Markdown covers every part:
`md1` digests the task's one file. A GitHub board reports every part for a copy's skip, though
its cache serves no forward edge: the relationship count moves for an edge added or removed and
for a far end deleted, so its token sees every change to a task's edges but one. **That one
bound is a far end's own kind flipping** — a far end gaining a sub-issue, or
retitled into a document — which moves nothing of the task that depends on it, measured stale 1
of 1 by `spike-coverage`: a skipping recopy keeps that edge as it was, and the next copy that
reads the task carries it (`a_far_end_turned_into_a_project_is_carried_by_the_next_copy_that_reads_the_task_after_it`
in `crates/onetaskgraph-e2e/tests/e2e/copy_skip.rs`; the Linear journeys are in
`crates/onetaskgraph-linear-e2e/tests/e2e/copy_skip.rs`).
<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->

**Why a GitHub token waits for its task to settle.** GitHub reports both of the token's times
to the whole second. A change made in the same second as the state a read saw would move
neither time, and, with the comment count unmoved, the token would read the same — so a copy
that recorded it would skip the task and keep content that had changed. The plugin therefore
gives a token only when the task's later time is more than `change_token::SETTLED`, 60 seconds,
before the server's own clock on the response that read it, its `Date` header. GitHub aborts a
GraphQL query that runs past ten seconds, so the state a response reports was read at most
about ten seconds before that `Date`; a change after the read then always falls in a later
second than the token records, and moves it. A task changed within the minute before a read,
and every task read from a response carrying no `Date`, has no token and is read in full; once
it settles, the next read gives it one. Every reader that keys on this token inherits the
rule — a copy's skip, and the board cache built on the same token.

### 4.13e The store's local cache

A source configured with `cache.enabled` has its tasks with their comments, a task's forward and
reverse edges, its projects with their forward and reverse edges, its documents, and the verified
bytes of
image assets kept on the host the engine runs on, keyed on the `version` of §4.13d and on
nothing else. The rule it may never break is that **it never serves stale data**, and its cost
is held over an hour rather than per read: over an hour of the assumed workload it sends fewer
requests than no cache (`linear-cache-cost-not-above-uncached`, and on GitHub
`github-cache-points-not-above-uncached`), though one read may cost more than it did — a widened
query, a Linear projects listing of more than 74 rows taking one more page, or a copy's one
page of projects (below) that pins none of the projects it files under. An entry is
served only when the version a
caller pinned — explicitly, or by a listing in the same command — is that entry's version or an
earlier one, by the source's own `order_versions`, and never when the two are `incomparable`.
Nothing asks a source whether a record changed. **A pin means "at least as fresh as the version
I hold", not "current"**, so a pinned read answered from the cache can predate a change at the
source nobody has read yet; that is the contract, and a caller wanting the source's present
state reads unpinned.

What a plugin says to the cache, through `TaskSource` and never over this protocol — a
subprocess-hosted plugin has no remote identity, so the engine keeps no cache of it:

- `version_coverage`, `project_version_coverage` and `document_version_coverage` — which parts
  of each kind its token covers: a task's fields, comments and forward edges; a project's
  fields and forward edges; a document's fields and its asset manifest. Each member a plugin
  leaves out reads as `Uncovered` with a reason, and the defaults leave every part uncovered. A
  part it does not cover is never served: the read goes to the source and its `cache.bypassed`
  says why. A record its source gives no token — a draft, a GitHub item changed within the
  minute before its read, a Linear item whose labels, comments, relations or inverse relations
  came back as only a first page — is read every time.
- `attach_cache` — called exactly once, when the engine wraps the source in its cache, before
  any read and before any coverage is read. A source not overriding it behaves exactly as
  before. `linear` uses it to widen the seven documents named below, which is what lets its
  task tokens carry their comments, relations and inverse relations parts and its project tokens
  their relations part, and so cover a task's comments and edges both ways and a project's
  forward edges.
- `forward_edges_version` — the token the response that answered the source's last
  `DependsOn` read of a task spelled, so a dependency read can fill an entry when nothing else
  in the command read the task. Those edges are served only to a pin that version is at or
  after, and never vouch for the task's own fields. Defaulted to none.
- `edges_version` — the same, for any item kind and either direction: the token the response
  that answered the source's last first-page dependency read of that item in that direction
  spelled. A project's forward edges and a task's reverse ones are kept only at a version it
  gives, and served only to a pin that version is at or after. Defaulted to
  `forward_edges_version` for a task's forward edges and none otherwise.
- `reverse_edges_coverage` — whether a task's or a project's token vouches for its reverse
  edges, which are recorded on the items that depend on it. Defaulted to not covering them, with
  a reason.
- `remote_identity`, on `SourcePlugin` — what names the remote it reads, which the cache
  directory is a digest of.
- `write_context`, `board_context`, `prime_write` and `unprime` — opaque records the cache
  keeps beside a task and beside the board, which `prime_write` takes back for a write
  expecting that version, and `unprime` drops again so the next write reads afresh.
- `begin_journal` and `end_journal` — every request sent while one write runs, each with what it
  came to and, for a mutation, the version and the whole task its own response shows.

**Every read, and why the ones kept remote are.** A pin comes from the caller —
`task show`, `task comment list`, `task show-many`, `task deps`, `project show`, `project deps`
and `document show`, each with `--pin`, or `Engine::pin`, `Engine::pin_project` and `Engine::pin_document` — or from a
listing earlier in the same command. A listing only pins: it never fills an entry. An entry is
filled lazily, by a load, and nothing is pre-warmed.

<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Held by the journeys that drive each row through the binary: `crates/onetaskgraph-github-projects-e2e/tests/e2e/cache.rs`, `cache_kinds.rs`, `cache_assets.rs` and `cache_writes.rs` for the board, and `crates/onetaskgraph-linear-e2e/tests/e2e/cache.rs` — whose `every_linear_read_kept_remote_reaches_linear_every_time_and_costs_what_it_did` makes each remote read twice and counts both — `cache_parts.rs`, whose `every_comment_and_edge_read_is_a_cold_load_a_hit_on_an_unchanged_pin_and_a_miss_on_a_newer_one` drives each newly served Linear read, and `cache_documents.rs` for Linear, with `a_declared_sources_verification_reaches_linear_with_its_cache_on_as_it_does_without` in `linear_boundary.rs` for `visibility` and `every_health_read_through_the_plugin_host_reaches_linear_with_the_cache_on_or_off` in `health.rs` for `health`; the reasons are the plugins' own `Uncovered` strings, which every bypassed read prints. -->
| Read | `github-projects` | `linear` |
| --- | --- | --- |
| a task's fields: `task show`, `task show-many`, a copy's loads | served | served |
| a task's comments | served | served, with the cache attached; the comments part is what vouches for them, all but each author's display name (§4.13d) |
| a task's forward edges (`DependsOn`), native and recorded | remote: an item's token does not attest a far end's kind, whether it became a document, or its deletion (a far end's kind flip and its retitle into a document each left the near token equal, 1 of 1) | served, with the cache attached; the relations part is what vouches for them |
| a project's fields | served | served |
| a project's forward edges | remote, for a task's reason | served, with the cache attached; the project's relations part is what vouches for them |
| a document's fields | served | served |
| a document's asset manifest, and a task's | served, at the version its record was served at | remote: Linear's asset reads send no request, so there is nothing to save |
| an asset's bytes | served by digest, below | — |
| a task's reverse edges (`DependedOnBy`) | remote: a reverse edge is another record's | served, with the cache attached; the inverse relations part is what vouches for them |
| a project's reverse edges | remote | served, with the cache attached; the project's inverse relations part is what vouches for them |
| `query_tasks`, `query_projects`, `query_documents` | remote: they pin, and fill nothing | remote |
| `labels`, `health`, `visibility`, and Linear's `status_names` | remote | remote |
<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->

**What `attach_cache` widens on Linear, and what it costs.** With the cache attached, `ISSUES`
and `ISSUE` gain exactly the comments, relations and inverse relations pages of §4.13d,
`ISSUE_DETAIL` the relations and inverse relations pages — its comments part is spelled from the
comments page it already reads — and `PROJECTS` and `PROJECT` the project relations and inverse
relations pages, so
every response that fills or pins an item carries the same parts. `ISSUE_RELATIONS` gains exactly
`updatedAt`, `identifier`, `url`, `state{name type}`, `labels{nodes{id name color}}`,
`project{id}` and the comments page, and `PROJECT_RELATIONS` exactly `updatedAt`,
`status{name type}` and `labels{nodes{id name color}}`, the tokens' other parts, so a dependency
read in either direction can fill; each asks for at most 10 rows a page, the most a token
vouches for, so an item with more takes a request a page. `PROJECTS` asks for at most 74 rows a
page. Every other
document is unchanged — `ISSUE_COMMENTS` among them, since a caching source's comment listing is
the detail read, which already fills the comments and the fields together — and so is the read a
write makes of its own target, whose answer fills nothing. With no cache attached each is byte
for byte the one 0.6.0 sent. Linear's complexity for each, against its per-query cap of 10,000,
priced from `spike-linear-coverage`: the plain documents, the relations page on `ISSUE` (+25),
the comments page on each `ISSUES` row (+3) and the relations page on each `PROJECTS` row asked
for (+25) are measured; each inverse relations page is taken as its relations page's 25, the
comments page as the same 3 wherever it is added, a project's relations page on `PROJECT` as on
`PROJECTS`, and `PROJECT_RELATIONS`' token parts as the 3 `ISSUE_RELATIONS`' were measured at:

| Document | Plain | Widened |
| --- | --- | --- |
| `ISSUES`, at 100 rows | 8 + 6 a row: 608 | 8 + 59 a row: **5,908** |
| `ISSUE` | 6 | 59 |
| `ISSUE_DETAIL`, at `last:100` | 11 | 61 |
| `ISSUE_RELATIONS` | 522, at `first:100` | 56, at `first:10` |
| `PROJECTS` | 70 a row asked for: 7,000 at 100 | 120 a row asked for, at most 74: **8,880** |
| `PROJECT` | 69 | 119 |
| `PROJECT_RELATIONS` | 522, at `first:100` | 53, at `first:10` |

So a widened query costs more than the one it replaces, and one listing of more than 74 projects
takes one request more; the cache earns both back by the reads it answers.

**A copy's one page of projects.** A copy files each task or document under its source project's
counterpart, which reads that project by id to follow its link. When the copy's source caches,
and the items it files name at least two of that source's projects it has not already looked
up in the command, the copy reads the first page of that source's projects once, which pins
every project it carries, so each of those reads is answered from the cache while unchanged. It
reads one page and no more, and the rest are read by id as before, so it costs at most one request
over reading each project, and only when that page pins none of them. A listing narrowed to the
projects the copy names would make the saving hold at every workspace size; it needs an id
predicate the engine's `ProjectQuery` does not have. The
`linear-cache-cost-not-above-uncached` budget holds it to sending strictly fewer requests than no
cache — what Linear's key runs out of first, at 2,500 an hour per user — over an hour of three
hosts, each a cold and a warm pass over 10 projects, 40 documents and 60 issues, and at ten times
that; and `linear-cache-largest-query-complexity` holds the dearest single query any read sends
with the cache on to 9,000, 90% of the cap. Four more hold the cache-on hour to absolute shares
of the key, each its measured figure plus 20% within a ceiling the user set:
`linear-hourly-requests-1x` (250 requests, ceiling 250), `linear-hourly-requests-10x` (2,350,
ceiling 2,500), `linear-hourly-requests-10x-warm` (750, every pass warm, ceiling 1,250) and
`linear-hourly-complexity-10x` (675,000 of the key's 3,000,000 complexity, ceiling 750,000). On GitHub nothing is widened:
`github-cache-points-not-above-uncached` holds every kind, sequence and scale to adding no
point.

**An asset's bytes are kept by their SHA-256.** A byte entry is written only when the bytes a
read returned hash to the digest the manifest names, and served only when they still do, so a
file truncated, corrupted or replaced is read again from the source — with no request to
validate it first. Bytes are served only for a manifest entry the cache served at a covered
version: the manifest says which bytes, and the digest says they are those bytes. Two records,
or two versions of one, naming the same digest share one entry.

**How large it grows.** Each record is one file, written whole and replaced in place, so the
item entries hold at most one file per task, task's forward edges, task's reverse edges,
project, project's forward edges and document a load has ever read — the board's or team's size at most, and usually what was loaded rather than
what exists. Byte entries hold one file per distinct digest ever read and are never replaced,
so an image edited ten times leaves eleven. Nothing expires by age and nothing caps the total.
At the GitHub budget's workload a pass loads 5 projects, 20 documents and 20 tasks, at most 45
item files, and its 8 images of 256 KiB are 2 MiB of bytes, growing by 256 KiB for each image
replaced; at ten times the size, 450 files and 20 MiB. At the Linear budget's, at most 60
task, 60 forward edge, 60 reverse edge, 10 project, 10 project forward edge, 10 project reverse
edge and 40 document files, and 2,500 at ten times. **`onetaskgraph cache
clear [<SOURCE>]` reclaims a source's item and byte entries together**, removing only files
this store wrote.

**A write advances an entry only to what its own response vouches for.** An entry holds the
whole task — fields, metadata, comments, assets — at one version, so it moves to a write's
version only when that write's last response carries every one of those parts at that
version. A response carrying only the fields it wrote, with a version, leaves the entry
unserveable: a person's concurrent edit to another field, or a new comment, can share that
version, and an entry labelled with it that lacked them would be served stale. A response that
determines no version leaves it unserveable too. A `github-projects` write made with `--expect`
selects the whole issue in its last mutation's payload for exactly this
(`graphql::whole_twin`); `deleteIssueComment` answers with nothing about the issue, so a
comment delete never advances an entry. **Against GitHub itself a write's `result.version` is
normally `null`**: the token waits for a task to settle (§4.13d), and a mutation's response is
stamped seconds after the change it made, so the entry is left unserveable and the next read
refreshes it. Every write without `--expect` leaves its task's entry unserveable.

**An entry is taken out of service before its write is sent, or the write is not sent.** The
store removes the task's entry first — or, on a Unix host where the cache directory refuses
that, empties the file in place, making it writable when only its mode refuses, and never
through a symlink — so no other process pinned to the old version is served it once the write
lands. An entry nothing can remove or empty (a read-only mount, another owner's file, a
symlink) refuses the write, naming the entry and the directory, before any request is sent —
including when this process cannot open it, which says nothing about whether another can. Only
an entry shown absent (nothing at its path, or a file where one of its directories belongs)
lets a write go without retiring it.

**Every write that can change a project, a document or an asset takes their entries out of
service the same way** — every entry naming the item, of every kind, since a write can turn a
task into a project or a document: a create, an update, a metadata write, a render, an asset
write, a deletion, and a copy's rollback. No such write fills an entry from its own response;
the next pinned read does. A byte entry is never retired, because it names bytes rather than a
record: a manifest that no longer names its digest is what stops it being served.

#### `--expect`: a write that sends only its mutations

`--expect VERSION` on `task comment add|edit|delete`, `task update`, `task priority set`, `task
status set`, `task metadata set`, `task content set`, and a `task copy` of one task onto a
caching source, says the caller knows the task at that version. The write takes from the cache
entry at that version or a later one what it would otherwise read: the issue's node id, its
board item, the board's field and option ids — kept once per board — the comments on the issue
and the current values its rule compares against. It then sends **only its mutation
requests**, and only the fields whose value differs from the expected version: `task priority`
sends the priority alone, `task status` the status alone, and `task update` and a bound copy
each field that differs among title, body, status, priority, labels and metadata. A person's
concurrent edit to a field the write does not set is never touched.

| Verb, with `--expect` and the task and its board cached | Requests |
| --- | --- |
| comment add, edit or delete | 1: ADD_COMMENT, UPDATE_COMMENT or DELETE_COMMENT — no ISSUE, no COMMENT_ISSUE |
| priority, status (no close or reopen), content, metadata | 1: the one mutation |
| update, bound copy | one per part whose fields differ: UPDATE_FIELDS, then UPDATE_ISSUE |
| any of them, after `cache clear` or with the entry older than expected | one read (ISSUE), then the mutations above |
| any of them, a cached option or id re-minted since it was cached | the refused mutation, one ISSUE read, and that part once more |

**The one write that sends a request more than it would without a cache.** When a board option was
deleted or renamed after the cache kept its id — `sources fields --apply` re-mints an option it
renames — GitHub refuses the mutation that sends it. The write then makes one read, which
refreshes the board's cached field and option ids so the same stale option cannot recur, and
sends the refused part once more: one request more than the write without a cache, and only on
the first write after the option moved. A part that already landed is not sent again, and
nothing is retried a second time.

**Its result comes from the mutations' own responses.** The write's JSON carries `result:
{"version", "parts": [{"part", "outcome", "fields"}]}`: each mutation sent, `landed`,
`rejected` with the backend's error, or `unknown` when the request left and no answer came back
— never sent again, because that could apply it twice — with each field it set and the value
its response shows, and the task's version after the write where the last response determines
it. No read is made to build or confirm it. A write that had to read first says so as
`fallback: {"read": true, "reason"}`, and `spent` counts that read. A failed write's failure
document carries the same members, so a caller tells a whole write, a partial one and an
ambiguous one apart.

**`--expect` is a cost and id contract, not a conditional write.** Neither GitHub's
`updateIssue` and `updateProjectV2ItemFieldValue` nor Linear's `issueUpdate` accepts a
precondition, so a person's edit to a field the write sets, landing between the caller's read
and the mutation, is overwritten. A write whose rule depends on a value a person may change is
the caller's to sequence: the store compares against the version the caller expects, and never
re-reads it behind the caller's back.

### 4.14 `metering`

Sent only to a plugin that answered `meters: true` at the handshake (§3.4), and never to one
that did not.

```json
{ "id": "12", "method": "metering", "params": {} }
```

```json
{
  "id": "12",
  "result": {
    "metering": {
      "requests": 7,
      "budgets": [
        { "budget": "graphql", "unit": "points", "measured": 0, "modelled": 6 },
        { "budget": "rest", "unit": "requests", "measured": 1, "modelled": 0 }
      ]
    }
  }
}
```

`metering` is a `Metering`: how many requests this plugin has sent to its own backend since
the handshake, and what those spent against each budget that backend meters it by. `budget`
and `unit` are the plugin's own open vocabulary. `measured` is what the backend reported, or a
count of requests against a budget metered in requests; `modelled` is what the plugin
estimated instead. Both are **running totals** and never a figure for one call: the engine
reads them before and after a command and reports the difference, so a plugin must not reset
them. `null` is read exactly as a plugin that does not meter.

The engine holds each pair of readings to that before it believes either: `requests` and
every `measured` and `modelled` may only grow, a budget named once stays named, and each
`budget` and `unit` pair is named at most once and with neither empty. A pair that breaks
any of it is reported as a plugin not metering for that command, never as a difference
clamped to zero.

A plugin that answers `metering` with an error is reported as not metering for that command.
The command does not fail over what it cost.

### 4.15 `task_comments`

Only a plugin that declared `comments` `"native"` in §4.2 is ever sent this.

```json
{ "id": "12", "method": "task_comments", "params": { "task": "ENG-1", "page": { "cursor": null, "limit": 50 } } }
{ "id": "12", "result": { "page": { "items": [], "next": null } } }
```

`task` is the `NativeId` of a task at **this** source and `page` is a `PageRequest` (§4.1).
`result.page` is a `Page<Comment>` holding that task's comments **oldest first**, or `null`
when this source holds no such task — "no such task" is `null` and not an error, exactly as
it is in §4.4, and a task that exists with no comments is a page with no items.

A `Comment` is an object with exactly these members:

| Member | Type | Meaning |
| --- | --- | --- |
| `id` | string | The comment's own id, as this source issued it. Never qualified (§3.2). |
| `author` | string or `null` | Who wrote it, in this source's own spelling of a person. |
| `created_at` | RFC 3339 string or `null` | When it was written. |
| `updated_at` | RFC 3339 string or `null` | When it last changed. |
| `body` | string | What it says, byte for byte — a trailing newline included. |
| `url` | string or `null` | Where a person can open it. |
| `origin` | object, optional | Present only on a comment a copy wrote: the comment it is a copy of, as `{"task": "<source>:<native task id>", "id": "<comment id there>", "author", "created_at", "updated_at", "url"}`. |

`null` says this source did not give that member. `id` and `body` are always present.

**A comment's `origin`** names the comment where it was written, however many copies it has
been through: a chained copy carries it on. A plugin that can keep it natively reports it as
the member (§4.16a). One that cannot — a hosted service that records every comment's author
and times itself — is handed the comment with its origin appended to the body as one hidden
line, `<!-- onetaskgraph.comment_origin `{…}` -->` after a blank line, the origin as compact
JSON whose `<`, `>` and `` ` `` are escaped; the engine takes that line back out of every
comment it reads and reports it as `origin`. A plugin keeping the marker should carry it over
when a person edits the comment's body, so the comment is still the copy it was.

**A plugin whose tasks have no comments refuses rather than answering an empty page**, for
the reason §4.11 gives about documents: it declared `comments` `"unsupported"`, so the engine
never sends it one, and a plugin should still implement the case defensively with a
`{"kind": "refused"}` naming itself.

### 4.16 `add_comment`, `edit_comment` and `delete_comment`

Only a plugin that declared `comments` `"native"` in §4.2 **and** answered `"supported"` to
§3.3 is ever sent any of these. Unlike §4.10, all three are verbs of the product: a person
adds, edits and removes a comment through them. A copy without `--comments` sends none of them;
a `--comments` copy sends `delete_comment` for a copy whose origin comment is gone, and writes
the rest through §4.16a — or, to a plugin that did not declare §3.12, through `add_comment` and
`edit_comment` with the origin's marker in the body.

```json
{ "id": "13", "method": "add_comment", "params": { "task": "ENG-1", "comment": { "body": "Seen again on main.\n", "author": null } } }
{ "id": "14", "method": "edit_comment", "params": { "task": "ENG-1", "comment": "C-7", "body": "Seen again on main, twice.\n" } }
{ "id": "15", "method": "delete_comment", "params": { "task": "ENG-1", "comment": "C-7" } }
```

For `add_comment`, `comment` is a `NewComment`: a `body` string, which is **never empty**,
and an `author` string or `null`. `result.comment` is the `Comment` this source now holds —
with the id it issued and the times it recorded — or `null` when this source holds no such
task.

For `edit_comment`, `comment` is the `NativeId` of the comment to change and `body` its new
body, never empty. Only the body and `updated_at` move; `id`, `author` and `created_at` are
the comment's own. `result.comment` is the edited `Comment`, or `null` when this source holds
no such task **or** that task has no comment under that id.

For `delete_comment`, `comment` is the `NativeId` of the comment to remove, and
`result.deleted` is the `NativeId` it removed, or `null` on exactly the terms `edit_comment`
answers `null`. A comment that is not there is **not** already gone the way an item of §4.10
is: a person named it, and the engine refuses by name what `null` reports.

A body is stored **byte for byte**, and nothing is silently dropped. A plugin that records the
author itself — a hosted service that knows which account is signed in — answers a
`NewComment` carrying an `author` with a `{"kind": "refused"}` saying why, rather than posting
under a name other than the one given. A plugin that cannot represent a body refuses it the
same way, naming why, rather than escaping it into something else. A comment id that names a
comment on a *different* task of this source is a comment this task does not have, and is
answered `null`.

### 4.16a `write_comment_copy`

Only a plugin that declared `comments` `"native"`, answered `"supported"` to §3.3 **and**
declared `comment_copies: true` (§3.12) is ever sent this.

```json
{ "id": "16", "method": "write_comment_copy", "params": { "task": "ENG-1", "existing": null, "copy": { "body": "Seen on main.\n", "origin": { "task": "board:I_1", "id": "IC_9", "author": "someone", "created_at": "2026-09-01T00:00:05Z", "updated_at": "2026-09-01T00:00:05Z", "url": null } } } }
{ "id": "16", "result": { "comment": { } } }
```

One comment a `--comments` copy carries onto `task`: a new one when `existing` is `null`, and
otherwise the comment `existing` — one a copy wrote before — made to say `copy.body` with
`copy.origin`. `result.comment` is the `Comment` this source now holds, reporting `origin` as
`copy.origin` however it keeps it, or `null` when this source holds no such task or no such
comment. A plugin that keeps a comment's author and times as it is told may take the origin's
for the copy; a folder of Markdown does, and its copy reads back with the origin's own id.
`copy.origin.task` is always a qualified task id, `<source>:<native task id>`: the shipped host
refuses one that is not with `{"kind": "malformed"}` before it writes anything, and an engine
reading back a comment whose `origin` names no qualified task reads it as having none.

### 4.17 `set_task_status` and `set_delivered_by`

Only a plugin that answered `"supported"` to §3.3 **and** `task_updates: true` to §3.6 is ever
sent either of these. Each writes one field of one task and nothing else.

```json
{ "id": "16", "method": "set_task_status", "params": { "id": "ENG-1", "category": "queued" } }
{ "id": "16", "result": { "status": { "category": "queued", "name": "Queued" } } }
{ "id": "17", "method": "set_delivered_by", "params": { "id": "ENG-1", "delivered_by": ["plan:P-1"] } }
{ "id": "17", "result": { "delivered_by": ["plan:P-1"] } }
```

For `set_task_status`, `id` is the `NativeId` of a task at **this** source and `category` a
`StatusCategory`. The plugin sets that task's status to where its own mapping sends the
category — exactly where a `write_task` of a task in that category would put it — and
changes **nothing else**: title, content, labels, metadata, dependencies, `delivers`,
`delivered_by`, project and comments stay as they are. `result.status` is the `Status` the
plugin now reads for that task, category and name, or `null` when it holds no such task. A
category this plugin has disabled is refused with `{"kind": "refused"}`, in the words its
`write_task` refuses that status with.

For `set_delivered_by`, `delivered_by` is the whole list the task holds afterwards — it
replaces what was there rather than merging with it — and every entry is a qualified
`<source>:<native>` id. Nothing else about the task changes. `result.delivered_by` is the list
the task now holds, or `null` when this plugin holds no such task: the trait method carries
`()` inside its `Option`, so all a plugin says is that it wrote the list, or that there was no
task to write it on.

A plugin that cannot write one of these on its own refuses with `{"kind": "refused"}`, naming
the field.

`TaskSource::set_task_status_reading` — a status write answered with the whole task as the
source now reads it — is not a message of this protocol. For a hosted source the engine answers
it the way the trait's default does, with one `get_task` and then, for a task found, one
`set_task_status`, so a plugin written against this document implements nothing more for the
engine to keep delivered tasks in step after a status write.

#### A task's `delivers` and `delivered_by`

A `Task` carries two optional lists of strings, each left out — never `null` — when empty:

- `delivers` — the tasks this one delivers, which finishing it finishes. An entry holding a
  colon is a qualified `<source>:<native>` id naming a task of any source; an entry without
  one names a task of this plugin's own source, colons being how the two are told apart.
- `delivered_by` — every task that delivers this one, by qualified id. It is the **engine's**
  to keep: a plugin holds it, reports it, and writes it when `write_task` or
  `set_delivered_by` hands it one, and nothing a copy reads from a source ever supplies it.

A plugin refuses — `{"kind": "malformed"}` on a read, `{"kind": "refused"}` on a write — an
entry that is not a task id, an entry naming the task itself (bare, or qualified with the
plugin's own `source_name` from §3.2), and an entry naming one task twice, naming the task
and the entry. A plugin with no field of its own for these lists keeps them under the reserved
metadata keys `onetaskgraph.delivers` and `onetaskgraph.delivered_by`, and never returns
either among a task's `metadata`.

#### What the engine does with them

These rules are the engine's. A plugin implements none of them, and is reached by them only
through the two methods above.

Whenever the engine writes a task whose `delivers` is not empty, or was not before the write —
a copy that lands it, or `task status set` on it, including a write that leaves the task's own
status as it was — it re-evaluates every task that list names and every task it named before
and does not any more:

1. **The back-reference.** A delivered task's `delivered_by` gains the deliverer's qualified id
   while the deliverer names it, and loses it once the deliverer has dropped it.
2. **What each deliverer counts as.** Over every deliverer that `delivered_by` names — the one
   just written included, one that just dropped the task excluded — `queued` and `in-progress`
   are a claim, `done` is finished, `todo`, `cancelled` and `unknown` are a release, and
   `draft` and `backlog` count as nothing. A deliverer its source reads as not found counts as
   nothing and is removed from `delivered_by` on the same write.
3. **The result**, by the first branch that matches: `done` when every deliverer is `done` or
   `cancelled` and at least one is `done`; otherwise `in-progress` when any is `in-progress`;
   otherwise `queued` when any is `queued`; otherwise `todo` when any releases or is `done`;
   otherwise `todo` when no deliverer remains; otherwise — every remaining one `draft` or
   `backlog` — nothing.
4. **The write.** A delivered task is written only when it reads `todo`, `queued` or
   `in-progress` and the result differs from what it holds, and only through
   `set_task_status`. One at `draft`, `backlog`, `unknown`, `done` or `cancelled` is left alone.
5. **A failure.** A delivered task that cannot be read or written, or a deliverer that cannot be
   read for any reason other than not found — a transport failure, a source the configuration
   does not name — is reported failed for that task, which is left as it was. The deliverer's
   own write still stands.

A task the rule may not write — `draft`, `backlog`, `unknown`, `done` or `cancelled` — is
reported without its other deliverers being read, so nothing is pruned from it and no unreadable
deliverer fails it on that write; its back-reference is still added or removed.

Each verb that writes a deliverer — `task status set`, and a copy that is not a dry run —
reports one entry per task it re-evaluated in a `delivered` list: `ticket` and `deliverer`, the
two qualified ids; `outcome`, one of `written`, `unchanged`, `left` or `failed`; `from`, the
category the task read, present whenever it could be read; `to`, the category written, present
only when `written`; `failure`, present only when `failed`; and `pruned`, the deliverers removed
for being not found, present only when there were any. `failure` is the failure object itself —
the `class`, `kind`, `source`, `message` and `retry_after_seconds` a failure document carries
under its own `failure` member — not that whole document nested again. A dry run writes
nothing, so it re-evaluates no task and its `delivered` list is empty.

### 4.18 `set_task_metadata`, `set_project_metadata` and `set_document_metadata`

Only a plugin that answered `"supported"` to §3.3 **and** `metadata_updates: true` to §3.7 is
ever sent any of these, and `set_document_metadata` only to one that also declared `documents`
`"native"` (§4.2). Each sets one key of one record's metadata and changes nothing else.

```json
{ "id": "18", "method": "set_task_metadata", "params": { "id": "ENG-1", "key": "myapp.review", "value": { "approved": true } } }
{ "id": "18", "result": { "task": { "id": "ENG-1", "title": "…", "metadata": { "myapp.review": { "approved": true } }, "…": "…" } } }
{ "id": "19", "method": "set_project_metadata", "params": { "id": "P-1", "key": "myapp.review", "value": 3 } }
{ "id": "19", "result": { "project": null } }
{ "id": "20", "method": "set_document_metadata", "params": { "id": "D-1", "key": "myapp.review", "value": null } }
{ "id": "20", "result": { "document": { "id": "D-1", "title": "…", "metadata": { "myapp.review": null }, "…": "…" } } }
```

`id` is the `NativeId` of a task, a project or a document at **this** source. `key` is a
`MetadataKey`: a string of two or more non-empty dot-separated segments whose first segment is
not `onetaskgraph`, `value` is any JSON value, `null` included, and the engine sends no other
key in that namespace but one — a plugin may refuse any other with `{"kind": "malformed"}`
if it is ever handed one.
<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Held by behaviour rather
     than by words: the served side refuses every other shape of this one key's value before
     the hosted source is asked, and
     `a_served_plugin_takes_the_copy_link_key_only_with_a_value_that_is_links` in
     `crates/onetaskgraph-core/tests/subprocess.rs` drives each refused shape and the accepted
     one across the wire; the value's own shape is stated once, in `docs/metadata.md`. -->
The one is `onetaskgraph.copies`, which a copy writes on an item it read from this plugin to
record where that item landed; its value is the object `docs/metadata.md` describes, and the
plugin holds it wherever it holds the item's metadata. A plugin that cannot hold it refuses
with `{"kind": "refused"}`, and the copy goes on without the link and says so rather than
failing.
<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->

The plugin holds `value` under `key` — adding the key when the record does not hold it and
replacing what it holds when it does — and changes **nothing else**: every other metadata key,
the title, content, labels, status, dependencies, `delivers`, `delivered_by`, project and
comments stay as they are. A `value` the record already holds under `key` owes no write at all.
`result.task`, `result.project` or `result.document` is the record as the plugin reads it
**after** the write — a `Task`, a `Project` or a `Document`, in the shape `get_task`,
`get_project` or `get_document` answers with (§4.4, §4.11) — or `null` when this plugin holds
no such record. The engine reports the value
that record holds under `key`, not the one it sent, so a plugin that stores a value in another
shape says so by reading it back.

A plugin that cannot write one key on its own refuses with `{"kind": "refused"}` and the
message §3.7 spells; one that cannot make this particular write without changing something
else — a record whose stored form it cannot edit that narrowly — refuses with
`{"kind": "refused"}`, naming the record and why. Metadata is not status: the engine keeps no
delivered task in step after any of these.

### 4.19 `set_task_priority`

Only a plugin that answered `"supported"` to §3.3 **and** declared `priority` `"native"` (§4.2)
is ever sent this. It sets one task's priority and changes nothing else.

```json
{ "id": "21", "method": "set_task_priority", "params": { "id": "ENG-1", "priority": "urgent" } }
{ "id": "21", "result": { "priority": "urgent" } }
```

`id` is the `NativeId` of a task at **this** source, and `priority` is a `Priority` (§4.13b);
`"none"` clears it. `result.priority` is the priority as the plugin reads the task **after**
the write, or `null` when this plugin holds no such task. Title, content, status, labels,
metadata, dependencies, `delivers`, `delivered_by`, project and comments stay as they are.

A plugin that has nowhere to put this priority — a board with no option for it — refuses with
`{"kind": "refused"}`, naming what it lacks. Priority is not status: the engine keeps no
delivered task in step after it.

### 4.20 `set_task_content`

Only a plugin that answered `"supported"` to §3.3 **and** `content_updates: true` to §3.9 is
ever sent this. It replaces one task's content and changes nothing else.

```json
{ "id": "22", "method": "set_task_content", "params": { "id": "ENG-1", "content": "Why this matters.\n" } }
{ "id": "22", "result": { "id": "ENG-1" } }
```

`id` is the `NativeId` of a task at **this** source, and `content` is a string: what the task's
`content` holds afterwards, as `get_task` reports it (§4.4). `result.id` is the task's id, or
`null` when this plugin holds no such task: the trait method carries `()` inside its `Option`,
so all a plugin says is that it wrote the content, or that there was no task to write it on.
Where the plugin keeps something else in the same
backend field as the content — a metadata block in an issue body — that is kept as it was, and
so is every other member: title, status, priority, labels, metadata, dependencies,
`delivers`, `delivered_by`, project and comments. There is no compare-and-set: the content is
replaced whatever it held a moment before.

### 4.21 `update_task`

Only a plugin that answered `"supported"` to §3.3 **and** `targeted_updates: true` to §3.10 is
ever sent this. It writes every field the update names, and nothing else, in as few writes as
the plugin's backend allows.

```json
{ "id": "23", "method": "update_task", "params": { "id": "ENG-1", "update": { "status": { "category": "done", "name": "Done" }, "metadata_set": { "team.review": true }, "metadata_remove": ["team.claim"] } } }
{ "id": "23", "result": { "outcome": { "task": { "id": "ENG-1", "title": "…" }, "written": ["metadata", "status"], "delivers_before": [] } } }
```

`id` is the `NativeId` of a task at **this** source, and `update` is a `TaskUpdate`: a member
left out names nothing, and every member that is present is written. `metadata_set` adds or
replaces the keys it holds and `metadata_remove` removes the keys it lists, a key the task does
not hold being no write; `delivers` replaces the whole list; `depends_on` replaces the task's
forward edges, each far end in this source named by its native id. A field already holding the
requested value is sent no write, and an update in which nothing differs sends none at all.
Everything the update leaves out — labels, repositories, project, `delivered_by` and comments
always among it — is left exactly as it was.

`result.outcome` is a `TaskUpdateOutcome`, or `null` when this plugin holds no such task:
`task` is the task as the plugin reads it once the update landed (§4.4), `written` is the
fields it actually wrote, in the `UpdatedField` vocabulary and never one the update did not
name, and `delivers_before` is the task's `delivers` as it stood before the update. The engine
reports all three and reads nothing else of the plugin for the call, so they are the plugin's
word: a `task` whose `id` is not the one asked about is a protocol violation.

The engine sends no update naming a status category this plugin's handshake did not list
(§3.5), a priority other than `"none"` to a plugin declaring none (§4.13b), or a `delivers` to
a plugin without `task_updates` (§3.6). An update naming one key in both `metadata_set` and
`metadata_remove` is refused before it is sent, with `{"kind": "refused"}`, and a plugin
handed one refuses it the same way.

### 4.22 `end_command`

Sent only to a plugin that answered `ends_commands: true` at the handshake (§3.11), and never
to one that did not.

```json
{ "id": "24", "method": "end_command", "params": {} }
{ "id": "24", "result": {} }
```

A **command** is one unit of work. For the binary it is one invocation, which is one
connection, so the binary never sends this: the connection ends with the command. A caller
holding one engine — and so one connection — for many units of work, such as a worker that
writes one settlement back after another over hours, sends it between them through
`Engine::end_command`, which asks every source it holds and names none of them.

Within a command a plugin may hold what it read and reuse it. **Once it has answered this, it
answers no read and bases no write on item content, statuses, board contents or search results
it held before the call.** It may keep only identifiers and vocabulary that stay valid in
normal use — a repository's node id, a team's id, its workflow states, a workspace's project
statuses — and only where a lookup that misses one reads afresh rather than refusing. Running
totals it answers `metering` (§4.14) from are not work and are kept; §4.14 forbids resetting
them. The reason is a person: between two commands they may edit an item's body or move it to
another status, and a write based on what was held before would overwrite the edit or move the
item from where it no longer is.

`result` is an empty object — the trait method carries `()`, so there is nothing for it to say
beyond having done it — and members in it are ignored (§2.1). An error envelope means the
plugin could not be sure it dropped what it held, and the engine reports it against the source
as `Engine::end_command`'s failure; the caller should not go on to the next command on that
source.

### 4.23 `status_contradictions`

Sent only to a plugin that declared `reconcile_status` `"native"` (§4.2). It changes nothing:
it answers every task this source holds whose **card contradicts its issue's closure**, each
with the status that brings it in line, and the engine writes each one through
`set_task_status` (§4.17).

```json
{ "id": "25", "method": "status_contradictions", "params": {} }
{ "id": "25", "result": { "items": [ { "task": { "id": "I_7", "title": "…", "status": { "category": "done", "name": "Done" }, "…": "…" }, "option": "Todo", "closed_as": "done", "to": "Done" } ] } }
```

The trait method answers `Vec<StatusContradiction>`, carried as `result.items`, one
`StatusContradiction` per task. A task is answered when its issue is closed with a reason that
reads `done` (completed, or no reason) or `cancelled` (not planned, duplicate) and its card does
not sit at the option this source's mapping gives that category; `option` is the option it sits
at, or `null`, `closed_as` the category its closure reads as — `done` or `cancelled`, the only
two a card is brought in line with — and `to` the name of the option that category is written
at. An open task, a closure that reads `unknown` and a card already in line are never answered —
nor is any item this source reads as a project rather than a task, whatever its closure, because
this method reaches task items only — and no task is answered twice: the engine refuses an
answer naming one task twice as malformed. `task` is the task as `get_task` reads it.

The `set_task_status` the engine sends for each moves the card alone: an issue already closed
in a way that reads as the category is not closed again, so its state and its reason — a
duplicate's among them — stay as they are.

### 4.24 `issue_holdings`

Sent only to a plugin that declared `complete_by_issue` `"native"` (§4.2), and never with an
empty list. It changes nothing: for each issue reference, in the order given and once each, it
answers the task this source holds for that issue and what moving it to `done` comes to. It
reaches task items only: an issue this source holds as a project is answered `absent`, exactly
as one it holds nothing for.

```json
{ "id": "26", "method": "issue_holdings", "params": { "issues": ["acme/work#12", "acme/work#40"] } }
{ "id": "26", "result": { "holdings": [ { "issue": "acme/work#12", "completion": "due", "to": "Done", "task": { "id": "I_12", "option": "Todo", "classification": "public", "delivers": ["tickets:T-1"] } }, { "issue": "acme/work#40", "completion": "absent" } ] } }
```

An issue reference is `owner/name#N` — exactly the spelling `onevcs` reports in a landing
event's `closes` — the `IssueRef` root of `onetaskgraph schema`, its owner and name compared
ignoring case. The trait method answers `Vec<IssueHolding>`, carried as `result.holdings`, one
`IssueHolding` per reference; the engine refuses, as malformed, an answer that does not name
each reference it was asked about exactly once and in the order given. Each holding's `completion` is one
of:

- `absent` — this source holds no item for the issue, or cannot see the issue at all; it alone
  carries no `task`;
- `skipped`, with `task` and `reason` — `closed as not planned` or `closed as duplicate`: the
  closure already says the work will not be done here, which a landing does not overrule;
- `unchanged`, with `task` — the issue is closed as completed and its card already sits at the
  `done` option;
- `due`, with `task` and `to`, the name of the `done` option — every other task: the engine
  writes it `done` through `set_task_status` (§4.17), which selects the `done` option and closes
  the issue as completed if it is still open, never reopening one.

`task` is the `HeldTask` root: the task's own `id`, the `option` its card sits at, and what the
write and the rules around it need of it — `classification`, `repositories` and `delivers` — so
the engine keeps every task it delivers in step exactly as after any status write (§4.17),
without reading the task again. A plugin's read
is proportional to the references it is given and never to the size of what it holds.

## 5. The error envelope

`error` carries a `SourceError` whole. It is internally tagged on `kind`, and every
variant's data is owned — the type was shaped this way for exactly this boundary.

| `kind` | Other members | Meaning |
| --- | --- | --- |
| `config` | `message` (string) | The configuration for this source is invalid. |
| `auth` | `message` (string) | Authentication failed or a credential is absent. |
| `refused` | `message` (string) | The source understood the request and refused it. |
| `rate-limited` | `retry_after_seconds` (integer or `null`), `message` (string, optional) | The source rate-limited the request. |
| `unavailable` | `message` (string) | The source could not be reached. |
| `malformed` | `message` (string) | The source returned data this interface cannot represent. |

```json
{ "id": "3", "error": { "kind": "rate-limited", "retry_after_seconds": 30 } }
{ "id": "4", "error": { "kind": "rate-limited", "retry_after_seconds": null,
                        "message": "the secondary rate limit refused this while creating an issue; `gh api rate_limit` does not report that limiter" } }
```

A `message` is for a person to read. It must not contain a credential, and it must
not contain a qualified id (§3.2).

**On `rate-limited`, `message` is optional and is omitted when the plugin has nothing to
add.** Absent means exactly what every plugin said before the member existed — that the
kind is the whole of what is known — so a reader that has never heard of it sees the shape
it was written for. It is there because a rate limit is the one refusal whose reason a
person cannot infer from the kind: a hosted service usually has more than one limiter, only
some of them are reported by the endpoint an operator would go and check, and reading one
as the other sends them to check a budget that looks fine and then to retry the burst that
was refused. A plugin that knows which limiter refused it, and what it was doing at the
time, says so here. `retry_after_seconds` is unchanged and stays the member a caller acts
on.

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

The optional `message` of the `rate-limited` error envelope (§5) was added **without** a
bump, and it is the plainest case of §2.1 there is: an added optional member with a
documented default, which a reader that has never heard of it skips. A plugin written
before it omits it and is read exactly as it was; an engine written before it ignores it
and acts on `retry_after_seconds` as it always did. Bumping for it would refuse every
existing plugin at the handshake over a member none of them has to understand.

The documents of §4.11 and §4.12, and the `location` member of §4.13, were added **without**
a bump, and the difference from §4.10 above is the whole reason. The engine sends a document
method only to a plugin that declared `documents` `"native"` in its handshake, and a plugin
written before there were documents cannot have declared that: it omits the member, §4.2
reads the omission as `"unsupported"`, and it is never sent one. A delete had no such gate —
`writes` says whether a plugin can be written, not whether it can be un-written — so a
version 1 plugin could be sent one halfway through undoing a copy. Adding a method behind a
declaration its peer cannot accidentally make is the "method a peer may decline" case below;
adding one a peer has already implicitly opted into is not.

A task's `key` (§4.13a) was added **without** a bump, and it is the `location` case again
rather than a new one: an added optional member with a documented default, on a shape both
peers already exchange. A plugin written before it omits the member and is read as the
source with no short handle it is; an engine written before it ignores one a plugin sends,
exactly as it ignored `location`. Nothing is gated on it because nothing has to be — no
method takes it, no method returns only it, and no request is decided by whether a peer
understands it, so there is no shape a peer could be handed that it was not written for.

A task's `priority` (§4.13b), the `priority` and `filter_by_priority` members of §4.2 and the
`priorities` member of §4.5 were added **without** a bump, beside `location` and `key` and for
their reason, with one gate more. Unlike a `key`, a priority is *written*, so a plugin written
before it could be handed one it would drop in silence under §2.1 — and it never is: such a
plugin omits `priority` from its capabilities, §4.2 reads that as `"unsupported"`, and the
engine refuses, by the source's name and the field's, every write that would carry it a
priority other than `"none"`. A `"none"` it already reports for every task, so a copy carrying
one writes exactly as before. It likewise omits `filter_by_priority`, so it is never sent a
`priorities` filter, and the engine narrows the wider set it returns.

The `filter_by_comment_activity` member of §4.2 and the `commented_since` member of §4.5 were
added **without** a bump, as `filter_by_priority` and `priorities` were: a plugin written
before them omits the first, is read as not applying the filter, and is never sent the second
— the engine narrows the wider set it returns over the plugin's own comments.

The `filter_by_metadata` and `filter_by_origin` members of §4.2 and the `metadata` and
`origin` members of §4.5 were added **without** a bump on the same terms: a plugin written
before them omits both capabilities, is read as applying neither, and is never sent either
member — the engine narrows the wider set it returns over each task's own `metadata`.

`check_task_write` (§4.9b) and the `write_checks` member of §3.14 were added **without** a
bump: the method reaches only a plugin that declared the member, which a plugin written before
them cannot have done.

`status_contradictions` (§4.23), `issue_holdings` (§4.24) and the `reconcile_status` and
`complete_by_issue` members of §4.2 were added **without** a bump on the same terms: each method
reaches only a plugin that declared its member `"native"`, which a plugin written before them
cannot have done, so such a plugin is reported as holding nothing to reconcile and listed as
unsupported by a completion — and is sent nothing either time.

`set_task_priority` (§4.19), `set_task_content` (§4.20) and the `content_updates` member of
§3.9 were added **without** a bump, as the methods of §4.18 were: the first reaches only a
plugin that declared `priority` `"native"`, and the second only one that answered
`content_updates: true`, neither of which a plugin written before them can have done.

`metering` (§4.14) and the `meters` member of §3.4 were added **without** a bump, for the
same reason the documents were: the engine sends `metering` only to a plugin that answered
`meters: true`, and a plugin written before there was metering omits the member, is read as
not metering, and is never sent it.

The comments of §4.15 and §4.16 were added **without** a bump, for exactly the reason the
documents were: the engine sends a comment method only to a plugin that declared `comments`
`"native"` in its handshake, and a plugin written before there were comments cannot have
declared that. The three that write are gated twice, on that declaration and on §3.3.

The `"queued"` status category, the `statuses` and `task_updates` members of §3.5 and §3.6,
the two methods of §4.17 and a task's `delivers` and `delivered_by` were added **without** a
bump, and each of them is gated so that a peer written against the earlier protocol is never
handed a shape it was not written for without a refusal by name. A new category is a value a
peer has to parse, so it is not safe under §2.1 on its own: an engine hands a plugin a category
only when that plugin's handshake listed it, a plugin tells an engine one only when that
engine's `initialize` listed it, and every other case is the one §3.5 spells out — answered
without it, refused by name, or reported as `"unknown"` under its own name. The two methods and
the two lists reach only a plugin that answered `task_updates: true`, which a plugin written
before them cannot have done, so such a plugin is refused by name before anything is sent
rather than asked for a method it has never heard of or handed a list it would drop.

The `metadata_updates` member of §3.7 and the three methods of §4.18 were added **without** a
bump, for the reason the methods of §4.17 were: they reach only a plugin that answered
`metadata_updates: true`, which a plugin written before them cannot have done, so such a plugin
is refused by name before anything is sent.

`onetaskgraph.copies` as a §4.18 key was added **without** a bump: it reaches only a plugin
that answered `metadata_updates: true`, and one written when §4.18 let a plugin refuse every
key in that namespace as malformed may go on doing so — the engine reads either refusal of
that one key as the plugin not holding the link, and the copy completes without it.

The `assets` member of §4.2 and the `assets` and `recorded_assets` members of §4.9a were added
**without** a bump, for the reason the documents were: the engine sends a write carrying them
only to a plugin that declared `assets` `"native"`, which a plugin written before assets cannot
have done. Such a plugin is sent every write exactly as before — a write of a record whose
content references no asset never carries either member — and a create or a copy of a record
whose content does reference one is refused by name before that plugin is sent anything.

`end_command` (§4.22) and the `ends_commands` member of §3.11 were added **without** a bump,
for the reason `metering` was: the engine sends the method only to a plugin that answered
`ends_commands: true`, and a plugin written before it omits the member and is never sent it.
Reading that silence as holding nothing is sound for every plugin of this build that existed
then — a hosted plugin of this build is served by its own host, which answers `true` and
forwards the call — and §3.11 makes declaring it the obligation of any plugin that holds more.

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
{"id":"1","result":{"items":[{"id":"tasks/migrate.md","key":null,"title":"Migrate the store","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"project":null,"url":null,"location":null,"created_at":null,"updated_at":null},{"id":"tasks/schema.md","key":null,"title":"Settle the schema","content":null,"status":{"category":"in-progress","name":"Doing"},"labels":[],"project":null,"url":null,"location":null,"created_at":null,"updated_at":null}],"next":"b2Zmc2V0PTI"}}
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

### GitHub Projects narrowing-search paging

<!-- github-search-paging:start -->
Board-scoped text, metadata, project-name and comment-activity searches send every
page at `first = 20` (SEARCH_PAGE_SIZE), the SEARCH_ISSUES document's one-point
ceiling. A later page is sent only when `hasNextPage` is true and the caller still
needs rows. A page is never resized to the rows still needed: GitHub orders one
search differently at different page sizes, so one fixed size makes a paged walk
send exactly the requests one whole read sends, and the answer's order is the order
those pages arrive in. A page below twenty would cost the same one point, and GitHub
prices this document by rows, so twenty-row pages cost per row what 100-row pages do.
Project-name lookup continues until an exact match or exhaustion. A task limit bounds
returned and fetched pages: a limit is sliced from the pages it needs, and local
confirmation can require more candidates than matching rows. Walking all pages
returns the whole answer. The opaque version-4 source cursor carries GitHub's page
cursor and how far into that page the last answer stopped, and resumes in the same
process or a new one, without duplicates or gaps. It carries no rows: one process
sends each page's search once, and a new process re-reads only the page it resumes
in, then sends a further page once, never as a re-read, only when its limit still
needs rows. Every request either walk sends is the one a whole read sends for that page. Own writes replace stale index
copies and complete missing rows at exhaustion. Cache entries are whole GitHub pages,
so a small answer cannot truncate a wider question. Origin pages remain three; whole-board sizing is unchanged.
Read-after-write is a per-process guarantee. A cursor resumed in a new process is
not required to include the original process's writes still omitted by the index.
<!-- github-search-paging:end -->

### GitHub Projects item records and writes

`task show ID` includes comments by default. `task show ID --no-comments` reads
only the item record and omits the optional `comments` member. The Python SDK
exposes the same choice as `task_show(id=ID, no_comments=True)`; TypeScript uses
`taskShow(ID, { noComments: true })`. Rust callers already have `Engine::task`
for the record and `Engine::task_detail` for the record with comments. The
response shape and schema version are unchanged: comments were already optional.

`task show-many ID... [--no-comments]` — `task_show_many(ids, no_comments=...)` in Python,
`taskShowMany(ids, { noComments })` in TypeScript, `Engine::task_details` for a Rust caller —
answers `{"details": [...]}`: one `TaskDetail` per id, in request order, each exactly what
`task show ID --json` prints. An id that cannot be read carries its failure in its own
`errors` and does not refuse the others; the command exits 4 exactly when some detail does.
The engine asks each source for its ids together through `TaskSource::get_task_details`,
which a plugin may override and which defaults to `get_task` and `task_comments` item by item.
It is not a message of this protocol: a subprocess plugin answers it through those two.

A GitHub Projects source reuses records resolved in its own instance for writes
and reuses their identity for comments. Explicit item reads still fetch fresh
records. A mutation invalidates its target's binding before sending, and a
successful write replaces it; a retry after a partial failure therefore resolves
the target again. Comment mutations preserve the binding because they change no
item fields. These records stay inside the plugin and are never persisted.
`end_command` (§4.22) drops every one of them, with the board, search and field
reads beside them, so the first write of the next command resolves its target
afresh; only repository node ids are kept.

A copy combines changed board fields in one GraphQL mutation request using
aliases, including a priority clear. Unchanged origin and status fields need no
mutation. A standalone priority write selects the stored priority in its mutation
response, so its answer remains a read-back, including when the host did not keep
the requested value, without resolving the issue a second time. The plugin's cost
table records the requests and declared prices proved by the loopback journeys.

`task copy --create` — `task_copy(..., create=True)` in Python, `taskCopy(ids, to, { create:
true })` in TypeScript, `CopyRequest::create` for a Rust caller — creates each task at the
destination without the correspondence lookup, so a GitHub Projects destination is sent no
`ORIGIN_LOOKUP`. The caller asserts the destination holds no carrier of the task, and that is
sound for a caller that has just run the origin query itself: the query reads GitHub's issue
search and the board's field filter, both of which lag a fresh write, and they lag it by the
same amount whoever repeats them — a copy asking again could find nothing the caller's own
question did not. It is a rule of the copy engine rather than of any plugin. It is refused
beside `--match-by` and `--recreate`, which are ways of looking, and for a task whose own
`onetaskgraph.copies` link or origin already names an item at the destination, naming that
item. The copy report is the one any copy prints, and a created item answers this process's
later reads exactly as any created item does.

### GitHub Projects request costs

Every request below is one GraphQL document priced at one point by the node-count model, so a
row's requests are its points. `crates/onetaskgraph-github-projects-e2e/tests/e2e/copy_cost.rs` pins each row
against the requests the loopback board served, and holds this table and the one in the
plugin's crate documentation to the same figures.

| Verb | Requests / points | Documents |
| --- | --- | --- |
| new copy | 5 | ORIGIN_LOOKUP, CREATION_CONTEXT (the board's fields and the repository's id together), CREATE_ISSUE, ADD_TO_BOARD, UPDATE_FIELDS |
| copy --create | 4 | CREATION_CONTEXT, CREATE_ISSUE, ADD_TO_BOARD, UPDATE_FIELDS: the new copy without its ORIGIN_LOOKUP |
| bound copy | 3 | ISSUE (with the board's fields and the issue's `blockedBy`), UPDATE_FIELDS, then UPDATE_ISSUE last |
| bound copy, filed under a project | 4 | the bound copy's three, and one ISSUE of the destination project its link names, read once per command |
| bound copy, newly naming n dependencies | + ceil(n / DETAIL_BATCH) + n | ISSUE_DETAILS for the far ends that do not already block the item, DETAIL_BATCH (24) to a request (one alone is ISSUE), then one ADD_BLOCKED_BY each; a far end already blocking it costs nothing |
| comment | 2 | ISSUE, ADD_COMMENT: the target is read first, because GitHub accepts a comment on any issue or pull request |
| detail | 1 | ISSUE_DETAIL: the item and its first page of comments, for `task show` and `task comment list`; `--no-comments` is ISSUE alone |
| batched detail | ceil(n / DETAIL_BATCH) | ISSUE_DETAILS: `task show-many` of `n` items, DETAIL_BATCH (24) at a time |
| copy --comments, n changed tasks | ceil(n / COPY_DETAIL_BATCH) | ISSUE_COPY_DETAILS: each task with its first page of comments and its `blockedBy`, COPY_DETAIL_BATCH (21) at a time; a task whose comments run past that page adds one ISSUE_DETAIL per further page, and a task skipped from its listing costs nothing beyond it |
| update | 3 | `task update` naming any of title, body, metadata, status and priority — all five included: ISSUE, UPDATE_FIELDS (the status option and the priority together), UPDATE_ISSUE (title, body and state) last |

`DETAIL_BATCH` is 24, the largest batch of `ISSUE_DETAILS` the node-count model prices at one
point: each aliased `node(id:)` item is six of GitHub's aggregate, so 24 come to 144, which
rounds to one point, and 25 to 150, which rounds to two. The document is fixed-size aliased
`node(id:)` fields rather than `nodes(ids:)`, which the model cannot see under.

A bound re-copy is 3 requests for a task under no project that names no dependency it does not
already carry. A project parent adds one: the engine reads the destination project its link
names by its own id, once per command, to confirm the link still holds. The dependencies a
re-copy newly names are read together — the item's own read already answered every far end
that blocks it — `DETAIL_BATCH` to one `ISSUE_DETAILS` request, rather than one request each;
recording each new edge is then one `addBlockedBy`.

An existing item is written body last. A bound re-copy and a `task update` send its board
fields first, then its parent and its `blockedBy`, and its title, body and state in one
`updateIssue` last. GitHub runs no two requests as one, and runs a document's mutation fields
in order without undoing an earlier field when a later one fails, so that order is what makes
a write refused part-way leave the item's body and metadata exactly as they stood; an origin
a copy re-points, the one metadata key written before the body, is put back when a later write
is refused, and when putting it back is refused too, the write's own refusal — its kind
unchanged — names that key, what it now holds and what it held, and the step that restores it.
When the refused write was the board-field write that carries the origin itself, GitHub does
not say which of its fields ran, so the refusal names both values the key may hold.

Two facts about GitHub the write rows rest on, read off GitHub's published schema artifact
(<https://docs.github.com/public/fpt/schema.docs.graphql>, 2026-10-01) and pinned in
`crates/onetaskgraph-github-projects/tests/fixtures/schema.graphql`, the first then put to
GitHub itself:

- `createIssue` accepts the board at creation — `CreateIssueInput.projectV2Ids: [ID!]`, "An
  array of Node IDs for Projects V2 associated with this issue" — but does not answer with the
  board item. The credentialed journey run against a real board on 2026-10-01 with a create
  sending the board there saw every one of its four creates answer with no item in
  `Issue.projectItems`, and the `addProjectV2ItemById` that then had to follow was refused
  "Content already exists in this project": GitHub filed the issue after answering, and
  refuses a second filing rather than answering with the item it holds. So a create sends no
  `projectV2Ids` and files the issue with `addProjectV2ItemById`, whose answer names the item;
  what a new copy saves is the read before it, the board's fields and the repository's id in
  one `CREATION_CONTEXT` request.
- A comment cannot skip its target's read: `AddCommentInput.subjectId` admits
  `Issue` and `PullRequest`, so GitHub refuses a draft but accepts a project's issue, a
  document's issue, an issue on no board and a pull request alike. There is no refusal to map
  into "that is not a task of this board", so the target is read first.

No field or repository id is kept past one command: the store holds no work data outside a
plugin, and an option id kept between runs would write the wrong `Status` once `sources
fields --apply` re-mints the options.
