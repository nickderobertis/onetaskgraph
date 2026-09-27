# @onetaskgraph/sdk

The TypeScript SDK for [onetaskgraph](https://github.com/nickderobertis/onetaskgraph).

It drives the real `onetaskgraph` binary as a subprocess and parses the machine-readable
output against the schema bundle that binary emits (`onetaskgraph schema`). The engine is
reused rather than reimplemented, so the CLI, a script and an application cannot answer the
same question differently.

Create a client and call the typed method matching the CLI command:

```ts
import { OnetaskgraphClient } from "@onetaskgraph/sdk";

const client = new OnetaskgraphClient();
const response = await client.taskList({ labels: ["bug"] });
```

The executable is resolved in this order: the constructor's explicit `binaryPath`, the
`ONETASKGRAPH_BIN` environment variable, then the executable supplied by the packaged
`@onetaskgraph/cli` dependency. If that dependency cannot be resolved, the client uses
`onetaskgraph` from `PATH`.

Every invocation requests JSON. The SDK parses stdout and validates it against runtime
schemas generated from `onetaskgraph schema` before returning it. Query responses retain
their `plan` and typed `errors`; exit code 4 therefore resolves to the validated partial
response instead of discarding the successful sources.

Every invocation also passes `--no-interactive`, so no call ever waits on a prompt: a
template variable nothing answers takes its default, and a required one left unanswered is
refused with exit 2. `templateRender(file, { searchPath, answers, vars })` hands `answers`
to the binary on standard input, and `vars` outrank them as `--var NAME=VALUE`.

Run `bun run --cwd sdks/typescript generate` after changing the binary contract. The gate's
`./scripts/nx.sh run sdk-typescript:generate-check` target builds the binary and then fails
naming any generated file that would change.

`taskCreate(source, project, title, { template, answers })` and `documentCreate(...)` create
an item from a template file — or from `templateLoader` (the path of a loader document naming
what to render), or from a plain `body` or `bodyFile` — and answer as `taskShow` and
`documentShow` do; a rendered item records its `onetaskgraph.template` provenance, whose shape
is the `TemplateProvenance` type. `taskRender(id, ...)` and `documentRender(id, ...)`
regenerate one in place from its stored answers, with `answers`, `vars` and `unset` laid over
them; `taskAnswers(id)` and `documentAnswers(id)` answer with the stored `TemplateAnswers`.
`templateVariables` and `templateRender` take `{ templateLoader }` in place of a file. A
`body` and `answers` are never passed together — both would go to the binary's one standard
input — so the call is refused instead.
