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

Run `bun run --cwd sdks/typescript generate` after changing the binary contract. The gate's
`./scripts/nx.sh run sdk-typescript:generate-check` target builds the binary and then fails
naming any generated file that would change.
