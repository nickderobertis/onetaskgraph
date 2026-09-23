# What one session of the live journey costs

The GitHub Projects live journey reaches a rate-limited account that everything else this
repository does draws on too, and until this branch nobody had ever measured what it spends.
This is the measurement, and the before and after of the reduction taken against it.

<!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] The golden-cost test derives these figures from the identical loopback journey and fails whenever this explanation and its checked-in fixture need to move together. -->
The terminal-status consistency journey adds two direct read-backs (20,200 worst-case
nodes), one narrow `cancelled` write, and the board-field halves of both terminal writes.
Those calls are deliberate: the shared real-GitHub and loopback journey now observes the
exact `Done`/`COMPLETED` and `Cancelled`/`NOT_PLANNED` pairs rather than inferring them from
the plugin's normalized status alone.

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
everything one credential does in an hour, and a document well under the node limit says
nothing about what GitHub charged for it — two numbers against two limits. What observes
what a **session** spends in points is the accounting in `src/accounting.rs`, which fills its
per-budget figures from the `x-ratelimit-*` headers a credentialed session's own responses
carry and prints them at the end of every run. That report comes from the live journey in
`tests/live.rs`, which runs in this repository's required check; it is not something this
file's figures can stand in for. **No figure in this file is a measurement of points**, and
that does not change below.

**What is computed offline in points is a per-document price, and it lives elsewhere.**
`worst_case_point_cost` prices one document from its own text under the largest page sizes
this source can be driven with, and `tests/point_cost.rs` pins every document in
`graphql::DOCUMENTS` at what it costs, so a shared fragment that gives a reduction back moves
a number somebody has to change. GitHub is the authority on its own pricing, and the
credentialed lane asks it: the `rateLimit(dryRun: true)` probe it already sends per read
document reports GitHub's own `cost`, and the reconciliation fails naming both figures when
they disagree. **None of that is what a session costs.** It is one document at a time, a
worst case rather than a bill, and the two quantities this file measures over a whole session
are still requests and worst-case nodes; what a whole session consumes of the hourly
allowance is still reported only by a credentialed run's own `x-ratelimit-*` headers.

One of the reductions recorded here **is** about points, and says so: *The board's own
`Labels` field* below argues from GitHub's published pricing rule — the one
`tests/journey/budget.rs` states in full — rather than from anything this file measured. Read
that section as arithmetic over a rule GitHub publishes, which is a different kind of claim
from the two quantities above and a weaker one than the accounting's observation; the
reductions recorded before it claim nothing about points at all. What that section argued by
hand is now a pinned figure: `tests/point_cost.rs` records the board read at **2 points**,
which is that argument's own "about 2" as the released `github-graphql-node-count` computes
it.

## How they are taken

`a_whole_session_of_the_live_journey_costs_what_the_record_beside_it_says`, in
`tests/plugin.rs`, drives the whole of `tests/journey` — the same code the credentialed
target drives — against this crate's loopback fixture board, with no credential and no
third-party API. The session it measures is the **whole** one: the schema verification, the
reconciliation of every read document's node count and price against GitHub's own, the board
and field lookups, every declared capability, this run's own cleanup and the end-of-run
orphan sweep, beside every request the source itself sends.
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

## The board's own `Labels` field, and what dropping it moved

`graphql::BOARD` was the last document selecting
`... on ProjectV2ItemFieldLabelValue{labels(…)}` — the shared `board_issue!` fragment had
already stopped, which is what took the three issue reads under GitHub's node limit. It is
now out of the board read too, and an item's labels come from its content's own `labels`
connection on every path.

In the two quantities this file measures offline, again in the record's own frame:

|                | before  | after  |
| -------------- | ------: | -----: |
| **requests**   |      99 |     99 |
| **node count** | 1757301 | 504801 |

**Not one request either way** — the selection was a field of a document already being
sent — and **1,252,500 worst-case nodes gone, 71% of the session's whole total.** The whole
of it lands in the two rows that send that document: `reading the board` goes from 1043251
nodes over 5 requests to 40751 over the same 5, and the one-request
`node-count reconciliation while reading the board` from 260150 to 10150. Every other row of
the record is byte-for-byte what it was, and `tests/node_count.rs` pins the document itself
at **260,150 → 10,150** nodes.

**This one is about points, which nothing here measures.** The published rule
`tests/journey/budget.rs` states in full is that a call costs `max(1, round(A / 100))`,
where `A` sums, over the call's connections, the product of the page sizes strictly above
each. That label connection sat under `fieldValues(first: 50)` under `items(first: 100)`, so
GitHub resolved it **5,000 times** for one page of board items — against roughly 202 for the
whole of the rest of that document. So `A` for a board read falls from about 5,202 to about
202, and `round(A / 100)` from about **52 points to about 2**. That is arithmetic over
GitHub's own rule rather than an observation: what observes points is still the accounting,
from the `x-ratelimit-*` headers a credentialed run's own responses carry, and this file
measures requests and worst-case nodes and nothing else.

What is given up is nothing. GitHub derives that field from the item's content: for `Issue`
content it *is* the issue's own labels, which the same document selects one level up, and a
`DraftIssue` exposes no `labels` field and cannot carry a value of the board field either —
`LABELS` is absent from `ProjectV2CustomFieldType`, so no project can create such a field,
and `ProjectV2FieldValue`, the whole of what `updateProjectV2ItemFieldValue` accepts, offers
no label member, so no item type's value is writable. A draft therefore reports no labels,
which is what it reported before this change too.

## Making a missed board membership recoverable, and what that document costs

Every document that reaches an issue carries a *page* of `Issue.projectItems` — the board
half of that issue — and this board's own entry can sit past it. That used to be refused,
naming the connection, because with no way to read the rest of it an unreached entry could
not be told from an issue this board really does not hold. `graphql::ISSUE_BOARD_ITEMS` is
the read that tells them apart: one issue's memberships and nothing else, resumed from the
page's own cursor and walked to exhaustion.

In the two quantities this file measures offline, in the record's own frame:

|                | before | after  |
| -------------- | -----: | -----: |
| **requests**   |     99 |    100 |
| **node count** | 504801 | 509901 |

**One request more, and 5,100 worst-case nodes.** Both are the same one thing, and it is
not a read of the board at all: the node-count reconciliation asks GitHub about every query
document this source sends, so a seventh document is a seventh probe, and 5,100 is that
document's own worst case as `tests/node_count.rs` pins it. **The session makes no recovery
read**, and that is the finding rather than an omission — every item on the fixture board
sits on one board, so its memberships arrive exhausted and there is nothing to recover.
That is what this costs a deployment whose issues sit on one board: nothing. Every other
row of the record is byte-for-byte what it was.

## The board memberships a read carries, and what shrinking them moved

`BOARD_ITEMS_PAGE_SIZE` — the page of `Issue.projectItems` that rides along on every
document reaching an issue — was ten and is now **three**. It sits under a page of a
hundred issues, so it multiplies through `SEARCH_ISSUES`, `SUB_ISSUES` and `ISSUE`, and
every point of it is paid whether or not any issue is on a second board. What had kept it
generous was the refusal above; with a miss recoverable, a constant that had to be generous
can be small.

In the two quantities this file measures offline, again in the record's own frame:

|                | before | after  |
| -------------- | -----: | -----: |
| **requests**   |    100 |    100 |
| **node count** | 509901 | 222516 |

**Not one request either way** — a page size is a bound on what a document may return
rather than on how many are sent — and **287,385 worst-case nodes gone, 56% of the
session's whole total.** The whole of it lands in the six rows that carry the fragment or
ask about it: `searching this board's issues` goes from 224400 nodes over 4 requests to
81600 over the same 4, `reading a project's tasks` from 112200 to 40800 over 2, and
`reading one issue` from 2240 to 812 over 4; the three matching reconciliation rows move
with them, 56100 → 20400, 56100 → 20400 and 560 → 203. Every other row of the record is
byte-for-byte what it was, and `tests/node_count.rs` pins the documents themselves at
**56,100 → 20,400** for both the search and the sub-issue read and **560 → 203** for the
issue read.

**Three is chosen against the recovery read's cost, which is a property of the product
rather than of this instrument.** At one, a deployment whose issues commonly sit on two or
more boards would pay that further request *per issue* — order N, against the one page read
per hundred issues a board-scoped read costs today. At three it is reached only by an issue
on four or more boards at once, which keeps the recovery path exceptional rather than
routine for a plausible deployment.

**One observation that is deliberately not a reason.** The estimate in
`tests/journey/budget.rs` divides by the smallest page size this source binds, which was ten
and is now three, so shrinking this constant loosens that bound: the estimate rises from 702
points to 934 even as the session's worst-case nodes fall by more than half. That is real,
and it is recorded here as an observation. It is **not** what chose the value — an estimate
deliberately sized high, whose job is to refuse a run rather than to describe one, must not
be what picks a production constant.

## Comments on a task, and what the five documents behind them moved

A task's comments are its issue's comments, so this source now sends five documents it did
not: `graphql::ISSUE_COMMENTS` and `graphql::COMMENT_ISSUE`, which are queries, and
`ADD_COMMENT`, `UPDATE_COMMENT` and `DELETE_COMMENT`, which are mutations. The journey drives
none of them against the board — nothing it proves reaches a comment — so what moved is the
two things the journey does for **every** document this source sends.

In the two quantities this file measures offline, in the record's own frame:

|                | before | after  |
| -------------- | -----: | -----: |
| **requests**   |    100 |    103 |
| **node count** | 222516 | 222616 |

**Three requests more, and 100 worst-case nodes.** Two of the requests are the node-count and
point-cost reconciliation asking GitHub about the two new query documents —
`reading a task's comments` at 100 nodes, which is its one `comments(first:)` connection and
nothing multiplied through it, and `reading which issue a comment is on` at none. The third is
the mutation schema introspection: three mutations bring three input and three payload types,
thirty-four types in all, and at GitHub's cap of two capped selections a document that is
**nine** documents rather than eight. The three mutations are not reconciled, because
`rateLimit` cannot be asked about a mutation; `tests/point_cost.rs` pins each of the five at
one point. Every other row of the record is byte-for-byte what it was.

The estimate in `tests/journey/budget.rs` moves with the record, as it is built to: **934
points to 941** against the GraphQL budget, and the REST estimate unchanged at 5 requests.

## Safely reconciling configured status options

The guarded status-options operation snapshots the board's `Status` options and every
item's assignment before it writes. The credentialed journey does not invoke that
operator-only operation, but its read document is still reconciled against GitHub beside
every other query document this source can send.

In the two quantities this file measures offline, in the record's own frame:

|                | before | after  |
| -------------- | -----: | -----: |
| **requests**   |    103 |    104 |
| **node count** | 222616 | 227766 |

**One request more, and 5,150 worst-case nodes.** Both belong to the one
`node-count and point-cost reconciliation while snapshotting board Status options and
assignments` row. The session makes no status-options snapshot itself, so every other row
of the record is byte-for-byte what it was. The precondition's estimate includes every
query document this source can send even when the journey does not call it, so the
GraphQL estimate rises from **941 to 961 points**; the REST estimate stays at 5 requests.

<!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] The golden-cost test regenerates session-cost.txt from the identical loopback journey, and these totals are read off that record: the test fails whenever this explanation and its checked-in fixture need to move together. -->
That change and the terminal-status consistency journey above landed independently of one
another, each measured against the same 103-request record, and the record now carries
both: **110 requests and 248,169 worst-case nodes** — the snapshot document's one request
and 5,150 nodes beside the consistency journey's six requests and 20,403 nodes, with no row
that the two changes both moved.

## The estimate the gate is sized from, and what it is not

`tests/journey/budget.rs` derives what this session will cost each of GitHub's two budgets
from the record above and a cost model it states in one place: **702 points** against the
GraphQL budget and **5 requests** against the REST one — 1955 points before the board read
stopped selecting the board `Labels` field. That is an *estimate*, deliberately
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

Every figure in this section was measured **before** the board read stopped selecting the
board's own `Labels` field, so its node counts are in the frame of the 1757301-node session
above rather than the 504801-node one. None of the three findings turns on the size of that
number — each is a comparison between two sessions measured the same way, and all three came
out *no change kept* — so they are left as they were recorded rather than re-run.

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
source's record of itself, and the rebuild the fixture settling loop makes per attempt is
what makes a change nothing here wrote visible at all — the label attached out of band, and
each further item GitHub has not reported yet. That loop settles on the first attempt
against this fixture and so builds exactly one source here, which is why the row above is
what it is; against GitHub it builds one more per attempt it has to make, because a source
answers every board question from one read and asking the same one twice asks GitHub once.
Collapsing any of them would buy one request by deleting a proof.

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

## What a project copy costs, and what moved it

The live journey above is one kind of session. The one that spends this account's GraphQL
allowance day to day is different: an engine projecting a run onto a board copies the
project again on every node's status change, and until this change every such copy read
the whole project to find out what had changed. What a copy costs now has a record of its
own, `tests/fixtures/copy-cost.txt`, in the same two quantities and the same per-call shape
as this file's — requests, and each GraphQL request's worst-case node count under the
variables it really sent — **and no figure in it is points either.**

It is taken by `a_project_copy_into_a_board_costs_what_the_record_beside_the_session_record_says`
in `crates/onetaskgraph/tests/e2e/copy_cost.rs`, which drives the compiled binary against
the loopback fixture board the copy journeys use, with pacing off. It lives in the binary's
crate rather than beside its record because a copy is the engine's and no plugin crate may
depend on the engine; every request the board served is named and counted through this
crate's own `accounting`, so the two records cannot count one document two ways. The four
copies it measures are:

- **(a)** a whole copy of a project of 10 tasks the board does not hold yet;
- **(b)** the same whole copy again, unchanged, with every item recording the destination
  item it landed on at `onetaskgraph.origin` — the shape a write-back's shadow project has;
- **(c)** a member copy of that project naming one task whose status changed;
- **(d)** the same one-member copy of a project of 3 tasks.

|                         | before (a) | after (a) | before (b) | after (b) | (c)  | (d)  |
| ----------------------- | ---------: | --------: | ---------: | --------: | ---: | ---: |
| **requests**            |         69 |        68 |         47 |        23 |    9 |    9 |
| **node count**          |      53150 |     32750 |      19452 |     14583 | 1209 | 1209 |

The **after** columns are this reduction's own after, and (b) has moved once since: reading a
board through both of GitHub's enumerations of it, below, took it to 24 requests and 34,983
nodes. `tests/fixtures/copy-cost.txt` is what a copy costs now, and it is the record the test
holds a copy to; the columns here are a comparison of one change and stay as they were taken.

Per call, before, measured on the same harness against the engine as it stood at 0.2.28:

```
(a)
   11       0  adding an issue to the board
   11       0  creating an issue
   10       0  filing an issue under its project
   11    2200  reading an issue's dependencies
    1   10150  reading the board
    1       0  reading the destination repository
    2   40800  searching this board's issues
   22       0  writing a board field

(b)
   12    2400  reading an issue's dependencies
   34    6902  reading one issue
    1   10150  reading the board
```

After: `tests/fixtures/copy-cost.txt`, which the test above holds every copy to.

**(a) 69 → 68, and (b) 47 → 23: each item's target resolved once.** A project copy planned
the project and every member to learn their destination ids, then planned each of them again
to land it, and compared each against a third read of the same destination item — and read
the project a fourth time to decide whether it had settled. Now every item is read and its
target decided once, before anything is written, and the read that found the target is the
read the comparison uses. In (b) that is the whole move: `reading one issue` falls from 34
to 11, one per item, and `reading an issue's dependencies` from 12 to 11. In (a), where no
item has a target yet, it is the second board-scoped search for the project's counterpart.
The one `reading the board` left in (b) is the walk for items the source no longer holds,
which a whole copy keeps because it was not told which members it carries.

**(c) and (d): 9 requests each, and the same 9.** A member copy reads, compares and writes
the project and the members it names and nothing else, runs no walk for orphans, and so does
not grow with the members it leaves alone. Its 9 are all reads and writes of one issue:
the project's issue and the member's issue, each read once for its target and its edges;
the member's issue read once more by the write, which takes the board's id and the
definitions of the `Status` and origin fields from that issue's own board entry instead of
from every page of the board; the member's blockers read before they are reconciled; the
update; and its two field writes. The test holds that by the documents sent — none is the
board read, the board-scoped search or a project's sub-issue walk — and by the node ids its
issue reads carry.

Two changes in this crate are what took the board out of (c) and (d), and both cost nothing
in the quantities above: neither adds a request or a node to any document.

- **`graphql::ISSUE_DEPENDENCIES` selects the issue's own `body`.** An issue's edges into
  other sources are recorded in its body, and reading them used to walk the board for that
  one field of one item. They now come out of the dependency read already made. A draft,
  whose body that read cannot carry, still reads the board.
- **The board entry every issue read carries names the board's `id`.** With it and the
  field definitions each value names, an update of an item this board holds reads that item
  rather than the board. An item that cannot say enough — one with no board entry naming
  the board, no value of the origin field, or no `Status` value when the write carries a
  status — is not guessed at: the write reads the board exactly as before. **That fallback
  costs one request more than it did** when the command has not already read the board —
  the item read that could not describe it — and nothing more once it has, because a board
  this command read is used as it is. A copy whose items were written by a copy, as a
  projection's are, does not take it.

What stays as it was: a member the copy names that records no origin is still looked for
by origin before it is created — that is correctness, not slack — and creating an item
still reads the board, which a create needs for the repository it files the issue in.

## Reading a board through both of GitHub's enumerations of it, and what that costs

A whole-board read now sends the board-scoped issue search beside `graphql::BOARD`, because
neither of GitHub's two enumerations of one board is complete on its own. `ProjectV2.items`
is a projection GitHub rebuilds behind the write, and an item added with
`addProjectV2ItemById` can be missing from it for minutes; the search reports the same item
within seconds. The reasoning and the measurements behind that are in the crate
documentation, at `GitHubProjectsSource::board`. This section is only what it costs.

In the two quantities this file measures offline, in the record's own frame:

|                | before | after  |
| -------------- | -----: | -----: |
| **requests**   |    110 |    111 |
| **node count** | 248169 | 268569 |

**One request more over the whole session, and 20,400 worst-case nodes** — one row moves,
`searching this board's issues`, from 4 requests to 5. Every other row of the record is
byte-for-byte what it was, `reading the board` included.

One request rather than five is the whole point of `GitHubProjectsSource::search_cache`. A
board read and a project listing are two questions about the same board, and both of them now
want that search: reading it once per source is what keeps the session's five whole-board
reads from buying five searches nobody asked for. `tests/fixtures/copy-cost.txt` is where that
shows most plainly — a whole project copy already searched once and searches once still, and
what moves there is the **repeat** copy, which read the board without ever listing a project
and so had no search to share.

The estimate in `tests/journey/budget.rs` moves with the record, as it is built to: **1042
points to 1112** against the GraphQL budget, and the REST estimate unchanged at 5 requests.
That is the worst-case price of one more `SEARCH_ISSUES`, and it estimates high on purpose —
the session the record is taken from is attributed 106.

What this does not buy is a reduction anywhere: it is a request spent to make a read correct,
and it is spent on every command that asks a question about the whole board. A command that
asks about one project still asks that project — `graphql::SUB_ISSUES`, no board read and no
search — so nothing about the cost of the reads this source was shaped to make instead of a
board read has changed.
