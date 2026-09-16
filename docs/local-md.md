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
`root/tasks/`, `root/projects/` and `root/documents/`, recursively.

A file's native
identifier is its path relative to *its own* directory without `.md`; for example
`tasks/team/release.md` is task `team/release` and `documents/design/engine.md` is document
`design/engine`. This makes identifiers stable and permits human-organized subfolders.

`root` may be relative, and where it is measured from is the layer that supplied it: a
configuration document's relative `root` is resolved against **the directory holding that
document**, while one from the environment layer or a flag resolves against the **process
working directory**. The whole rule, for a reader of either side, is
[Relative paths in a configuration document](../README.md#relative-paths-in-a-configuration-document).

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
delivers: [security/review, "tracker:ENG-42"]
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

`delivers` names the tasks this one delivers: finishing it finishes them. A bare entry is a
task of this folder — `security/review` above — and `<source>:<id>` is a task of any source,
as `tracker:ENG-42` is. `delivered_by` is the other half and is onetaskgraph's to keep:
whenever it writes a task's `delivers` — a copy into this folder, or `task status set` on the
task — each task that list names gains `delivered_by: ["<source>:<id>"]` naming it, a task it
stopped naming loses that entry, and nothing else in the delivered task's file changes. Both
keys belong to a task alone; an entry that is not a task id, that names its own task, or that
names one task twice is refused naming the task and the entry, and so is either key in a
project's file. The same write moves a delivered task's status along with its deliverers while
it reads `todo`, `queued` or `in-progress`, rewriting only its `status:` line as
`task status set` does.

Status names are preserved for display and mapped case-insensitively to normalized
categories. **Every normalized category's own canonical spelling is accepted and reads back
as that category** — the words onetaskgraph itself prints and `task status set` takes:
`draft`, `backlog`, `todo`, `queued`, `in-progress`, `done` and `cancelled`. The default
mapping is those seven words, each to the category it names, plus the display aliases
`in progress` and `doing` → in-progress and `canceled` → cancelled. `queued` is work claimed
by something that will do it and not yet started, where `todo` is work ready to be picked up
that nothing has claimed. Other words map to unknown, and this source writes the original
word into the Markdown so it round-trips by name. That differs from `github-projects`, which can only
write an existing board option or a closed state: its unknown category is disabled by
default, and mapping it to one option folds every unknown word into that option. Replace
the mapping with `status_mapping` in the source configuration:

```yaml
sources:
  notes:
    plugin: local-md
    config:
      root: /home/me/notes
      status_mapping: { next: todo, active: in-progress, shipped: done }
```

## Setting a status on its own

`onetaskgraph task status set <source>:<id> <category>` rewrites the one `status:` line of
the task's front matter and **no other byte of the file** — title, labels, metadata,
`depends_on`, `delivers`, `delivered_by`, body and comments are left exactly as they were,
and a Windows file keeps its line endings. The word written is one `status_mapping` reads
back as that category: the category's own spelling when the mapping has it, preferring the
spaced form where the mapping holds both, so the default mapping writes `queued` and
`in progress`; otherwise the first word the mapping sends there. A task
already in the category is left byte for byte, word and all. A category the mapping reaches
with no word is refused in the words a copy of that status is refused with — `this source
reads "queued" as unknown, not queued` — and a task with no `status:` line gains one as the
last line of its front matter.

## Setting one metadata key on its own

`onetaskgraph task|project|document metadata set <source>:<id> <key> <value>` edits exactly
one entry of the record's `metadata:` block and **no other byte of the file**. An entry
already holding the key is replaced where it is; a missing one is added as the last entry of
the block; a file with no `metadata:` block gains one as the last line of its front matter.
The entry written is one line, `"<key>": <compact JSON>`, at the block's own indent — two
spaces when the block is new — and a Windows file keeps its line endings. A key is matched by
what it decodes to, so `myapp.note:`, `'myapp.note':` and `"myapp.note":` all name the same
entry, and an entry is everything that belongs to it: a nested block mapping, a block scalar
with blank lines inside it, and an indentless `- item` sequence under its key.

Setting the value a key already holds writes nothing at all — the file's bytes, inode and
modification time are unchanged, and no staging file is created.

The edit is checked before anything is written: the edited front matter has to read back as
the same record with that one key set to that value. Front matter that cannot be edited that
narrowly is **refused naming the file and the reason**, and the file is left as it was — it
is never reformatted to make room. That covers a `metadata:` written on one line as a flow
mapping, a block indented with a tab or unevenly, an entry whose key this source cannot read,
a key held twice, and a value whose JSON would not read back as itself; the refusal's next
action is to write the block as one `key: value` entry to a line and set the key again.

The file is replaced atomically: the new text is written to a staging file beside the record,
named `.<file>.<pid>-<n>.onetaskgraph-staging`, and renamed over it, so a reader walking the
folder at the same moment reads the old file or the new one and never a part of either. The
staging name does not end in `.md`, and a walk of the folder skips it by name, so it is never
listed as a record.

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

## Comments

A task's comments are an **optional trailing section of the task's own file**, after its
body — readable by a person at full fidelity, and never JSON:

```markdown
---
title: Ship the release
status: todo
---
Long-form task content.

## Comments

<!-- onetaskgraph:comment id="20260913T151107Z-1" author="ada" created_at="2026-09-13T15:11:07Z" updated_at="2026-09-13T15:11:07Z" -->
### ada — 2026-09-13T15:11:07Z

The body, byte-for-byte: any Markdown, including its own `##` headings.

<!-- /onetaskgraph:comment -->
```

`onetaskgraph task comment add|list|edit|delete` read and write this section; so can you, by
hand. The rules, all of which the plugin enforces:

- **The section** is the heading line `## Comments` followed by one or more comment blocks
  separated by blank lines, and nothing else to the end of the file. A `## Comments` heading
  anywhere else, or one followed by anything that is not a comment block, is ordinary task
  content. A task with no comments has no section: deleting the last comment removes the
  heading too.
- **A block** opens with the marker line `<!-- onetaskgraph:comment ... -->`, carrying `id`,
  `author` when one was given, `created_at` and `updated_at`. Each value is double-quoted,
  with `&`, `"`, `<` and `>` written as `&amp;`, `&quot;`, `&lt;` and `&gt;`. A heading line
  for a person follows — `### <author> — <created_at>`, or `### comment — <created_at>` when
  there is no author — then a blank line, the body exactly as written, a blank line, and the
  closing line `<!-- /onetaskgraph:comment -->`. The marker line is what the plugin reads;
  the heading is regenerated whenever the section is written.
- **The body** is stored byte for byte, its own headings and a trailing newline included. A
  body with a line that is exactly the closing line is refused on `add` and `edit`, saying
  why, rather than escaped — an escaped body would not be the one you wrote.
- **A comment id** is minted as the comment's creation time in UTC as `YYYYMMDDTHHMMSSZ`,
  a dash, and the smallest positive integer not already an id in that file:
  `20260913T151107Z-1`, then `20260913T151107Z-2` for a second comment in the same second.
  Ids are never renumbered.
- **The section is not the task's content.** `task show` and every query report the content
  as the body above the section, so `search --in content` never matches a comment's text.
- **A copy never touches it.** Copying a task into this folder over an existing file keeps
  that file's section byte for byte, and no comment of the task being copied is written into
  it. Content that would itself read back as a section is refused rather than turned into
  comments nobody wrote.

`--author` is recorded here as given — this folder has no signed-in account to record
instead — and must be one line with no control characters.

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
