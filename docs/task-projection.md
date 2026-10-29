# Human task projection

`onetaskgraph task project-human --input FILE --json` projects one human action item into the
tracker its routing selects, idempotently by its `item_id`, and answers where it is. `FILE` is
`-` for standard input.

## The contract

The Rust task-projection domain owns the [request and binding declarations](../crates/onetaskgraph-core/src/task_projection/mod.rs):
`HumanTaskProjection` (the v1 request) and `HumanTaskProjected` (the answer — `task`, `url`,
`project`, `source`). Their schema roots come from those declarations through `schema_bundle`,
and the bundle's versioned golden detects drift; both SDKs generate their models from it. Use
the exported schema for accepted fields and values. A request with any other field, version or
state — a question included — is refused before any source is started (`projection-input`).

## Configuration

A source that receives human tasks declares its authority:

```yaml
sources:
  human-public:
    plugin: github-projects
    config: { ... }
    visibility: public
    routes:
      - { classification: private, purpose: human-task, to: human-private }
      - { repositories: [github.com/example-org/*], purpose: human-task, to: example-org }
    task_projection:
      host: workstation-1          # the one host authoritative over these intents
      directory: /abs/path/intents # where the durable intents live
      project: PROJECT-1           # the project a planless item is filed under
```

The request's `source` names where routing starts; only route entries with `purpose:
human-task` route a human task, and an entry without one is the legacy route every other
writer takes. While the public boundary is active a task is private when any of its
repositories is not answered public, and a private task with no private route is refused,
never written publicly; a public destination's write is screened by the caller's check. A request's
`project` is the plan's project as resolved in the destination — `<source>:<id>` naming
another source than the one routing chose is refused — and a planless item takes the
destination's `task_projection.project`. The destination must declare `task_projection`.

The current host is `ONETASKGRAPH_HOST`, else the machine's name; a process on another host
than the destination's `task_projection.host` is refused before anything is written.

## The durable intent

`IntentStore::new` takes the absolute intent directory, its authoritative host identity and
the current host identity; a host mismatch refuses before creating any files. A claim holds an
operating-system lock for its source identity — the same digest as the source's local cache —
and external item ID, so simultaneous local owners serialize.

`begin_create` durably records that a create may reach the tracker before it returns
authority to send one. A retry in that phase must reconcile by the recorded origin.
`reconcile` accepts exactly one indexed match and verifies its owner, item identity, origin,
payload digest, host, project, privacy, title, body, priority and repository set. An absent,
multiple or unverifiable match refuses without granting another create. The initial tracker
write carries those keys in the issue's own metadata (`human.item_id`, `human.owner`,
`human.payload_digest`, `human.host`, `onetaskgraph.origin`), so recovery does not depend on a
second mutation landing.

`bind` commits the source-qualified task ID and canonical HTTPS URL before they can be
acknowledged. The source is derived from the qualified identity and a contradictory response
refuses deserialization. An identical binding is a no-op; another ticket cannot replace it. A
bound retry answers the binding and sends the tracker nothing.

A failed durable commit makes the live claim uncertain: discard it and reopen from disk. The
rename may already have installed the transition, so reopening can require positive
reconciliation even when the preceding call failed. New Unix directories and files exclude
group and other access. The journal holds no task title or body. A create the tracker refuses
after the claim was committed stays pending, and is reconciled by hand.

## Terminal projections

A terminal intent is committed before its write and acknowledged after the tracker confirms
it. Terminal intents are immutable: a different state or reason is refused. The state is the
issue's own — its status and its closure (completed for `completed`, not planned for
`dismissed`; their Linear equivalents) — so a terminal projection writes no body unless the
request carries a dismissal `reason`, recorded as `human.reason`; an absent reason stays
absent. **No terminal projection writes a body from a stored copy of an earlier one**: a body
change is composed only against the body as it stands — on GitHub, the issue the `Status`
write's own answer returns. A terminal write whose answer was lost is read once by the retry,
which writes nothing the tracker already holds.

## Context kept between processes

Every projection is its own short-lived process, so two kinds of context are kept, neither
holding an item's title or body.

- **The written item's ids** (`TaskSource::written_context`) are kept on the binding, so a
  terminal write addresses the issue without reading it first.
- **The placement** (`TaskSource::placement_context`) — on GitHub the repository's node id — is
  kept in the store's per-host local cache for the source, `placement.json` beside
  `designator.json` in the directory the cache lives in, whether or not the cache is on. It is
  held under a plugin-owned **setup-context token**: the digest of every input it depends on —
  the board's field definitions and options, the repository the issue is created in, the
  project it is filed under, and the source's declared visibility. A later create recomputes
  the token from the read of its project issue that it makes anyway, which carries the board's
  whole field list, and takes the kept placement only when the two agree; otherwise it reads
  its context afresh and keeps the new one. So the steady state adds no request, and a changed
  option, field, repository, project or visibility is never reused. A source whose create
  needs no setup beyond the read the token would be computed from keeps none (Linear: its
  team resolution is that read).
