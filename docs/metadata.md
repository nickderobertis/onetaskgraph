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

- `onetaskgraph.` belongs to this product. It defines exactly two keys, and both are
  spelled once in the contract crate so no source can invent its own:
  `onetaskgraph.repositories` (`Repository::METADATA_KEY`) and
  `onetaskgraph.depends_on` (`DependencyEdge::RECORDED_KEY`).
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

**Writing is not this product's yet.** No source here is writable: the write seam arrives
with the **copy verb**, and each remote source's own write side lands after that. Those
nodes carry the write obligations — a faithful round trip, and a destination refusing a
key it cannot carry rather than dropping it — and Linear's write side writes back to the
slot described above.
