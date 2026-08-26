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

- `onetaskgraph.` belongs to this product. It defines exactly three keys, each spelled
  once so no source can invent its own: `onetaskgraph.repositories`
  (`Repository::METADATA_KEY`) and `onetaskgraph.depends_on`
  (`DependencyEdge::RECORDED_KEY`) in the contract crate, and `onetaskgraph.origin`
  (`GlobalId::ORIGIN_KEY`) in the engine — that last one carries a *qualified* id, which
  no plugin ever constructs or interprets.
- `onepipeline.` belongs to that consumer.

Every other key is the caller's. A source returns it exactly as it holds it — the same
value, of the same JSON type — and this product never interprets it.

## Repositories

A repository is identified by its **normalized origin as one string**:
`github.com/nickderobertis/onetaskgraph` — no scheme, no `.git` suffix, at least
`host/owner/name`. That is the identity a person types and the identity other tools
resolve. A list names each origin once; a repeat is refused rather than silently
collapsed.

A source with a native notion of it reads it from there. `github-projects` derives it
from an issue-backed item's own repository. Every source reads it from
`onetaskgraph.repositories` where it has no native slot, so it is reachable everywhere.

## Dependencies that leave the source

One rule decides where an edge lives, and every source follows it: **the backend's own
relationship wherever that relationship can name the far end, and the reserved key only
where it cannot.** `docs/plugin-protocol.md` §4.8 is the normative text. A far end in
another source is the reserved key's case everywhere, because no backend relates an id in
a system it knows nothing about.

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
| `github-projects` issue item | issues of this project | another source, or a board |
| `github-projects` draft item | nothing | anything |
| `github-projects` board | boards, aggregated from its issues | another source, or a task |

An edge is always oriented `from` **depends on** `to`, whichever way the backend spells it.
GitHub's `blockedBy` and `blocking` are one relationship read from either end, so both
report the same edge with the waiting item as `from`, rather than two mirrored ones.

## Where each source keeps metadata

| source | reads metadata from |
| --- | --- |
| `local-md` | a `metadata:` mapping in the YAML front matter |
| `in-memory` | as given in its configuration |
| `github-projects` | canonical JSON in a project text field named `onetaskgraph.metadata` for an item, and in the board's own description slot for the project |
| `linear` | canonical JSON in a slot the source owns on the item itself |

### Linear's slot, settled

Linear gives a caller no field of their own, so the source owns a **trailing Markdown
comment at the end of the item's `description`**:

```text
The description a person wrote.

<!-- onetaskgraph.metadata
{"onepipeline.turn_budget":12}
-->
```

Two properties decided it. Every other field the item carries — its title, its content,
its labels, its state — still round-trips unchanged beside the metadata, because the slot
is inside a field Linear already treats as free text. And a person opening that issue in
Linear's own interface still sees their issue rather than a payload, because Linear
renders the description as Markdown and a Markdown comment does not render.

The read side takes the slot off the visible description, so `content` is what the person
wrote. Only a comment at the very **end** of the description is a slot; one in the middle
is visible content and is left alone. GitHub Projects uses the same encoding for a board's
description, which has no custom fields of its own.

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

Two keys never travel as metadata even though a source may store them that way.
`onetaskgraph.repositories` and `onetaskgraph.depends_on` are the *encoding* a source
without a native slot uses; the truth is the typed `repositories` field and the item's own
edges, and those are what a copy carries. Writing the encoding beside them would have a
destination hold one thing twice and disagree with itself the moment one changed.

A copy adds one reserved key of its own, `onetaskgraph.origin`, whose value is the
qualified id the item was copied from. It is what makes a second copy an update rather
than a duplicate, and it lives on the item inside the plugin that owns it — nothing
anywhere holds a mapping.
