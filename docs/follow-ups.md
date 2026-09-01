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

## Documents: one of the four hosted sources has none yet

Unsupported fields: `onetaskgraph-github-projects` `documents`

The plugin contract carries documents — `Document`, `Location`, `DocumentQuery`, the
`documents` capability, and the four `TaskSource` methods — and `in-memory`, `local-md`
and `linear` implement them. The one above declares `documents: Support::Unsupported` and
keeps the four methods' defaults, which refuse: `documentless` for the two reads,
`unwritable` for the two writes.

That is sound as it stands, and it is what the capability rules ask for. `documents` is
not a predicate the engine compensates for; it says whether a source has documents at all,
the engine reads it once at the handshake exactly as it reads `writes`, and a source that
says it has none is never asked for one. So no caller can reach a refusal by accident, and
a document read across several sources reports such a source as holding none rather than
as having failed. What a plugin must never do here is answer an empty page, which reads as
a source that has documents and holds none matching.

It is a gap rather than a limit, and this plugin's own verdict row says why: a GitHub
repository holds files a board item could name.

Closing the entry means: implementing that plugin's four methods, flipping its `documents`
to `Support::Native`, updating its row in `crates/onetaskgraph/tests/e2e/fixtures.rs` —
which the reconciliation journey will otherwise fail, naming the row and the field — and
deleting the line above together with the verdict row's wording. That is what `local-md`
and `linear` did. `local-md` added `documents/` beside the `tasks/` and `projects/` it
already read, and `docs/local-md.md` records why the folder is the discriminator. `linear`
read Linear's own first-class `Document`, which brought back one thing worth knowing before
the same is attempted elsewhere: a backend that *has* the concept can still be missing a
field the shared dataset gives it. Linear's `Document` carries no labels where its `Issue`
and `Project` do, so what closed the entry was not only the four methods but a new
dimension of the shared journey table — `Ready::labels_its_documents` — so a row states
what its source really reports and the shared journeys drive that claim rather than one
plugin's shape standing for every plugin's.

## What a Windows location is spelled like, and who decides

`local-md` reports a location by handing `std::fs::canonicalize` to `Location::Path`. On
Windows that answers with an extended-length path — `\\?\C:\…` — and that spelling is not
what `docs/local-md.md` implies when it says a reader "can print the path or read the
contents out for a person, without knowing anything about this plugin": no other language
in this repository writes a path that way, and many tools a person would hand it to refuse
it. The TypeScript SDK's own test runner is one of them — `fs.realpathSync` there returns
an extended-length path unchanged rather than resolving it — which is evidence for that
reading rather than an argument against it. The spelling is nevertheless *the* canonicalized
absolute path on that platform, which is what the source promises, so both readings stand
and the choice between them is the contract's rather than a test's.

Nothing is wrong today. Every assertion over a location compares the file named rather
than the string, and none of them depends on how a runtime spells a canonical path: the
Rust tests canonicalize on the expectation side too, the Python SDK test asks the operating
system with `Path.samefile`, and the TypeScript SDK test writes a sentinel through the path
it built itself and reads it back through the path the source reported, which one file
holds and no other file can. That is also what makes them independent of the symlinked
temporary tree macOS hands out.

Settling it means deciding whether `local-md` strips the `\\?\` prefix before reporting —
so a Windows location reads `C:\…` — and if so, saying that in `docs/local-md.md` beside
the sentence above and stripping it in the six Rust sites that canonicalize on the
expectation side, so the two halves of each comparison keep agreeing.
