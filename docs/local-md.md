# Local Markdown source

<!-- llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] This page is the
     human-facing description of the input contract whose single executable source is
     `onetaskgraph-local-md/src/lib.rs`: `FrontMatter`, `DocumentFrontMatter`, `Dependency`,
     `LabelInput`, `EdgeKind`, `Kind`, `default_statuses`, `MAX_PAGE_SIZE`, and the
     traversal implementation.
     Public integration tests exercise the documented defaults, mappings, paging,
     traversal, and confinement at the plugin boundary. Generating prose from those Rust
     types would make the documentation less useful without adding an independent source
     of truth; contract changes therefore update this page and their boundary tests in the
     same change. -->

Configure `plugin: local-md` with a `root` directory. The plugin reads Markdown from
`root/tasks/`, `root/projects/` and `root/documents/`, recursively. A file's native
identifier is its path relative to *its own* directory without `.md`; for example
`tasks/team/release.md` is task `team/release` and `documents/design/engine.md` is document
`design/engine`. This makes identifiers stable and permits human-organized subfolders.

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
`status` defaults to `backlog`, so a task that says nothing about its status is
work that has been written down but not yet queued. `labels`, `project`, `metadata`, `repositories`, and
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
the kind of the near item — and an `id` of the form `<source>:<native>` names an item
of another source entirely. The engine reports that far end without fetching it; opening
it is a command of your own against that qualified id.

`metadata` keys beginning `onetaskgraph.` and `onepipeline.` are reserved; see
[`metadata.md`](./metadata.md).

Status names are preserved for display and mapped case-insensitively to normalized
categories. The default mapping is `draft` → draft, `backlog` → backlog, `todo` → todo,
`in progress` and `doing` → in-progress, `done` → done, and `cancelled`/`canceled` →
cancelled. Other words map to unknown. Replace the mapping with `status_mapping` in the source configuration:

```yaml
sources:
  notes:
    plugin: local-md
    config:
      root: /home/me/notes
      status_mapping: { next: todo, active: in-progress, shipped: done }
```

## Documents

A document is one piece of information that lives in a project and is not work — a design,
a runbook, a note. It goes in `root/documents/`, read recursively on exactly the terms
above, and its front matter is a task's minus the two things a document does not have:

```markdown
---
title: Engine design
labels: [spec]
project: platform
url: https://example.invalid/design
metadata:
  caller.reviewers: [ada, grace]
repositories: [github.com/nickderobertis/onetaskgraph]
---
# Engine design

The long-form document goes here.
```

So: `title`, `labels`, `project`, `metadata`, `repositories` and `url`, all optional and
all read exactly as a task's are — and **no `status`** and **no `depends_on`**. Both are
refused naming the key rather than read and quietly ignored, because a document is not
work: it has no place in a status filter and none in a dependency graph. `document list`
accordingly has no status filter and `document` has no dependency verb.

### Why the folder is what tells a document from a task

The folder is already how this source tells a task from a project, so a document needs no
new mechanism: a source with a folder per kind knows which kind an item is without being
told. Two alternatives were considered and rejected.

A **metadata marker** — a reserved front-matter key naming the kind — belongs to a source
whose backend has one undifferentiated pile of items and no other way to say. That is one
plugin's problem and only that plugin's; adopting it here would add a key every file has to
carry to say what its own folder already says, and would let a file's folder and its marker
disagree.

A **distinct file extension** breaks this source's identifier rule. An identifier here is
the path with `.md` removed, so a second extension either makes that rule untrue or makes
`design.md` and `design.mdx` both claim the identifier `design`. The folder keeps one rule
for all three kinds.

## Where this source says an entity is

Every task, project and document this source reports carries a **location**: the
canonicalized absolute path of the file it was read from, as the contract's `{"path": …}`
shape. That is what a location is for on this backend — a reader holding one of these
entities can print the path or read the contents out for a person, without knowing anything
about this plugin. A path that would escape the configured root is refused rather than
reported, exactly as an identifier that would is.

The location does not replace, derive from or interact with the `url` front-matter key: a
file that names a URL goes on reporting it, and every consumer sees exactly what it saw.

Traversal is lexicographic by canonical path. Cursors encode an offset in that traversal,
so the plugin holds no index or state between calls. The maximum page size is 200: large
enough for interactive folder use while bounding one scan response. The configured root
and every traversed path are canonicalized; a symlink or identifier escaping the root is
rejected as a configuration error and is never read.

## Being copied into

This source has a write side, so it is a destination `onetaskgraph task copy --to`,
`onetaskgraph project copy --to` and `onetaskgraph document copy --to` can name. A copy
writes one file per item — a document under `documents/`, on the same terms as the other
two — with the front matter above and the item's body beneath it, and reads back exactly
what it wrote: every caller-defined `metadata` key with its value and its JSON type intact.
A document's front matter is written without a `status` and without a `depends_on`, so what
lands under `documents/` is a document rather than a task with fields left blank.

Three things it refuses rather than doing quietly. A `target` naming an item this folder
does not hold is refused instead of created, because the engine established that id before
asking. A **status** this folder's `status_mapping` would read back as a different category
is refused naming the field and the mapping to add, because writing it would quietly change
what the item says — a document reaches none of this, having no status to disagree about.
And a create never takes a name an item of that kind already answers to: it files the new
item under the next free `<id>-2`, `<id>-3`, and so on, so nothing is written over.

A created file is named after the id the item was read under at its source, with every
character a path gives meaning to replaced — `..` cannot be spelled and no separator can
reach outside the configured root. `url`, the **location**, and the created and updated
times are the destination's own and are never written: an item copied in reports the path
of the file this folder put it in, whatever its source said about where it was.
