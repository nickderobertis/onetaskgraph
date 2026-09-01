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

Unsupported fields: `onetaskgraph-linear` `documents`

The plugin contract carries documents — `Document`, `Location`, `DocumentQuery`, the
`documents` capability, and the four `TaskSource` methods — and `in-memory`, `local-md` and
`github-projects` implement them. The one above declares `documents: Support::Unsupported`
and keeps the four methods' defaults, which refuse: `documentless` for the two reads,
`unwritable` for the two writes.

That is sound as it stands, and it is what the capability rules ask for. `documents` is
not a predicate the engine compensates for; it says whether a source has documents at all,
the engine reads it once at the handshake exactly as it reads `writes`, and a source that
says it has none is never asked for one. So no caller can reach a refusal by accident, and
a document read across several sources reports such a source as holding none rather than
as having failed. What a plugin must never do here is answer an empty page, which reads as
a source that has documents and holds none matching.

It is a gap rather than a limit for both, and each plugin's own verdict row says why:
Linear has documents of its own.

Closing an entry means: implementing that plugin's four methods, flipping its `documents`
to `Support::Native`, updating its row in `crates/onetaskgraph/tests/e2e/fixtures.rs` —
which the reconciliation journey will otherwise fail, naming the row and the field — and
deleting that plugin's line above together with the verdict row's wording. That is what
`in-memory` did: its `documents` is a `CapabilityConfig` key a document sets, and a
configuration listing documents without declaring it is refused where it is read. It is
what `local-md` did: `documents/` is a folder beside the `tasks/` and `projects/` it
already read, and `docs/local-md.md` records why the folder is the discriminator. And it
is what `github-projects` did, from a backend with no document type at all: a board holds
issues, so a document there is the issue whose title begins `DESIGN: `, and
`docs/metadata.md` settles what that discriminator implies.

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

## What "the life of one command" means to a caller that is not the binary

`github-projects` reads the whole board once and answers from that read for as long as the
source object lives. That is what stops a copy of a project re-reading the board for every
item it writes, and it is why `a_project_copy_reads_the_board_and_the_repository_once_for_the_whole_command`
and `the_board_this_command_reads_holds_what_this_command_has_itself_written` both hold the
board to a single request. The read is completed from what this process has itself created
and replaced where this process has itself written, so nothing this source did is ever
missing from its own view.

The proxy for "one command" is the source object's lifetime, and for the binary that proxy
is exact: one invocation is one process, one source and one read. It is not exact for a
caller that links the crate and holds a source across what would be several commands — the
consumer the recorded decision about `Engine::copy` names. Such a caller sees a board that
went stale the moment something outside its own source changed the board, and has no verb
that says "read it again".

Nothing is wrong for any consumer this repository ships. The CLI is a process per command,
and both SDKs drive that binary as a subprocess, so all three get a fresh read per command
by construction. The one place the proxy showed through is this crate's own live journey,
which drives every capability through a single source and attaches a label to an issue
through GitHub's REST API rather than through the source; it now takes a source built the
way the next command would build one for the legs after that mutation, and says so at the
site.

Settling it means deciding what a long-lived caller is owed: a source-level way to discard
the cached board, a bound on how long a read may be answered from it, or a ruling that
holding a source across commands is not a supported shape and saying that in the crate's
own documentation. The first two both cost the single-request guarantee above, so whichever
is chosen has to say what the two tests named here should assert instead.
