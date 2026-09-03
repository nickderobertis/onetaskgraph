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

The one difference between the two drives is pacing: the fixture drive turns off the
interval this source spaces its own content-creating mutations by, which changes how long a
session takes and none of what it sends.

## Before and after

|                | before  | after   |
| -------------- | ------: | ------: |
| **requests**   |     120 |      91 |
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

## What was kept, and what each change measured

**One introspection instead of thirty.** The schema verification asked GitHub for the
`Mutation` type and then for each of twenty-eight input and payload types, one request each.
GitHub allows any number of aliased root fields on one query and `__type` is not a
connection, so the whole contract fits in one document that adds nothing to the node count.
Every name, input, payload, member and type signature the checks held GitHub to is still
asked for, from the same two tables.
**120 → 92 requests; node count unchanged at 1757401.**

**One walk of the board's field connection instead of two.** `ensure_origin_field` and
`live_write_status` each read the project's fields for themselves, and one read answers
both — nothing the first creates can change what the second reads, because the `Status`
field was on the board before either ran.
**92 → 91 requests; 1757401 → 1757301 nodes.**

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

**The source resolving the same board and repository repeatedly.** Measured directly, by
adding one more source instance to the journey and reading through it: **one more command
costs one more request and 56100 more worst-case nodes**, because a source holds its board
and its destination repository for its own lifetime and shares neither with the next one. So
what this session pays really does scale with the number of commands the journey stands in
for rather than with how much it reads. **No change is kept.** Every source this journey
builds is load-bearing: the read-configured one proves that a source configured with no
`status_mapping` reads the board, each rebuild inside `await_on_board` is what makes GitHub's
own view visible rather than the writing source's record of itself, and the rebuild after the
label is attached out of band is what makes a change nothing here wrote visible at all.
Collapsing any of them would buy one request by deleting a proof.

**The page sizes the source asks for.** Measured by driving the journey with its page limit
at 100 and at 5, against 50 as it stands:

| journey page limit | requests | node count |
| ------------------ | -------: | ---------: |
| 100                |       91 |    1757701 |
| **50 (kept)**      |   **91** | **1757301** |
| 5                  |       91 |    1756941 |

The finding is that **a caller's limit reaches the wire in exactly one document** — the
dependency read — and nowhere else: this source filters before it pages, so every other read
binds `MAX_PAGE_SIZE` whatever the caller asked for, and the whole ±400-node spread above is
that one document. **No change is kept.** Raising the limit costs nodes for nothing, and
lowering it looks free here only because this fixture board holds fewer rows than a page:
the round trip a smaller page buys appears on a board with more rows than the limit, which
is the half of the trade this instrument cannot see. `MAX_PAGE_SIZE` itself is out of
scope — making the source read less is a different decision from making it affordable.
