# What one session of the live journey costs

The GitHub Projects live journey reaches a rate-limited account that everything else this
repository does draws on too, and until this branch nobody had ever measured what it spends.
This is the measurement, and the before and after of the reduction taken against it.

## What these numbers are, and what they are not

Two quantities, both taken offline:

- **Requests** — how many HTTP requests one whole session sends. Exact: the source makes the
  same calls against the fixture board as against GitHub.
- **Node count** — the sum, over every GraphQL request, of the **worst-case number of nodes
  the document it sent may return** under the bindings that request really used. That is
  `nodeCount`, which GitHub limits **per query** and refuses a query above before executing
  it. It is arithmetic over the document's own text, computed by
  `github-graphql-node-count`.

**Neither of them is rate-limit points.** `cost` is metered by GitHub per call across
everything one credential does in an hour, and nothing offline can observe it — a document
well under the node limit says nothing about what GitHub charged for it. What observes
points is the accounting in `src/accounting.rs`, which fills its per-budget figures from the
`x-ratelimit-*` headers a credentialed session's own responses carry and prints them at the
end of every run. That report comes from the live journey in `tests/live.rs`, which runs in
this repository's required check; it is not something this file's figures can stand in for.
**Nothing here claims a reduction in points.**

## How they are taken

`a_whole_session_of_the_live_journey_costs_what_the_record_beside_it_says`, in
`tests/plugin.rs`, drives the whole of `tests/journey` — the same code the credentialed
target drives — against this crate's loopback fixture board, with no credential and no
third-party API. The session it measures is the **whole** one: the schema verification, the
node-count reconciliation, the board and field lookups, the start-of-run residue sweep,
every declared capability, and the cleanup, beside every request the source itself sends.
`tests/fixtures/session-cost.txt` is the checked-in record of the figures below, and that
test fails when a session stops costing them.

Two things differ between the two drives, and neither changes what is sent. One is where
the calls go — the fixture board rather than `api.github.com`, which is what makes the
measurement free. The other is pacing: the fixture drive turns off the interval this source
spaces its own content-creating mutations by, which changes how long a session takes and
nothing about how many requests it makes or what each carries.

## Before and after

|                | before  | after   |
| -------------- | ------: | ------: |
| **requests**   |     120 |      98 |
| **node count** | 1757401 | 1757301 |

Per call, before:

```
    1       0  DELETE /repos/{owner}/{repo}/labels/{name}
    2       0  GET /repos/{owner}/{repo}/labels
    1       0  POST /repos/{owner}/{repo}/labels
    1       0  account allowance after the node-count reconciliation
    1       0  account allowance before the node-count reconciliation
    6       0  adding an issue to the board
    2     100  board page size probe
    6       0  creating an issue
    6       0  deleting an issue
    3       0  filing an issue under its project
    6       0  live artifact cleanup
    3     300  live artifact lookup
    1       0  live label attachment
    1       0  live origin field cleanup
    1       0  live origin field creation
    1       0  mutation contract introspection
   28       0  mutation type introspection
    1   56100  node-count reconciliation while reading a project's tasks
    1     200  node-count reconciliation while reading an issue's dependencies
    1     560  node-count reconciliation while reading one issue
    1  260150  node-count reconciliation while reading the board
    1       0  node-count reconciliation while reading the destination repository
    1   56100  node-count reconciliation while searching this board's issues
    1       0  nominated board lookup
    2  112200  reading a project's tasks
    9    1400  reading an issue's dependencies
    4    2240  reading one issue
    5  1043251  reading the board
    2       0  reading the destination repository
    2       0  recording a dependency
    4  224400  searching this board's issues
    1       0  updating an issue
    4     400  writable field discovery
   10       0  writing a board field
```

After: `tests/fixtures/session-cost.txt`, which the test above holds the session to.

## What the budget precondition added afterwards

The reduction's two figures above are a comparison of the reduction, and they stand. What
`tests/fixtures/session-cost.txt` records **now** is one request more — **99 requests, node
count 1757301** — because the budget precondition that landed after it makes one:
`GET /rate_limit`, the account's allowance, before the session does any of the work it
exists to do. That read is deliberately in the record rather than outside it, so what the
gate itself costs is measured beside everything else instead of assumed; the node count is
untouched, because a REST call sends no document.

Nothing about the reduction moved. The rows are the reduction's rows plus one, and every
figure in the table above is still what those two changes were worth.

## The estimate the gate is sized from, and what it is not

`tests/journey/budget.rs` derives what this session will cost each of GitHub's two budgets
from the record above and a cost model it states in one place: **1955 points** against the
GraphQL budget and **5 requests** against the REST one. That is an *estimate*, deliberately
high — it is what refuses a run rather than what a run spends, and an estimate that is too
low is the thing that exhausts a shared budget.

**It is still not a measurement of points, and neither is anything else in this file.** What
observes points is the accounting, from the `x-ratelimit-*` headers a credentialed
session's own responses carry. What the gate adds is that the session report now prints the
estimate beside those figures, so a run says how far the model was from GitHub's own numbers
rather than asking anybody to trust it.

## What was kept, and what each change measured

Both changes landed in one commit, so read each summary's arithmetic off the per-call rows
rather than off the totals: subtracting the two totals gives their sum, not either one. The
intermediate figures below are measured rather than derived — the same session, driven with
only the first change applied, costs 100 requests and 1757401 nodes, which is 99 in the
table's frame. Every request figure in this section is in that frame, the reduction's own:
it sets the budget precondition's one `GET /rate_limit` aside, because that read landed
after the reduction and is no part of what either change was worth. The node counts need no
such reading — a REST call sends no document.

**Eight introspections instead of twenty-nine.** The schema verification asked GitHub for
the `Mutation` type and then for each of twenty-eight input and payload types, one request
each — the `mutation contract introspection` (1) and `mutation type introspection` (28) rows
of the before record, twenty-nine requests in all. GitHub allows any number of aliased root
fields on one query and `__type` is not a connection, so the contract folds into documents
that add nothing to the node count. What bounds the fold is a separate limit: **GitHub caps
how many times one document may select a given introspection field, at two.** A first
version of this change put all twenty-nine roots in one document and GitHub refused it
outright — `INTROSPECTION_LIMIT_EXCEEDED`, *"__Type.fields (14), __Type.inputFields (15)"* —
which cost the whole request rather than part of the answer, and which only a credentialed
run meets, because the loopback board answers whatever it is asked. So the fold is batched
to that cap: fifteen `inputFields` selections and fourteen `fields` ones, two of each per
document, is **eight** documents. Every name, input, payload, member and type signature the
checks held GitHub to is still asked for, from the same two tables. Those twenty-nine
requests become the eight `mutation schema introspection` rows of the after record: **29
replaced by 8, a net reduction of 21.**
**120 → 99 requests; node count unchanged at 1757401.**

`no_introspection_document_selects_a_capped_field_more_often_than_github_allows`, in
`tests/plugin.rs`, holds the batch to GitHub's own stated number and to asking about every
type exactly once, so a later fold cannot buy requests back by re-tripping that cap or by
dropping a type.

**One walk of the board's field connection instead of two.** `ensure_origin_field` and
`live_write_status` each read the project's fields for themselves, and one read answers
both — nothing the first creates can change what the second reads, because the `Status`
field was on the board before either ran. It is the `writable field discovery` row, which
falls from 4 requests and 400 nodes to 3 and 300: the last of the 22 requests the two
changes remove between them, and the whole of the node-count move.
**99 → 98 requests; 1757401 → 1757301 nodes.**

## The three places this step was told to look

**The lane's own setup, residue sweep and cleanup.** This is where both kept changes came
from, above. What is left there is not slack: the two allowance reads either side of the
node-count reconciliation are the observation that asking is free, the two board page-size
probes are the assertion that `max_page_size` is GitHub's own connection maximum and not a
guess, and each artifact-lookup and label-listing request is either a sweep or the
confirming re-read that says the sweep worked. The six documents of the node-count
reconciliation cannot be folded into one: `rateLimit(dryRun: true)` reports the count for the
whole operation it sits in, so merging six documents would answer their sum and reconcile
none of them.

One further candidate was examined and **rejected**: the cleanup deletes each artifact's
board item and then its issue, and on GitHub deleting an issue is understood to remove the
project items whose content it was, which would make the first of those two redundant and
save six requests a session. It is not kept, because this instrument cannot settle it — a
fixture board modelling that behaviour would only be answering back the assumption that was
put into it, and the property is GitHub's to demonstrate.

**The source resolving the same board and repository repeatedly.** Measured directly, as a
pair. Adding one more source instance to the journey and listing tasks through it costs
**one more request and 260150 more worst-case nodes**, and the whole of that lands in one
row, `reading the board`, which goes from 5 requests and 1043251 nodes to 6 and 1303401.
Making that same extra read through a source the journey already has costs **nothing at
all** — not one request and not one node — because a source holds its board and its
destination repository for its own lifetime and shares neither with the next one. So what this session pays really does scale
with the number of commands the journey stands in for rather than with how much it reads.
**No change is kept.** Every source this journey builds is load-bearing: the read-configured
one proves that a source configured with no `status_mapping` reads the board, each rebuild
inside `await_on_board` is what makes GitHub's own view visible rather than the writing
source's record of itself, and the rebuild after the label is attached out of band is what
makes a change nothing here wrote visible at all. Collapsing any of them would buy one
request by deleting a proof.

**The page sizes the source asks for.** Measured by driving the journey with its page limit
at 100 and at 5, against 50 as it stands. All three are whole sessions as this branch now
sends them, so each includes the budget precondition's one request:

| journey page limit | requests | node count |
| ------------------ | -------: | ---------: |
| 100                |       99 |    1757701 |
| **50 (kept)**      |   **99** | **1757301** |
| 5                  |       99 |    1756941 |

The finding is that **a caller's limit reaches the wire in exactly one document** — the
dependency read — and nowhere else: this source filters before it pages, so every other read
binds `MAX_PAGE_SIZE` whatever the caller asked for, and the other two sessions above differ
from the kept one in that row alone. `reading an issue's dependencies` carries 1800 nodes at
100, 1400 at 50 and 1040 at 5, over 9 requests in all three, which is both the whole 760-node
spread in the table — 400 nodes above the kept limit and 360 below it — and why every row of
it reads the same request count. **No change is kept.** Raising the limit costs nodes for nothing, and
lowering it looks free here only because this fixture board holds fewer rows than a page:
the round trip a smaller page buys appears on a board with more rows than the limit, which
is the half of the trade this instrument cannot see. `MAX_PAGE_SIZE` itself is out of
scope — making the source read less is a different decision from making it affordable.
