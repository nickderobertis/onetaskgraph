# Projection intent API

The Rust task-projection domain owns the [request and binding declarations](../crates/onetaskgraph-core/src/task_projection/mod.rs).
Their schema roots come from those declarations through `schema_bundle`; the bundle's
versioned golden detects drift. Use the exported schema for accepted fields and values.

`IntentStore::new` takes an absolute intent directory, its authoritative host identity,
and the current host identity. A host mismatch refuses before creating any files. The
caller supplies a validated placement using the same source-identity digest as the
source's existing local cache.

A claim holds an operating-system lock for its source identity and external item ID.
`begin_create` durably records that a create may reach the tracker before it returns
authority to send one. A retry in that phase must reconcile by the recorded origin.
`reconcile` accepts exactly one indexed match and verifies its owner, item identity,
origin, payload digest, host, project, privacy, title, body, priority and repository set.
An absent or unverifiable match refuses without granting another create.

`bind` commits the source-qualified task ID and canonical HTTPS URL before they can be
acknowledged. The source is derived from the qualified identity and a contradictory
response refuses deserialization. An identical binding is a no-op; another ticket cannot
replace it. The initial tracker write must carry the metadata verified by
[`ProjectionClaim::reconcile`](../crates/onetaskgraph-core/src/task_projection/intent.rs),
so recovery does not depend on a second mutation landing.

Terminal intent is committed before its write and acknowledged after the tracker confirms
it. Terminal intents are immutable; an absent reason stays absent. The
journal holds no task title or body. Preserve intervening tracker edits by reading current
content whenever a terminal mutation must change its body.

A failed durable commit makes the live claim uncertain: discard it and reopen from disk.
The rename may already have installed the transition, so reopening can require positive
reconciliation even when the preceding call failed. New Unix directories and files exclude
group and other access. Tracker reads and plugin setup-context validation remain the
responsibility of the caller using the existing engine and plugin interfaces.
