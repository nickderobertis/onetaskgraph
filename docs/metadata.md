# Custom metadata, repositories, and where each source keeps them

A task and a project each carry `metadata` — an ordered, string-keyed map of arbitrary
JSON — and `repositories`, an ordered list of normalized origins. Both default to empty,
so a document, a configuration and a query written before they existed mean exactly what
they meant.

Metadata is how a consumer's own attributes ride on a task without becoming vocabulary of
a general task framework. A plan node's persona, its turn budget, its publication policy:
none of those belong in this product's model, and all of them have to survive a round
trip through the ticketing system the user already works in.

## Reserved key prefixes

Keys are free-form, with two prefixes reserved:

<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Held by a drift gate
     rather than by words: `the_reserved_key_inventory_names_exactly_the_keys_the_code_spells`
     in `crates/onetaskgraph-e2e/tests/e2e/surface.rs` reads this bullet, and fails when the keys it
     lists or the count it states in words differ from the constants each key is spelled once
     as — `MetadataKey::COPIES_KEY` among them. -->
- `onetaskgraph.` belongs to this product. It defines exactly ten keys, each spelled
  once so no source can invent its own: `onetaskgraph.repositories`
  (`Repository::METADATA_KEY`), `onetaskgraph.depends_on`
  (`DependencyEdge::RECORDED_KEY`), `onetaskgraph.delivers` (`TaskRef::DELIVERS_KEY`),
  `onetaskgraph.delivered_by` (`TaskRef::DELIVERED_BY_KEY`), `onetaskgraph.item_kind`
  (`ItemKind::METADATA_KEY`), `onetaskgraph.template` (`MetadataKey::TEMPLATE_KEY`),
  `onetaskgraph.copies` (`MetadataKey::COPIES_KEY`), `onetaskgraph.members`
  (`MetadataKey::MEMBERS_KEY`) and `onetaskgraph.member_of` (`MetadataKey::MEMBER_OF_KEY`)
  in the contract crate, and
  `onetaskgraph.origin` (`GlobalId::ORIGIN_KEY`) in the engine —
  that last one carries a *qualified* id, whose contents no plugin ever constructs or
  interprets, though `github-projects` routes the key itself into a text field of its own.
<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->
- `onepipeline.` belongs to that consumer.

Every other key is the caller's. A source returns it exactly as it holds it — the same
value, of the same JSON type — and this product never interprets it.

### `onetaskgraph.template`: where a rendered item came from

A task, a project or a document created or regenerated from a template — `task create`,
`project create`, `document create`, `task render`, `project render`, `document render`, or
the engine's `create_task`, `create_project` and `render_task` from Rust — records this entry,
computed the same way for all three; one created from a plain body records none:

```json
{"template": "<string>", "digest": "sha256:<hex>", "body_digest": "sha256:<hex>", "answers_digest": "sha256:<hex>"}
```

- `template` is the absolute path of a `--template` file, a loader document's `reference`
  verbatim, or a library caller's own string.
- `digest` is the template chain's digest the content was rendered with — the one `template
  variables` reports.
- `body_digest` is the SHA-256 of the item's content exactly as written.
- `answers_digest` is the SHA-256 of the resolved answers, defaults applied, as canonical
  JSON: keys sorted at every depth, no insignificant whitespace, UTF-8.

Its schema is the `TemplateProvenance` root of `onetaskgraph schema`. It has **no length
cap**: three fixed-size hashes and the caller's string, bounded only by what the destination
bounds a whole item by, which the source reports when a write exceeds it. A copy carries it
like any other metadata. Only a rendering write sets it: a `--metadata` key, `metadata set`
and a caller's `MetadataKey` can never name it, because the whole `onetaskgraph.` namespace
is refused there.

From the entry alone, a hand edit — the content's own hash is not `body_digest` — and a
changed template — the chain's digest is not `digest` — are both visible. **The accepted
weakness:** a check that reads only these hashes trusts them, so provenance forged by hand,
hashes and all, passes it. It says what an item was rendered from, not who wrote it.

**The answers are not in it, and are in no other metadata key either.** They are kept only
where an item is authored as a file of its own — `local-md` keeps them in a block of the
item's file, after its body and before any comments section — and nowhere else: not in the
content, not in the metadata, and not in a comment, so a hosted item carries the four
strings above and nothing of its answers but what the template rendered into its content.
A copy carries content and metadata alone, so **it never carries answers**, at either end:
a board item copied from a folder holds the provenance and no answers, and a file copied
back out of the board holds no answers block. A regenerate starts from the stored answers
only when they hash to `answers_digest`; otherwise — a source that keeps none, or answers
edited by hand — it needs every required answer again.

#### The template loader document

A caller that layers templates of its own states the result as a **loader document**, and
this product renders exactly what it says — it never resolves a caller's layers, never runs
a caller's command, and never turns a recorded `reference` into a location:

```json
{"reference": "<string>", "entry": "<name>", "search_path": ["<absolute dir>", "..."], "templates": [{"name": "<name>", "source": "<text>"}], "digest": "sha256:<hex>"}
```

`reference` is required and non-empty, and is what the item records as `template`, verbatim.
`entry` is required, and is loaded over the `search_path` directories in order and then the
`templates` pairs, exactly as a template's own `extends`, `include` and `import` names are.
`digest`, when present, must equal the digest that chain computes, or the render is refused
naming both. Every other key is ignored. A malformed document, a missing key, a directory
that is not absolute or cannot be read, and a mismatched digest are each refused by name,
and nothing is written. It is given as `--template-loader FILE` (`-` for standard input,
never beside `--answers -`), and in Rust as `LoaderDocument`. Regenerating an item whose
recorded `template` is not a readable file **requires** one: the refusal says so and names
the recorded reference.

### `onetaskgraph.item_kind` is one plugin's, and only one plugin's

The first three keys above are obligations on **every** source. `onetaskgraph.item_kind`
is not one, and the difference is the point: it is spelled in the contract crate for the
reason the others are — a key under this product's prefix is the product's to name, so no
plugin can invent a colliding spelling — and it obliges nobody.

**`github-projects` is the one source that reads or writes it.** A GitHub Projects board
holds only issues. A project there *is* an issue and its tasks are that issue's
sub-issues, so an issue with sub-issues is a project and an issue without them is a task —
except for the one state a project copy necessarily passes through, between creating the
project and filing its first task, where an empty project is indistinguishable from a
task. The marker is what makes that state readable. It is **sufficient and never
necessary**: an unmarked issue with sub-issues is still a project, so a person can author
one on the board by hand with no knowledge of this metadata at all, and clearing a marker
never hides a project. A sub-issue is always a task, whatever it carries. The key accepts
exactly `"project"` and `"task"`; anything else is malformed.

**No other source has that problem, so none of them touches the key.** `local-md` has a
folder per kind and Linear has native projects, so each already knows which kind an item
is without being told. To them the key is ordinary caller metadata: they hand it back with
its value and JSON type intact, exactly as they hand back every other key they do not own.

### A `github-projects` document is told by its title, not by a key

A board has no document type either, and this one is **not** solved with a metadata key. A
document there is an ordinary issue whose title begins `DESIGN: ` — spelled once, as
`onetaskgraph_github_projects::DESIGN_TITLE_PREFIX`, and never guessed at elsewhere. An
issue whose title starts with it is a document; every other issue is the task or the
project the sub-issue rule above makes it.

Three consequences, settled here rather than discovered:

- **The reported title is the title a person wrote**, with the prefix taken off — the same
  way this source takes its metadata slot off the body so `content` is what the person
  wrote. Writing a document puts the prefix back on, so a document copied out and copied
  back returns the title it started with.
- **The prefix is read before the sub-issue rule.** A document is never a project and never
  a task, whatever sub-issues it has or does not have; reading the prefix later would make
  a design issue with no sub-issues an *empty project*, which is the one state the
  `onetaskgraph.item_kind` marker exists to make readable.
- **A document carries no `onetaskgraph.item_kind`.** That key names what a dependency
  endpoint points at, and nothing may point at a document, so writing one neither sets the
  marker nor keeps a marker an earlier write left behind. A caller's own keys travel in the
  body slot exactly as a task's do.

The prefix is a *title*, so it is visible to a person reading the board — which is the
point: a design document on a board a person reviews should read as one in GitHub's own
interface, not only through this product.

## Repositories

A repository is identified by its **normalized origin as one string**:
`github.com/nickderobertis/onetaskgraph` — no scheme, no `.git` suffix, at least
`host/owner/name`. That is the identity a person types and the identity other tools
resolve. A list names each origin once; a repeat is refused rather than silently
collapsed.

<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] The human-facing statement
     of a rule whose one executable source is `GitHubProjectsSource::creation_target` in
     `onetaskgraph-github-projects`; that crate's `tests/plugin.rs` drives every arm stated
     here against the loopback board and asserts on `createIssue`'s own `repositoryId`, so
     the rule cannot change without failing there, and the refusals are not repeated here. -->
A source with a native notion of it reads it from there. `github-projects` derives it
from an issue's own repository, and records the key **only** when the item's list is not
exactly that one repository. The list also decides **where** that source creates an issue,
under one rule: exactly one entry, and the issue is created in that repository; zero
entries or two or more, and a task's or a document's issue is created in the repository
its parent project's issue lives in, while a project's issue — or a task's or document's
written with no parent — is created in the source's configured `repository:`. So a task
naming the one repository its work changes is filed where a person looks for it, and the
read side and the write side agree by construction: one entry that is where the issue
lives is derived and never written down, none is recorded as `[]`, and several are
recorded as named. An existing issue is never moved; a list that no longer matches where
it lives is recorded in the key. Every source reads it from `onetaskgraph.repositories`
where it has no native slot, so it is reachable everywhere.
<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->

## Dependencies that leave the source

One rule decides where an edge lives, and every source follows it: **the backend's own
relationship wherever that relationship can name the far end, and the reserved key only
where it cannot.** A far end in another source is the reserved key's case everywhere,
because no backend relates an id in a system it knows nothing about.

The rule is enforced, not just described. A source **refuses** an `onetaskgraph.depends_on`
entry its own relationship could have held, naming the entry and saying to record it in the
backend instead — otherwise a plan would drift into a text field the backend cannot read
and a person cannot follow, one entry at a time. "Another source" below means another
source *by name*: an entry qualified with the reading source's own configured name is an
item of that source, and is refused exactly as the bare id would be. What each source will
and will not accept:

| near item | its native relationship holds | the key may hold |
| --- | --- | --- |
| `linear` issue | issues of this workspace | another source, or a Linear project |
| `linear` project | projects of this workspace | another source, or a Linear issue |
| `github-projects` task issue | task issues of this board | another source, or a project |
| `github-projects` project issue | project issues of this board | another source, or a task |
| `github-projects` draft item | nothing | anything |
| `github-projects` document issue | nothing | nothing |

A document is the one row that holds nothing at either end, and that is the contract
rather than a limit of this backend: a document is not work, so it takes part in no
dependency graph. A write naming a dependency for one is refused, and a board relationship
this source finds pointing at a design issue is refused by name too — reporting it as a
task or as a project would name an id no task or project read of that source can find.

An edge is always oriented `from` **depends on** `to`, whichever way the backend spells it.
GitHub's `blockedBy` and `blocking` are one relationship read from either end, so both
report the same edge with the waiting item as `from`, rather than two mirrored ones.

## Where each source keeps metadata

| source | reads metadata from |
| --- | --- |
| `local-md` | a `metadata:` mapping in the YAML front matter |
| `in-memory` | as given in its configuration |
| `github-projects` | canonical JSON in a trailing comment slot at the end of the issue body, for a project issue, a task issue and a document issue alike |
| `linear` | canonical JSON in a slot the source owns on the item itself |

### Linear's slot, settled

Linear gives a caller no field of their own, so the source owns a **trailing Markdown
comment at the end of the item's long form** — an issue's `description`, a project's or a
document's `content` — on one line, with the canonical JSON inside a code span:

```text
The description a person wrote.

<!-- onetaskgraph.metadata `{"onepipeline.turn_budget":12}` -->
```

Two properties decided the comment. Every other field the item carries — its title, its
content, its labels, its state — still round-trips unchanged beside the metadata, because the
slot is inside a field Linear already treats as free text. And a person opening that issue in
Linear's own interface still sees their issue rather than a payload, because Linear renders the
description as Markdown and a Markdown comment does not render.

**A project's long form is its `content`, and that is a breaking change from earlier
releases.** Linear's schema documents `Project.description` as "the short description of the
project" and `Project.content` as the project's Markdown body, and a project's description
plus a slot holding a plan's budget answers — about ten kilobytes of JSON under one key — is
a long form. Earlier releases kept a project's description and its slot in `description`; this
source no longer reads that field at all, so a project written by one of them reads with no
content and no metadata — no origin, no member keys, no recorded far ends — until it is copied
again or migrated by moving its `description` into its `content`. Issues and documents are
unchanged.

The code span is Linear's doing. Linear normalizes the text *inside* an HTML comment in a
description or a document as it normalizes prose — observed 2026-10-02: a domain-like key such
as `caller.live` comes back `[caller.live](<http://caller.live>)`, an array's `[` and `]` come
back escaped, `\"` loses its backslash, `_y_` becomes `*y*`, and a line opening `-->` comes
back `\-->` — so JSON written bare there does not survive. Inside a code span on one line every
byte comes back as written. `<`, `>` and backticks inside a string are written as `\u003c`,
`\u003e` and `\u0060`, which JSON reads as the same characters, so no value can end the span or
the comment. A comment's *body* is not normalized: an HTML comment there comes back byte for
byte in any shape. The source still reads the multi-line slot it wrote before,

```text
<!-- onetaskgraph.metadata
{"onepipeline.turn_budget":12}
-->
```

in either close Linear hands it back with, and the next write of that item writes the code span.

The read side takes the slot off the visible description, so `content` is what the person
wrote. Only a comment at the very **end** of the description is a slot; one in the middle
is visible content and is left alone.

A task's `delivers` and `delivered_by` live in that same slot, under `onetaskgraph.delivers`
and `onetaskgraph.delivered_by`, each a JSON list of qualified ids — the shape and the rules
`github-projects` keeps in its body slot. A narrow write — `metadata set`, the store's
`delivered_by`, a copy recording `onetaskgraph.copies`, a regenerated rendering — sends one
update of the item's long form — an issue's `description`, a project's or a document's
`content` — that differs from what Linear holds only inside the slot.

**So the slot is one slot in two spellings, and each source writes the one its host keeps byte
for byte.** Linear writes the code span and reads both. `github-projects` writes and reads the
multi-line spelling, because GitHub keeps an issue body as it was written and so needs no code
span. `scripts/check-metadata-slot-encoding.sh` holds every source that keeps a slot to the
one multi-line spelling, any source that writes the code span to the one code-span spelling,
and both to the same opening marker, so a third spelling cannot appear in silence.

**`github-projects` keeps its slot in the issue body for the same reason plus one of its own:
length.** A ProjectV2 custom field is only `TEXT`, `NUMBER`, `DATE`,
`SINGLE_SELECT`, `MULTI_SELECT` or `ITERATION`, a `TEXT` value is length-bounded, and a
board's `shortDescription` is capped at 300 characters — of which the metadata comment
spends about 110 before any content, so a project carrying an ordinary 278-character goal
statement could not be copied at all. Caller metadata is unbounded (`onepipeline.steps`
carries whole task prose), so it goes where the body is: in the issue. Short typed things
do not: status goes to the board's `Status` single-select and the issue's own open or
closed state, `onetaskgraph.origin` goes to a source-owned `onetaskgraph.origin` text
field, and dependencies go to `blockedBy` and to sub-issue links.

<!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] This metadata guide must explain the two fields status occupies; the github-projects loopback tests and shared live journey reconcile the mapping, mutation order, and read-back at the real interface. -->
For terminal status, those two GitHub fields move together: `done` selects the mapped
`Done` option and closes as completed; `cancelled` selects the mapped `Cancelled` option
and closes as not planned. An open-category write reopens a closed issue before selecting
its mapped option. A missing option refuses the write before either half changes. On read,
the close reason decides `done` versus `cancelled` for a closed issue regardless of its
displayed Status, while the Status option decides an open issue's category.

## Setting one key on its own

`onetaskgraph task|project|document metadata set <ID> <KEY> <VALUE>` sets one key of one
record's metadata and changes nothing else about the record: the key is added when the record
does not hold it and replaced when it does, every other key and every other field is left as
it was, and a key already holding the value is a write that changes nothing. Metadata is not
status, so no delivered task is re-evaluated after one.

- **`ID`** is qualified, `<source>:<native-id>`.
- **`KEY`** is `<namespace>.<name>`: two or more non-empty dot-separated segments whose first
  segment is not `onetaskgraph`. The whole namespace is refused rather than the six keys above,
  because every key this product reserves — now or later — lives under it and is the store's
  to keep in step. `MetadataKey` in the contract crate is the one spelling of that rule, and it
  is enforced wherever one is decoded, the plugin protocol included.
- **`VALUE`** is exactly one JSON value, parsed strictly as JSON and never as YAML, so `yes`
  and `2026-01-01` are refused rather than silently given a type.

An unqualified id, a key or a value outside those shapes is refused **before any source is
built or asked**. What the verb answers — the `MetadataSet` root of the schema bundle — is the
id, the key, the value **as the source reads the record back after the write**, which is not
always the value it was handed, and the record's location when the source reports one.

| source | a metadata set |
| --- | --- |
| `local-md` | edits the one entry of the front matter's `metadata:` block and no other byte, atomically, and refuses by name a block it cannot edit that narrowly |
| `github-projects` | one update of the issue body that changes only its trailing metadata slot, for a task, a project and a document issue alike; no title, label, status or field request, and nothing at all when the key already holds the value |
| `in-memory` | holds the value for the life of its process |
| `linear` | one update of the issue's `description`, or the project's or the document's `content`, that changes only its trailing metadata slot — every byte above the slot as it was; nothing at all when the key already holds the value |
| a stdio plugin | answers the three methods of `docs/plugin-protocol.md` §4.18 when its handshake declares `metadata_updates` (§3.7), and is refused, without being asked, when it does not: `the <kind> plugin cannot write a task's metadata on its own`, with `a project's` or `a document's` for the other two verbs |

A source with no write side is refused naming its plugin, a record the source does not hold is
refused naming the id, and a source declaring it has no documents is never asked for a document
one. These are the same refusals `task status set` makes.

## Reading and writing are different obligations

**On read, every source is faithful about what it holds.** It returns the metadata it can
represent with its value and its JSON type intact, including a source whose only native
slot is text and which therefore decodes the canonical JSON encoding it stored. A source
with no representation for caller-defined metadata at all returns it empty and never
fabricates one — a rule kept for a source somebody else writes, since no source this
repository ships is that case.

**On write, every writable source round-trips exactly.** Reading back a value a copy wrote
returns what was written, value and JSON type alike. A source with only a text slot stores
the canonical JSON encoding and decodes it on read. A destination that cannot carry a key
**refuses the write, naming the source and the keys it could not carry**, rather than
dropping them — and so does one handed a field it cannot represent, naming the field.

The write seam arrived with the **copy verb**: `TaskSource::writes` says whether a source
has a write side at all, and `write_task` and `write_project` are how a destination is
reached. Both are defaulted to refusing, so a source with nothing to write into needs no
edit. `local-md` and `in-memory` are writable today; each remote source's own write side
lands with its own node, and Linear's writes back to the slot described above.

Four keys never travel as metadata even though a source may store them that way.
`onetaskgraph.repositories`, `onetaskgraph.depends_on`, `onetaskgraph.delivers` and
`onetaskgraph.delivered_by` are the *encoding* a source without a native slot uses; the truth
is the typed `repositories`, `delivers` and `delivered_by` fields and the item's own edges,
and those are what a copy carries — `delivered_by` excepted, which a copy never takes from its
source and which the destination keeps as it holds it. Writing the encoding beside them would have a
destination hold one thing twice and disagree with itself the moment one changed.

A copy adds one reserved key of its own, `onetaskgraph.origin`, whose value is the
qualified id the item was copied from. It is what makes a second copy an update rather
than a duplicate, and it lives on the item inside the plugin that owns it — nothing
anywhere holds a mapping.

One copy does not write it: a copy **back**, which is a copy whose counterpart was found
because the item being copied already named it. There the destination is the original and
the item being copied came out of it, so the destination keeps the origin it holds — and
keeps holding none where it holds none. The original's provenance is its own, and
overwriting it with the id of its own copy would leave the next ordinary copy from the
store the item was authored in matching nothing and creating a second item beside it.

### `onetaskgraph.copies`: where a copied item landed

<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Held by a drift gate:
     `a_served_plugin_takes_the_copy_link_key_only_with_a_value_that_is_links` in
     `crates/onetaskgraph-core/tests/subprocess.rs` reads the example below out of this page and
     fails unless the one validator of this value, at the stdio boundary, accepts it — beside
     the shapes that validator refuses. -->
`onetaskgraph.origin` is on the copy and names where it came from; `onetaskgraph.copies` is
on the item that was copied and names where it went. Its value is a JSON object mapping a
destination **source name** to the **qualified id** of that item's counterpart there, with at
most one entry per destination:

```json
{"followups": "followups:I_kwDOAbc123", "notes": "notes:T-1"}
```
<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->

A copy that is not a dry run records or refreshes the entry for its destination on every item
whose counterpart it found by this link, by searching the destination, or by `--match-by`,
and on every item it created — once the whole copy has landed, through the item's own
source's narrow metadata write (the one `metadata set` uses), and undone with the rest of the
copy if the copy cannot finish. Every other destination's entry is left as it is, so long as
it is a link at all: an entry whose value is not a qualified id of the source it is filed
under is dropped when the entry is written, because no copy could follow it. A copy that found its
counterpart by the item's own `onetaskgraph.origin` records nothing, because that
correspondence is already written down, on the destination item. The next copy of the item
reads the entry first: when the item it names still records this item as its origin, it is
the counterpart, found by one read by id and no search — see the copy section of the README
for the order the rules are tried in and the `stale-link` refusal.

It lives in the `onetaskgraph.` namespace like the origin: `metadata set` refuses to write it,
and a copy never carries it onto a destination — a destination item keeps the entry it holds,
which says where *it* was copied to. A source that cannot hold it is copied from exactly as
before, and the copy reports the link `unrecorded` rather than failing.

<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Held by behaviour, row by
     row: `crates/onetaskgraph-e2e/tests/e2e/copy_link.rs` records the link in a folder's front
     matter and reports a one-line `metadata:` as not recorded, over both boundaries;
     `crates/onetaskgraph-core/tests/copy_link.rs` records it on an in-memory source;
     `the_copy_link_is_kept_in_the_body_slot_and_reads_back_on_every_kind` in the
     github-projects plugin tests holds the body slot; the Linear plugin's
     `a_metadata_key_is_set_by_rewriting_the_slot_alone_for_every_record` holds its slot, and
     `a_metadata_set_moves_only_the_slot_and_a_render_only_the_body_and_its_provenance_on_linear`
     in `crates/onetaskgraph-linear-e2e/tests/e2e/linear.rs` records one on a Linear item through the binary,
     by a copy out of Linear; and
     `a_plugin_whose_handshake_does_not_declare_metadata_updates_is_refused_without_being_asked`
     with `a_served_plugin_takes_the_copy_link_key_only_with_a_value_that_is_links` hold the
     stdio row. -->
| source | where it keeps `onetaskgraph.copies` |
| --- | --- |
| `local-md` | an entry of the front matter's `metadata:` block, one line of compact JSON; a record whose `metadata:` is written on one line, or that this source otherwise cannot edit one key of narrowly, cannot hold it |
| `in-memory` | beside its other metadata, for the life of its process |
| `github-projects` | the trailing metadata slot of the issue body, beside the caller's keys — the value is small |
| `linear` | the trailing metadata slot of the issue's `description`, or the project's or the document's `content`, beside the caller's keys |
| a stdio plugin | wherever it keeps metadata, when its handshake declares `metadata_updates` (`docs/plugin-protocol.md` §4.18); otherwise nowhere, and unrecorded |
<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->

### `onetaskgraph.members` and `onetaskgraph.member_of`: a plan that spans sources

A routed copy can land one plan in two sources: a **home** project, and at most one **member** project in each other source a
task of it routes to. The two keys are what tie them together, and the store is where they
live:

- `onetaskgraph.members`, on a home, is a JSON list of qualified project ids, each in a
  different source from the home and from the others: `["hellopatient:a1b2c3"]`.
- `onetaskgraph.member_of`, on a member, is the qualified id of its home: `"plans:42"`.

Both are written by a routed write alone — a copy, or a `task create` routed away from the
source its project is in: a member's `member_of` when the write creates it, and the home's
`members` once the member exists, through the home's own project write, and taken back with
the rest of the write if it cannot finish. `metadata set` refuses either, as it
refuses every key in the namespace, and a copy never carries either onto a destination: a
destination project keeps the ones it holds, which describe *its* plan. `task list --project
<home> --members` reads `members` off the home on every request, and nothing else is kept.

<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Held by behaviour, row by
     row: `crates/onetaskgraph-e2e/tests/e2e/routes.rs` writes and reads both keys back through a
     folder of Markdown, a GitHub board home with a Linear member, and a Linear home with a
     folder member; the in-memory row is the engine's own write path, driven by
     `crates/onetaskgraph-e2e/tests/e2e/no_persistence.rs`. -->
| source | where it keeps `onetaskgraph.members` and `onetaskgraph.member_of` |
| --- | --- |
| `local-md` | entries of the project file's front matter `metadata:` block |
| `in-memory` | beside its other metadata, for the life of its process |
| `github-projects` | the trailing metadata slot of the project issue's body, beside the caller's keys |
| `linear` | the trailing metadata slot of the project's `content`, beside the caller's keys |
| a stdio plugin | wherever it keeps a project's metadata, through its project write |
<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->
