# Local Markdown source

<!-- llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] This page is the
     human-facing description of the input contract whose single executable source is
     `onetaskgraph-local-md/src/lib.rs`: `FrontMatter`, `Dependency`, `LabelInput`,
     `EdgeKind`, `default_statuses`, `MAX_PAGE_SIZE`, and the traversal implementation.
     Public integration tests exercise the documented defaults, mappings, paging,
     traversal, and confinement at the plugin boundary. Generating prose from those Rust
     types would make the documentation less useful without adding an independent source
     of truth; contract changes therefore update this page and their boundary tests in the
     same change. -->

Configure `plugin: local-md` with a `root` directory. The plugin reads Markdown from
`root/tasks/` and `root/projects/`, recursively. A file's native identifier is its path
relative to that directory without `.md`; for example `tasks/team/release.md` is task
`team/release`. This makes identifiers stable and permits human-organized subfolders.

Each file starts with YAML front matter and continues with ordinary Markdown:

```markdown
---
title: Ship the release
status: doing
labels: [release, urgent]
project: platform
metadata:
  onepipeline.turn_budget: 12
  caller.reviewers: [ada, grace]
repositories: [github.com/nickderobertis/onetaskgraph]
depends_on:
  - design
  - id: security/review
    kind: related
  - id: work:PLAT-9
    item: project
---
# Ship the release

Long-form task content goes here.
```

`title` is optional; it falls back to the first level-one heading, then the file name.
`status` defaults to `todo`. `labels`, `project`, `metadata`, `repositories`, and
`depends_on` are optional. `metadata` is an ordered mapping of JSON-compatible YAML
values; `repositories` is an ordered list of normalized origins. A simple
dependency is a blocking edge; the expanded form accepts `kind: blocks` or `related`.
Labels may also use `{id: label-id, name: release, color: red}` when an explicit stable
identifier or color is useful. `url` is optional.
Projects use the same fields except that `project` is ignored conceptually and should be
omitted.

A bare `depends_on` entry is this source's own item, colons and all, so an identifier
containing one is never mistaken for a source name. The expanded form is where an author
says otherwise: `item: task` or `item: project` names what the far end is — it defaults to
the kind of the near document — and an `id` of the form `<source>:<native>` names an item
of another source entirely. The engine reports that far end without fetching it; opening
it is a command of your own against that qualified id.

`metadata` keys beginning `onetaskgraph.` and `onepipeline.` are reserved; see
[`metadata.md`](./metadata.md).

Status names are preserved for display and mapped case-insensitively to normalized
categories. The default mapping is `backlog` → backlog, `todo` → todo, `in progress` and
`doing` → in-progress, `done` → done, and `cancelled`/`canceled` → cancelled. Other words
map to unknown. Replace the mapping with `status_mapping` in the source configuration:

```yaml
sources:
  notes:
    plugin: local-md
    config:
      root: /home/me/notes
      status_mapping: { next: todo, active: in-progress, shipped: done }
```

Traversal is lexicographic by canonical path. Cursors encode an offset in that traversal,
so the plugin holds no index or state between calls. The maximum page size is 200: large
enough for interactive folder use while bounding one scan response. The configured root
and every traversed path are canonicalized; a symlink or identifier escaping the root is
rejected as a configuration error and is never read.

## Being copied into

This source has a write side, so it is a destination `onetaskgraph task copy --to` and
`onetaskgraph project copy --to` can name. A copy writes one document per item, with the
front matter above and the item's body beneath it, and reads back exactly what it wrote —
every caller-defined `metadata` key with its value and its JSON type intact.

Three things it refuses rather than doing quietly. A `target` naming a document this
folder does not hold is refused instead of created, because the engine established that
id before asking. A **status** this folder's `status_mapping` would read back as a
different category is refused naming the field and the mapping to add, because writing it
would quietly change what the item says. And a create never takes a name a document
already answers to: it files the new item under the next free `<id>-2`, `<id>-3`, and so
on, so nothing is written over.

A created document is named after the id the item was read under at its source, with
every character a path gives meaning to replaced — `..` cannot be spelled and no separator
can reach outside the configured root. `url`, and the created and updated times, are the
destination's own and are never written.
