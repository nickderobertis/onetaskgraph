# @onetaskgraph/sdk

The TypeScript SDK for [onetaskgraph](https://github.com/nickderobertis/onetaskgraph).

It drives the real `onetaskgraph` binary as a subprocess and parses the machine-readable
output against the schema bundle that binary emits (`onetaskgraph schema`). One
implementation of the query semantics — the engine — is what the SDK reuses rather than
reimplements; see the repository's `AGENTS.md` for the decision and for the stdio plugin
protocol named as the upgrade path.

The client surface is generated from the schema bundle and lands with a later change.
This package currently carries only the version the generated surface will be pinned to.
