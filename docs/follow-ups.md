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


## Linear: a project this source created cannot be removed again

A copy is either complete or it leaves the destination as it found it, and the engine makes
that true by undoing the writes of a copy that could not finish — through
`TaskSource::delete_task` and `TaskSource::delete_project`. `onetaskgraph-linear` implements
the first with `issueDelete` and refuses the second, naming what the refusal costs. So a
*project* copy into Linear that fails part way reports the project it created and leaves it
there, rather than taking it back; every other destination this repository ships takes both
back, and `EngineError::CopyNotUndone` is what names the difference to the user rather than
their discovering it on the board.

It is a gap rather than a limit, and the gap is deliberate about *evidence*. This crate is
written against a pinned subset of Linear's schema —
`crates/onetaskgraph-linear/tests/fixtures/schema.graphql`, recorded on 2026-08-24 — and
that subset carries `issueDelete` and no project delete. Adding a mutation nobody observed
is how a plugin starts sending Linear operations this repository cannot check, and the
pinned-schema test would be checking an invention against itself.

Closing it means: re-observing Linear's published schema, pinning the project-delete
mutation there if it carries one, implementing `delete_project` against it, and deleting
this entry and the comment on that method in the same change. If the observation says
Linear has no such mutation, this stops being a gap and the comment becomes the ruling.
