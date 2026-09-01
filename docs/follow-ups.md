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

**This is now narrower than it was, and the part it no longer covers is settled.** A read
that is *not* about the whole board does not read the board at all: one item by its own id
resolves that id, one project's tasks come from that project issue's own sub-issues, and
the projects a board holds come from an issue search scoped to it. Those three are answered
by GitHub each time they are asked, so a change something else made to that item is visible
to a source that has already read the board — which is the thing this entry says a
long-lived caller has no verb for, for the reads where the question is about one item. What
is left is the reads whose cost is the board's size anyway: an unconstrained task list, a
document list, the label list, and every write. Those still answer from the one board read,
and that is what the rest of this entry is about.

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

What is true today is pinned rather than only described:
`a_board_changed_by_something_else_is_seen_by_the_next_source_and_not_by_this_one` retitles
a board item without going through the source, asserts that source still reports the title
it read *through a whole-board read*, asserts that a source built the way the next command
builds one reports the new one, and asserts that the same source reading that item **by its
id** reports the new one too without buying a second board read. So the behaviour this entry
rules on fails a test when it moves rather than turning a live lane red — and so does the
half of it that has already moved.

Settling what is left means deciding what a long-lived caller is owed for the whole-board
reads: a source-level way to discard the cached board, a bound on how long a read may be
answered from it, or a ruling that holding a source across commands is not a supported
shape and saying that in the crate's own documentation. The first two both cost the
single-request guarantee above, so whichever is chosen has to say what the three tests
named here should assert instead.
