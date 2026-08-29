# Tracked follow-ups

Work this repository has *decided* on and deliberately not done yet. An entry here is a
ruling, not a wish: it says what is true today, why the state it describes is sound as it
stands, and what would change if it were closed. A capability recorded as unsupported in a
plugin's own documentation names its entry here when the reason is *unimplemented* rather
than *unsupportable* — the difference between the two is what stops a gap being read as a
limit and left forever.

Closing an entry means deleting it in the same change that does the work.

## Linear: title and content search are unimplemented

`onetaskgraph-linear` declares `search_title` and `search_content` as
`Support::Unsupported`, so the engine over-fetches and narrows in memory. That is correct
and sound — capability rule 2 says an unsupported predicate is *ignored*, never
half-applied, and the shared journeys assert this source returns the same rows every
native source returns with the plan naming the engine as what applied them.

It is a gap rather than a limit. Linear's published API offers issue search —
`searchIssues` is a documented operation of it — so nothing about the remote service makes
a title-only or a body-only match impossible. What is missing is a production operation in
this crate that sends one, pinned against `crates/onetaskgraph-linear/tests/fixtures/schema.graphql`
the way every other operation there is.

Closing it means: adding that operation, pinning it, flipping both fields to
`Support::Native`, updating this plugin's row in
`crates/onetaskgraph/tests/e2e/fixtures.rs` — which the reconciliation journey will
otherwise fail, naming the row and the field — and deleting this entry and the ruling in
`onetaskgraph-linear`'s module documentation.

Deliberately out of scope of the change that recorded it: that change's subject was making
the journey table prove every declared capability, and implementing a new remote operation
inside it would have been a second, unreviewed thing.
