# Tracked follow-ups

Work this repository has *decided* on and deliberately not done yet. An entry here is a
ruling, not a wish: it says what is true today, why the state it describes is sound as it
stands, and what would change if it were closed.

A capability a source plugin records as unsupported says which of two things it is, in one
word: `unimplemented` — nobody has written it — or `unsupportable` — the backend cannot do
it. The difference is what stops a gap being read as a limit and left forever, and it is
enforced rather than asked for. `scripts/check-capability-verdicts.sh`, a target in
`just check`, reads every plugin's verdict table and holds it against the
`Unsupported fields:` lines below in both directions: an unimplemented capability with no
entry here fails, and an entry here naming a capability since implemented fails too.

Closing an entry means deleting it in the same change that does the work.

## Linear: title and content search are unimplemented

Unsupported fields: `onetaskgraph-linear` `search_title`, `search_content`

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
`onetaskgraph-linear`'s module documentation. `scripts/check-capability-verdicts.sh` reads
the field line above and fails while it names a capability that plugin no longer calls
unsupported, so the entry cannot outlive the gap it describes.

## Documents: no source has one yet

Unsupported fields: `onetaskgraph-github-projects` `documents`
Unsupported fields: `onetaskgraph-in-memory` `documents`
Unsupported fields: `onetaskgraph-linear` `documents`
Unsupported fields: `onetaskgraph-local-md` `documents`

The plugin contract carries documents — `Document`, `Location`, `DocumentQuery`, the
`documents` capability, and the four `TaskSource` methods — and **no source implements
them**. Every plugin declares `documents: Support::Unsupported`, and the four methods keep
their defaults, which refuse: `documentless` for the two reads, `unwritable` for the two
writes.

That is sound as it stands, and it is what the capability rules ask for. `documents` is
not a predicate the engine compensates for; it says whether a source has documents at all,
the engine reads it once at the handshake exactly as it reads `writes`, and a source that
says it has none is never asked for one. So no caller can reach a refusal by accident, and
a document read across several sources reports such a source as holding none rather than
as having failed. What a plugin must never do here is answer an empty page, which reads as
a source that has documents and holds none matching.

It is a gap rather than a limit for all four, and each plugin's own verdict row says why:
Linear has documents of its own; a GitHub repository holds files a board item could name;
a folder of Markdown is already files on disk; and the in-memory source holds whatever a
document configures it to. `documents` is the one `in-memory` field that is *not* a
`CapabilityConfig` key, because a key that could declare it native would let a
configuration claim something no code there can serve.

Closing an entry means: implementing that plugin's four methods, flipping its `documents`
to `Support::Native`, updating its row in `crates/onetaskgraph/tests/e2e/fixtures.rs` —
which the reconciliation journey will otherwise fail, naming the row and the field — and
deleting that plugin's line above together with the verdict row's wording.
