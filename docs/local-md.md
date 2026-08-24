# Local Markdown source

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
depends_on:
  - design
  - id: security/review
    kind: related
---
# Ship the release

Long-form task content goes here.
```

`title` is optional; it falls back to the first level-one heading, then the file name.
`status` defaults to `todo`. `labels`, `project`, and `depends_on` are optional. A simple
dependency is a blocking edge; the expanded form accepts `kind: blocks` or `related`.
Labels may also use `{id: label-id, name: release, color: red}` when an explicit stable
identifier or color is useful. `url` is optional.
Projects use the same fields except that `project` is ignored conceptually and should be
omitted. Dependencies name native identifiers within the same source.

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
