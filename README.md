# onetaskgraph

One interface over the ticketing systems your work actually lives in.

Tasks, projects, labels and the dependencies between them are spread across Linear, GitHub
Projects and a folder of Markdown files, and every tool that wants to reach them ends up
reimplementing all three. `onetaskgraph` implements them once, behind a single query
surface, and exposes that surface three ways: a command-line tool, a Python SDK, and a
TypeScript SDK. All three drive the same engine, so they cannot drift apart.

Two properties make it different from a lowest-common-denominator wrapper:

- **A rich source is not reduced to a poor one's floor.** Each source declares what it can
  do natively. The engine pushes those predicates down and compensates in memory for the
  rest — and every response carries the plan it ran, so `--explain` shows you which source
  filtered server-side and which one the engine narrowed for.
- **Nothing of your work is kept.** No cache, no index, no local mirror. The engine holds
  at most one source page at a time and writes nothing down — enforced by a supply-chain
  gate that refuses every embedded store and cache crate, a sandboxed journey that fails
  if any file written during a run contains your data, and an assertion that the same
  query asked twice reaches the source twice.

> **Status.** The plugin contract, the workspace, the gate, the configuration layer and the
> query engine are in place, and the binary answers every verb below. The `in-memory`
> source is complete; `local-md`, `linear` and `github-projects` are registered and refuse
> with a clear message until their nodes land — and a configuration naming one of them
> costs you that source, not the query.

## Using it

```bash
onetaskgraph sources list

onetaskgraph task list [--source S]... [--label L]... [--not-label L]...
                       [--status S]... [--project P | --no-project]
                       [--search TEXT] [--in title|content|both]
                       [--limit N] [--page TOKEN] [--explain] [--allow-partial] [--json]
onetaskgraph task show <ID>
onetaskgraph task deps <ID> [--direction depends-on|depended-on-by]

onetaskgraph project list / show / deps          # the same flags, minus the project filter
onetaskgraph label list [--source S]...
onetaskgraph search <TEXT> [--in ...] [--kind task|project|both]
```

An `<ID>` is qualified: `work:ENG-142`. Repeating `--label` narrows — a second one is a
second requirement — and `--not-label` excludes. `--project` takes a qualified id, which
narrows the query to that project's own source, or a bare native id, which is asked of
every selected source.

### Seeing which plan you got

`--explain` renders the plan the query ran, per source:

```
$ onetaskgraph task list --label bug --explain
work:ENG-142   in-progress  Rate-limit the sync loop
notes:2026-08  todo         Write up the migration

plan:
  work (linear)  1 page(s)
    pushed down: label
  notes (local-md)  3 page(s)
    applied locally: label
```

Linear filtered server-side; the folder of Markdown could not, so the engine pulled pages
and narrowed them itself. Both answers are correct and you can see which you got.
`--json` carries the same plan as a field, so a script does not have to parse the prose.

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success — every source asked, every source answered. |
| `1` | The command failed while running: an id that names nothing, a configuration it will not run on, a source name nothing configures. |
| `2` | The invocation itself was wrong — an unknown flag, a value out of range. |
| `4` | The query ran and at least one source did not answer. The others' results still stand and the failure is named on standard error. |

`--allow-partial` says a partial answer is acceptable and turns `4` into `0`. Nothing else
does: a run that lost a source never exits `0` unless you asked for that.

### Paging

`--limit N` gives you a page and, when there is more, the token for the next one:

```bash
onetaskgraph task list --limit 20
# ... rows ...
# next page: --page 5b7b22736f7572...
```

Rows are interleaved across the selected sources — one from each in configured-name order,
then the next from each — and within a source they keep that source's own order. The token
is the engine's own; a source's cursor travels inside it untouched, and a walk returns
every row exactly once whatever page size you choose.

## Install

The command-line tool ships as a self-contained binary. Once a release is cut, install it
whichever way suits your machine:

```bash
cargo install onetaskgraph            # from crates.io
uv tool install onetaskgraph-cli      # from PyPI, no Rust toolchain needed
npm install -g onetaskgraph-cli       # from npm, no Rust toolchain needed
```

The two SDKs are separate packages, and both drive the installed binary rather than
reimplementing the engine:

```bash
uv add onetaskgraph-sdk               # Python
bun add @onetaskgraph/sdk             # TypeScript
```

To work on the repository instead, clone it and run `just bootstrap`; `just --list` shows
the rest.

## Configure

One YAML document, `onetaskgraph.yaml`, discovered upward from the working directory and
layered over a user-level file at `$XDG_CONFIG_HOME/onetaskgraph/config.yaml`:

```yaml
default_sources: [work, notes]   # omitted means every configured source
page_size: 50
output: text                     # text | json
sources:
  work:
    plugin: linear
    config: { api_key_env: LINEAR_API_KEY, team: ENG }
  notes:
    plugin: local-md
    config: { root: ~/notes/tasks }
```

Every setting is reachable at three layers, lowest precedence first: **the file, then the
environment, then a command-line flag.**

An environment variable is `ONETASKGRAPH_` followed by the config path, each segment
upper-cased with `-` replaced by `_` and segments joined by a double underscore; a list is
comma-separated:

| Variable | Sets |
| --- | --- |
| `ONETASKGRAPH_PAGE_SIZE=100` | top-level `page_size` |
| `ONETASKGRAPH_DEFAULT_SOURCES=work,notes` | top-level `default_sources` |
| `ONETASKGRAPH_SOURCES__WORK__CONFIG__ROOT=/tmp/tasks` | the `root` of the source named `work` |
| `ONETASKGRAPH_SOURCES__GH_MAIN__PLUGIN=github-projects` | the plugin of the source named `gh-main` |

The mapping is unambiguous because a source name may not contain an underscore.

On the command line the same dotted path is `--set`, and a few common settings have named
flags of their own:

```bash
onetaskgraph config show --set sources.work.config.root=/tmp/tasks
onetaskgraph config show --page-size 100 --default-sources work,notes --json
```

`onetaskgraph config show` is what makes precedence something you can see rather than
something you have to reason about: it prints every setting, its value, and the layer it
came from — which file, which environment variable, or which flag — and `--json` renders
the same thing for a script.

### Credentials

A configuration document never holds a credential — it names the environment variable that
does (`api_key_env: LINEAR_API_KEY`). Before resolving sources the CLI reads
`$XDG_CONFIG_HOME/onetaskgraph/secrets.env` (override with `ONETASKGRAPH_SECRETS_FILE`) as
`KEY=VALUE` lines with `#` comments, and applies each value **only where the process
environment does not already define that name** — so anything you exported wins. A missing
file is not an error. A credential is never printed, never in debug output, and never in a
log line.

There is exactly one name per credential everywhere — in that file, in the configuration,
in the documentation and in CI: `LINEAR_API_KEY` and `GH_PROJECTS_TOKEN`. Nothing anywhere
translates between spellings.

## Addressing

Every item is qualified by the source it came from and rendered `<source>:<native>` —
`work:ENG-142`, `notes:2026-08-inbox`. Parsing splits on the **first** colon, so a native
id may contain colons freely.

## Writing a source in another language

A source that is not a Rust crate speaks a line-oriented JSON protocol over stdio:
[`docs/plugin-protocol.md`](./docs/plugin-protocol.md) specifies it completely — the
framing, the capability handshake, one method per trait method, and the error envelope.

## Licence

MIT. See [`LICENSE`](./LICENSE).
