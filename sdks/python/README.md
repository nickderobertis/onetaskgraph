# onetaskgraph-sdk

The Python SDK for [onetaskgraph](https://github.com/nickderobertis/onetaskgraph).

It drives the real `onetaskgraph` binary as a subprocess and parses the machine-readable
output against the schema bundle that binary emits (`onetaskgraph schema`). The engine is
reused rather than reimplemented, so the CLI, a script and an application cannot answer the
same question differently.

The client surface is generated from the schema bundle and lands with a later change.
This package currently carries only the version the generated surface will be pinned to.
